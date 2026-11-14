use eframe::egui;
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

enum ResultState {
    Latest(Option<(String, String)>),
    Built(PathBuf),
}

#[derive(Default)]
pub struct Updater {
    job: Option<Receiver<io::Result<ResultState>>>,
    available: Option<(String, String)>,
    built: Option<PathBuf>,
    status: String,
    failed: bool,
}

fn version(value: &str) -> Option<[u32; 3]> {
    let mut parts = value.split('.');
    let mut numbers = [0; 3];
    for item in &mut numbers {
        let part = parts.next()?;
        *item = part.parse().ok()?;
        if item.to_string() != part {
            return None;
        }
    }
    parts.next().is_none().then_some(numbers)
}

fn check_result(output: &str) -> io::Result<ResultState> {
    let invalid = || io::Error::other("GitHub returned invalid update information.");
    let (sha, candidate) = output.trim().split_once(' ').ok_or_else(invalid)?;
    if sha.len() != 40
        || !sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid());
    }
    let newer = version(candidate).ok_or_else(invalid)? > version(VERSION).unwrap();
    Ok(ResultState::Latest(
        newer.then(|| (sha.into(), candidate.into())),
    ))
}

impl Updater {
    fn start(&mut self, ctx: &egui::Context, candidate: Option<(String, String)>) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        self.status = if candidate.is_some() {
            "Downloading and building the update…"
        } else {
            "Checking GitHub…"
        }
        .into();
        self.failed = false;
        self.job = Some(rx);
        thread::spawn(move || {
            let result = if let Some((sha, version)) = candidate {
                build(&sha, &version).map(ResultState::Built)
            } else {
                check()
            };
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    pub fn poll(&mut self) -> Option<PathBuf> {
        if let Some(job) = &self.job {
            let result = match job.try_recv() {
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some(Err(io::Error::other("Update stopped unexpectedly.")))
                }
                Ok(result) => Some(result),
            };
            if let Some(result) = result {
                self.job = None;
                match result {
                    Ok(ResultState::Latest(latest)) => {
                        self.status = latest.as_ref().map_or_else(
                            || "You're up to date.".into(),
                            |(_, version)| format!("Rectangle {version} is available."),
                        );
                        self.available = latest;
                    }
                    Ok(ResultState::Built(path)) => {
                        self.status = "Update built. Park tabs and restart to install it.".into();
                        self.built = Some(path.clone());
                        return Some(path);
                    }
                    Err(error) => {
                        self.failed = true;
                        self.status = error.to_string();
                    }
                }
            }
        }
        None
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        ui.heading(format!("Rectangle {VERSION}"));
        ui.label("Updates are built from the official GitHub repository. Git, Rust/Cargo, and the Windows C++ build tools must be installed.");
        if !self.status.is_empty() {
            ui.colored_label(
                if self.failed {
                    egui::Color32::LIGHT_RED
                } else {
                    crate::INK
                },
                &self.status,
            );
        }
        if self.job.is_some() {
            ui.spinner();
        } else if let Some(path) = &self.built {
            if ui.button("Install and restart").clicked() {
                return Some(path.clone());
            }
        } else {
            if ui
                .add_enabled(cfg!(windows), egui::Button::new("Check for updates"))
                .clicked()
            {
                self.available = None;
                self.start(ui.ctx(), None);
            }
            if let Some((sha, version)) = &self.available
                && ui.button(format!("Update to {version}")).clicked()
            {
                self.start(ui.ctx(), Some((sha.clone(), version.clone())));
            }
        }
        None
    }
}

#[cfg(windows)]
fn encoded_command(script: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<_> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut command = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        command.push(ALPHABET[(first >> 2) as usize] as char);
        command.push(ALPHABET[((first & 3) << 4 | second >> 4) as usize] as char);
        command.push(if chunk.len() > 1 {
            ALPHABET[((second & 15) << 2 | third >> 6) as usize] as char
        } else {
            '='
        });
        command.push(if chunk.len() == 3 {
            ALPHABET[(third & 63) as usize] as char
        } else {
            '='
        });
    }
    command
}

fn ps(mode: &str) -> io::Result<Command> {
    #[cfg(not(windows))]
    return Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Updates are available on Windows.",
    ));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let root = std::env::var_os("SystemRoot")
            .ok_or_else(|| io::Error::other("Windows directory not found."))?;
        let mut command = Command::new(
            PathBuf::from(root).join("System32/WindowsPowerShell/v1.0/powershell.exe"),
        );
        command
            .args(["-NoProfile", "-NonInteractive", "-EncodedCommand"])
            .arg(encoded_command(include_str!("update.ps1")))
            .env("RECTANGLE_UPDATE_MODE", mode)
            .stdin(Stdio::null())
            .creation_flags(0x0800_0000);
        Ok(command)
    }
}

fn check() -> io::Result<ResultState> {
    let result = ps("check")?.output()?;
    if !result.status.success() {
        let error = String::from_utf8_lossy(&result.stderr).trim().to_owned();
        return Err(io::Error::other(format!("Could not check GitHub: {error}")));
    }
    let output = String::from_utf8_lossy(&result.stdout);
    check_result(&output)
}

fn build(sha: &str, version: &str) -> io::Result<PathBuf> {
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).map_err(|error| io::Error::other(error.to_string()))?;
    let id: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    let parent = crate::settings::window_path()?.with_file_name("updates");
    fs::create_dir_all(&parent)?;
    let directory = parent.join(id);
    fs::create_dir(&directory)?;
    let log = directory.join("build.log");
    let output = fs::File::create(&log)?;
    let result = ps("build")?
        .env("RECTANGLE_UPDATE_DIR", &directory)
        .env("RECTANGLE_UPDATE_SHA", sha)
        .env("RECTANGLE_UPDATE_VERSION", version)
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::from(output))
        .status();
    for folder in ["source", "target"] {
        let _ = fs::remove_dir_all(directory.join(folder));
    }
    if !result?.success() {
        return Err(io::Error::other(format!(
            "Update build failed. See {}",
            log.display()
        )));
    }
    let candidate = directory.join("rectangle.exe");
    if !candidate.is_file() {
        return Err(io::Error::other(
            "The update did not produce a Windows executable.",
        ));
    }
    Ok(candidate)
}

pub fn install(candidate: &Path) -> io::Result<()> {
    let target = std::env::current_exe()?;
    let args = std::env::args_os()
        .skip(1)
        .map(|value| quote_argument(&value.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ");
    ps("install")?
        .env("RECTANGLE_UPDATE_TARGET", target)
        .env("RECTANGLE_UPDATE_CANDIDATE", candidate)
        .env("RECTANGLE_UPDATE_PID", std::process::id().to_string())
        .env("RECTANGLE_UPDATE_CWD", std::env::current_dir()?)
        .env("RECTANGLE_UPDATE_ARGS", args)
        .spawn()?;
    Ok(())
}

fn quote_argument(value: &str) -> String {
    let mut quoted = String::from("\"");
    let mut slashes = 0;
    for ch in value.chars() {
        if ch == '\\' {
            slashes += 1;
        } else {
            quoted.extend(std::iter::repeat_n(
                '\\',
                if ch == '"' { slashes * 2 + 1 } else { slashes },
            ));
            slashes = 0;
            quoted.push(ch);
        }
    }
    quoted.extend(std::iter::repeat_n('\\', slashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updater_only_accepts_newer_numbered_commits() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert!(matches!(
            check_result(&format!("{sha} 10.0.0\r\n")).unwrap(),
            ResultState::Latest(Some(_))
        ));
        assert!(matches!(
            check_result(&format!("{sha} {VERSION}")).unwrap(),
            ResultState::Latest(None)
        ));
        assert!(check_result(&format!("{sha} 10.00.0")).is_err());
        assert!(check_result("main 10.0.0").is_err());
    }
}
