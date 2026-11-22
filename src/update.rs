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
    Progress(String),
    Built(PathBuf),
    ServersCurrent,
    ServerFailed(Option<PathBuf>, String),
}

#[derive(Default)]
pub struct Updater {
    job: Option<Receiver<io::Result<ResultState>>>,
    available: Option<(String, String)>,
    built: Option<PathBuf>,
    prepared: Option<PathBuf>,
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
    let available = version(candidate).ok_or_else(invalid)? >= version(VERSION).unwrap();
    Ok(ResultState::Latest(
        available.then(|| (sha.into(), candidate.into())),
    ))
}

impl Updater {
    fn start(
        &mut self,
        ctx: &egui::Context,
        candidate: Option<(String, String)>,
        hosts: Vec<crate::Connection>,
        desktop: bool,
    ) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let prepared = if desktop { self.prepared.take() } else { None };
        self.status = if desktop && prepared.is_none() {
            "Downloading and building the update…"
        } else if candidate.is_some() {
            "Checking configured servers…"
        } else {
            "Checking GitHub…"
        }
        .into();
        self.failed = false;
        self.job = Some(rx);
        thread::spawn(move || {
            let result = (|| {
                let Some((sha, version)) = candidate else {
                    return check();
                };
                let binary = if desktop {
                    Some(match prepared {
                        Some(binary) => binary,
                        None => build(&sha, &version)?,
                    })
                } else {
                    None
                };
                for (index, connection) in hosts.iter().enumerate() {
                    let host = connection.host.trim();
                    let _ = tx.send(Ok(ResultState::Progress(format!("Checking and updating {host} ({}/{}). Image builds can take several minutes…", index + 1, hosts.len()))));
                    ctx.request_repaint();
                    if let Err(error) =
                        update_server(connection, &sha, &version, binary.as_deref(), index)
                    {
                        return Ok(ResultState::ServerFailed(
                            binary,
                            format!("{host}: {error}"),
                        ));
                    }
                }
                Ok(binary.map_or(ResultState::ServersCurrent, ResultState::Built))
            })();
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    pub fn poll(&mut self) -> Option<PathBuf> {
        while let Some(job) = &self.job {
            let result = match job.try_recv() {
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some(Err(io::Error::other("Update stopped unexpectedly.")))
                }
                Ok(result) => Some(result),
            };
            if let Some(Ok(ResultState::Progress(message))) = result {
                self.status = message;
            } else if let Some(result) = result {
                self.job = None;
                match result {
                    Ok(ResultState::Latest(latest)) => {
                        self.status = match latest.as_ref() {
                            Some((_, version)) if version != VERSION => {
                                format!("Rectangle {version} is available.")
                            }
                            _ => "The desktop client is up to date.".into(),
                        };
                        self.available = latest;
                    }
                    Ok(ResultState::Progress(_)) => unreachable!(),
                    Ok(ResultState::ServersCurrent) => {
                        self.status = "All configured servers are up to date.".into();
                    }
                    Ok(ResultState::ServerFailed(binary, error)) => {
                        self.prepared = binary;
                        self.failed = true;
                        self.status = error;
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
            } else {
                break;
            }
        }
        None
    }

    pub fn show(&mut self, ui: &mut egui::Ui, hosts: &[crate::Connection]) -> Option<PathBuf> {
        ui.heading(format!("Rectangle {VERSION}"));
        ui.label("Update builds the desktop client and updates configured servers that are behind. The desktop build needs Git, Rust/Cargo, and platform build tools (Xcode Command Line Tools on Mac; C++ Build Tools on Windows).");
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
                .add_enabled(
                    cfg!(any(windows, target_os = "macos")),
                    egui::Button::new("Check for updates"),
                )
                .clicked()
            {
                self.available = None;
                self.prepared = None;
                self.start(ui.ctx(), None, Vec::new(), false);
            }
            if let Some((sha, version)) = self.available.clone() {
                let desktop = version != VERSION;
                let label = if desktop && !hosts.is_empty() {
                    format!("Update desktop and servers to {version}")
                } else if desktop {
                    format!("Update desktop to {version}")
                } else {
                    "Sync servers".into()
                };
                if (desktop || !hosts.is_empty()) && ui.button(label).clicked() {
                    self.start(ui.ctx(), Some((sha, version)), hosts.to_vec(), desktop);
                }
            }
        }
        None
    }
}

fn update_server(
    connection: &crate::Connection,
    sha: &str,
    target: &str,
    binary: Option<&Path>,
    index: usize,
) -> io::Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    let parent = crate::settings::window_path()?.with_file_name("updates");
    fs::create_dir_all(&parent)?;
    let directory = binary.and_then(Path::parent).unwrap_or(&parent);
    let path = directory.join(format!("server-{}-{}.log", index + 1, target));
    let mut log = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&path)?;
    let command = format!("exec ~/.local/bin/rectangle-session server-update {sha} {target}");
    let output =
        crate::session::ssh_command(connection.host.trim(), connection.key.as_deref(), &command)?
            .stdin(Stdio::null())
            .stderr(Stdio::from(log.try_clone()?))
            .output()?;
    if output.status.success() {
        let output = String::from_utf8_lossy(&output.stdout);
        let mut fields = output.split_ascii_whitespace();
        if fields.next() == Some("RECTSERVERUPDATE1")
            && matches!(fields.next(), Some("updated" | "skipped"))
            && fields
                .next()
                .and_then(version)
                .is_some_and(|v| Some(v) >= version(target))
            && fields.next().is_none()
        {
            return Ok(());
        }
    }
    let size = log.metadata()?.len();
    log.seek(SeekFrom::Start(size.saturating_sub(4096)))?;
    let mut tail = Vec::new();
    log.read_to_end(&mut tail)?;
    let tail = String::from_utf8_lossy(&tail);
    let detail = if tail.contains("Invalid Rectangle display resolution.") {
        "This host needs one-time server updater setup. Run the 0.1.10 server installer with --keep-egress --enable-server-updates."
    } else {
        tail.lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("The host did not confirm the update.")
    };
    Err(io::Error::other(format!(
        "{detail} Log: {}",
        path.display()
    )))
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

fn platform_command(mode: &str) -> io::Result<Command> {
    #[cfg(all(not(windows), not(target_os = "macos")))]
    return Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Updates are available on Windows.",
    ));
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", include_str!("update-macos.sh"), "rectangle-update"])
            .env("RECTANGLE_UPDATE_MODE", mode)
            .stdin(Stdio::null());
        // Finder does not inherit the shell's Rust PATH.
        let home = std::env::var_os("HOME")
            .ok_or_else(|| io::Error::other("No home directory available"))?;
        let mut paths = vec![
            PathBuf::from(home).join(".cargo/bin"),
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        command.env(
            "PATH",
            std::env::join_paths(paths).map_err(io::Error::other)?,
        );
        Ok(command)
    }
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
    let result = platform_command("check")?.output()?;
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
    let result = platform_command("build")?
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
    let candidate = directory.join(if cfg!(windows) {
        "rectangle.exe"
    } else {
        "rectangle"
    });
    if !candidate.is_file() {
        return Err(io::Error::other(
            "The update did not produce a client executable.",
        ));
    }
    Ok(candidate)
}

#[cfg(not(target_os = "macos"))]
pub fn install(candidate: &Path) -> io::Result<()> {
    let target = std::env::current_exe()?;
    let args = std::env::args_os()
        .skip(1)
        .map(|value| quote_argument(&value.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ");
    platform_command("install")?
        .env("RECTANGLE_UPDATE_TARGET", target)
        .env("RECTANGLE_UPDATE_CANDIDATE", candidate)
        .env("RECTANGLE_UPDATE_PID", std::process::id().to_string())
        .env("RECTANGLE_UPDATE_CWD", std::env::current_dir()?)
        .env("RECTANGLE_UPDATE_ARGS", args)
        .spawn()?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
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
            ResultState::Latest(Some(_))
        ));
        assert!(check_result(&format!("{sha} 10.00.0")).is_err());
        assert!(check_result("main 10.0.0").is_err());
    }
}

#[cfg(target_os = "macos")]
pub fn install(candidate: &Path) -> io::Result<()> {
    let target = std::env::current_exe()?;
    // Fail before closing the UI if this installation cannot be replaced.
    let probe = target.with_extension(format!("update-probe-{}", std::process::id()));
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&probe)?;
    fs::remove_file(probe)?;
    let script = candidate.with_file_name("install.sh");
    fs::write(&script, include_str!("update-macos.sh"))?;
    Command::new("/usr/bin/nohup")
        .arg("/bin/sh")
        .arg(script)
        .env("RECTANGLE_UPDATE_MODE", "install")
        .env("RECTANGLE_UPDATE_TARGET", target)
        .env("RECTANGLE_UPDATE_CANDIDATE", candidate)
        .env("RECTANGLE_UPDATE_PID", std::process::id().to_string())
        .env("RECTANGLE_UPDATE_CWD", std::env::current_dir()?)
        .args(std::env::args_os().skip(1))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}
