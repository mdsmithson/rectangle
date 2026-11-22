use crate::{Connection, saved, session};
use eframe::egui;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
        mpsc::{self, Receiver},
    },
    thread,
};

const CHUNK: usize = 256 * 1024;

#[derive(Default)]
struct Progress {
    cancelled: AtomicBool,
    done: AtomicU64,
    total: AtomicU64,
    child: Mutex<Option<Child>>,
}

impl Progress {
    fn cancel(&self) {
        self.cancelled.store(true, Relaxed);
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }

    fn check(&self) -> io::Result<()> {
        if self.cancelled.load(Relaxed) {
            Err(io::Error::other("Transfer cancelled."))
        } else {
            Ok(())
        }
    }

    fn finish(&self) -> io::Result<()> {
        // Leave the child accessible to Cancel until it exits.
        loop {
            let status = {
                let mut child = self.child.lock().unwrap();
                let status = child
                    .as_mut()
                    .ok_or_else(|| io::Error::other("Missing SSH process."))?
                    .try_wait()?;
                if status.is_some() {
                    child.take();
                }
                status
            };
            if let Some(status) = status {
                self.check()?;
                return if status.success() {
                    Ok(())
                } else {
                    Err(io::Error::other(
                        "The Agent did not complete the operation.",
                    ))
                };
            }
            thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

pub(super) struct Job {
    title: &'static str,
    cancellable: bool,
    progress: Arc<Progress>,
    pub result: Receiver<io::Result<()>>,
}

impl Job {
    pub fn show(&self, ui: &mut egui::Ui, interactive: bool) {
        ui.horizontal(|ui| {
            let done = self.progress.done.load(Relaxed);
            let total = self.progress.total.load(Relaxed);
            if total == 0 {
                ui.spinner();
                ui.label(self.title);
            } else {
                let text = if self.title == "Deleting" {
                    format!("Deleting {done}/{total}")
                } else {
                    format!("{} · {} / {}", self.title, size(done), size(total))
                };
                ui.add(
                    egui::ProgressBar::new((done as f64 / total as f64).min(1.0) as f32)
                        .desired_width(ui.available_width().clamp(60.0, 320.0) * 0.76)
                        .text(text),
                );
            }
            if self.cancellable
                && ui
                    .add_enabled(
                        interactive && !self.progress.cancelled.load(Relaxed),
                        egui::Button::new("Cancel"),
                    )
                    .clicked()
            {
                self.progress.cancel();
            }
        });
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if self.cancellable {
            self.progress.cancel();
        }
    }
}

pub(super) enum Work {
    Create {
        directory: String,
        name: String,
        folder: bool,
    },
    Upload {
        directory: String,
        files: Vec<PathBuf>,
    },
    Download {
        directory: String,
        name: String,
        destination: PathBuf,
    },
    Delete {
        directory: String,
        name: String,
    },
    Rename {
        directory: String,
        name: String,
        to_directory: String,
        to_name: String,
    },
}

pub(super) fn start(ctx: &egui::Context, connection: &Connection, token: &str, work: Work) -> Job {
    let (sender, result) = mpsc::channel();
    let progress = Arc::new(Progress::default());
    let state = progress.clone();
    let connection = connection.clone();
    let token = token.to_owned();
    let ctx = ctx.clone();
    let (title, cancellable) = match &work {
        Work::Create { .. } => ("Creating", false),
        Work::Upload { .. } => ("Uploading", true),
        Work::Download { .. } => ("Downloading", true),
        Work::Delete { .. } => ("Deleting", false),
        Work::Rename { .. } => ("Moving", false),
    };
    thread::spawn(move || {
        let outcome = run(&connection, &token, &state, work);
        let _ = sender.send(outcome);
        ctx.request_repaint();
    });
    Job {
        title,
        cancellable,
        progress,
        result,
    }
}

fn run(connection: &Connection, token: &str, progress: &Progress, work: Work) -> io::Result<()> {
    let enc = saved::encode;
    match work {
        Work::Create {
            directory,
            name,
            folder,
        } => command(
            connection,
            progress,
            &format!(
                "agent-file {token} {} {} {}",
                if folder { "mkdir" } else { "new-file" },
                enc(&directory),
                enc(&name)
            ),
            |stdin, mut stdout| {
                drop(stdin);
                expect_ok(&mut stdout)
            },
        ),
        Work::Upload { directory, files } => {
            let mut sources = Vec::new();
            let mut errors = Vec::new();
            for path in files {
                let result = (|| {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .filter(|n| super::valid_name(n))
                        .ok_or_else(|| io::Error::other("Choose a file with a valid UTF-8 name."))?
                        .to_owned();
                    let meta = fs::metadata(&path)?;
                    if !meta.is_file() {
                        return Err(io::Error::other("Only regular files can be uploaded."));
                    }
                    Ok((name, meta.len()))
                })();
                match result {
                    Ok((name, bytes)) => sources.push((path, name, bytes)),
                    Err(error) => errors.push(format!("{}: {error}", path.display())),
                }
            }
            progress.total.store(
                sources
                    .iter()
                    .fold(0u64, |total, (_, _, bytes)| total.saturating_add(*bytes)),
                Relaxed,
            );
            for (path, name, bytes) in sources {
                progress.check()?;
                let base = progress.done.load(Relaxed);
                let result = File::open(path).and_then(|mut file| {
                    command(
                        connection,
                        progress,
                        &format!(
                            "agent-upload {token} {} {} {bytes}",
                            enc(&directory),
                            enc(&name)
                        ),
                        |mut stdin, mut stdout| {
                            copy(&mut file, &mut stdin, bytes, progress)?;
                            if file.read(&mut [0; 1])? != 0 {
                                stdin.write_all(&[0])?;
                            }
                            drop(stdin);
                            expect_ok(&mut stdout)
                        },
                    )
                });
                if let Err(error) = result {
                    progress.check()?;
                    errors.push(format!("{name}: {error}"));
                }
                progress.done.store(base.saturating_add(bytes), Relaxed);
            }
            if !errors.is_empty() {
                return Err(io::Error::other(errors.join("\n")));
            }
            Ok(())
        }
        Work::Download {
            directory,
            name,
            destination,
        } => {
            let mut random = [0u8; 16];
            getrandom::fill(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
            let random = u128::from_ne_bytes(random);
            let temp =
                destination.with_file_name(format!(".rectangle-download-{random:032x}.part"));
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            let result = command(
                connection,
                progress,
                &format!(
                    "agent-file {token} download {} {}",
                    enc(&directory),
                    enc(&name)
                ),
                |stdin, mut stdout| {
                    drop(stdin);
                    let bytes = header(&mut stdout, "RECTDOWNLOAD1 ")?;
                    progress.total.store(bytes, Relaxed);
                    copy(&mut stdout, &mut output, bytes, progress)?;
                    if stdout.read(&mut [0; 1])? != 0 {
                        return Err(io::Error::other("Invalid download length."));
                    }
                    output.sync_all()
                },
            );
            drop(output);
            let result = result
                .and_then(|_| progress.check())
                .and_then(|_| fs::rename(&temp, &destination));
            if result.is_err() {
                let _ = fs::remove_file(&temp);
            }
            result
        }
        Work::Delete { directory, name } => command(
            connection,
            progress,
            &format!(
                "agent-file {token} delete {} {}",
                enc(&directory),
                enc(&name)
            ),
            |stdin, mut stdout| {
                drop(stdin);
                progress
                    .total
                    .store(header(&mut stdout, "RECTDELETE1 ")?, Relaxed);
                loop {
                    let line = line(&mut stdout)?;
                    if line == "OK" {
                        progress.done.store(progress.total.load(Relaxed), Relaxed);
                        break Ok(());
                    }
                    let done = line
                        .strip_prefix("PROGRESS ")
                        .and_then(|count| count.parse().ok())
                        .ok_or_else(|| io::Error::other("Deletion was not confirmed."))?;
                    progress.done.store(done, Relaxed);
                }
            },
        ),
        Work::Rename {
            directory,
            name,
            to_directory,
            to_name,
        } => command(
            connection,
            progress,
            &format!(
                "agent-file {token} rename {} {} {} {}",
                enc(&directory),
                enc(&name),
                enc(&to_directory),
                enc(&to_name)
            ),
            |stdin, mut stdout| {
                drop(stdin);
                expect_ok(&mut stdout)
            },
        ),
    }
}

fn command(
    connection: &Connection,
    progress: &Progress,
    request: &str,
    work: impl FnOnce(std::process::ChildStdin, BufReader<std::process::ChildStdout>) -> io::Result<()>,
) -> io::Result<()> {
    progress.check()?;
    let mut child = session::ssh_command(
        connection.host.trim(),
        connection.key.as_deref(),
        &format!("exec ~/.local/bin/rectangle-session {request}"),
    )?
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()?;
    let (stdin, stdout, mut stderr) = (
        child.stdin.take().unwrap(),
        child.stdout.take().unwrap(),
        child.stderr.take().unwrap(),
    );
    let diagnostics = thread::spawn(move || {
        let mut detail = Vec::new();
        let _ = (&mut stderr).take(4096).read_to_end(&mut detail);
        let _ = io::copy(&mut stderr, &mut io::sink());
        String::from_utf8_lossy(&detail).trim().to_owned()
    });
    *progress.child.lock().unwrap() = Some(child);
    if progress.cancelled.load(Relaxed) {
        progress.cancel();
    }
    let result = work(stdin, BufReader::new(stdout));
    let exit = progress.finish();
    let detail = diagnostics.join().unwrap_or_default();
    progress.check()?;
    if exit.is_err() && !detail.is_empty() {
        return Err(io::Error::other(detail));
    }
    result.and(exit)
}

fn copy(
    input: &mut impl Read,
    output: &mut impl Write,
    mut left: u64,
    progress: &Progress,
) -> io::Result<()> {
    let mut buffer = vec![0; CHUNK];
    while left > 0 {
        progress.check()?;
        let count = input.read(&mut buffer[..left.min(CHUNK as u64) as usize])?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "File transfer ended early.",
            ));
        }
        output.write_all(&buffer[..count])?;
        left -= count as u64;
        progress.done.fetch_add(count as u64, Relaxed);
    }
    Ok(())
}

fn line(input: &mut impl BufRead) -> io::Result<String> {
    let mut value = String::new();
    input.take(256).read_line(&mut value)?;
    if !value.ends_with('\n') {
        return Err(io::Error::other(
            "Incomplete response from the Agent file service.",
        ));
    }
    Ok(value.trim_end().to_owned())
}
fn header(input: &mut impl BufRead, prefix: &str) -> io::Result<u64> {
    line(input)?
        .strip_prefix(prefix)
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| {
            io::Error::other(
                "Unexpected response from the Agent file service. Update the server launcher.",
            )
        })
}
fn expect_ok(input: &mut impl BufRead) -> io::Result<()> {
    if line(input)? == "OK" {
        Ok(())
    } else {
        Err(io::Error::other("The Agent did not confirm the operation."))
    }
}

pub(super) fn size(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        format!("{:.1} GiB", bytes as f64 / (1_u64 << 30) as f64)
    } else if bytes >= 1 << 20 {
        format!("{:.1} MiB", bytes as f64 / (1_u64 << 20) as f64)
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(windows)]
pub(super) fn save_as(name: &str) -> io::Result<Option<PathBuf>> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::UI::Controls::Dialogs::*;
    let mut path = vec![0u16; 32768];
    let name: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    for (slot, value) in path.iter_mut().zip(name.encode_utf16()) {
        *slot = value;
    }
    let title: Vec<_> = "Save Agent file as\0".encode_utf16().collect();
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFile: path.as_mut_ptr(),
        nMaxFile: path.len() as u32,
        lpstrTitle: title.as_ptr(),
        Flags: OFN_EXPLORER | OFN_NOCHANGEDIR | OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST,
        ..Default::default()
    };
    if unsafe { GetSaveFileNameW(&mut dialog) } == 0 {
        let error = unsafe { CommDlgExtendedError() };
        return if error == 0 {
            Ok(None)
        } else {
            Err(io::Error::other(format!(
                "Could not open Save As (code {error})."
            )))
        };
    }
    let end = path.iter().position(|n| *n == 0).unwrap_or(path.len());
    Ok(Some(PathBuf::from(std::ffi::OsString::from_wide(
        &path[..end],
    ))))
}

#[cfg(all(not(windows), not(target_os = "macos")))]
pub(super) fn save_as(_: &str) -> io::Result<Option<PathBuf>> {
    Err(io::Error::other(
        "Save As is available in the Windows client.",
    ))
}

#[cfg(target_os = "macos")]
pub(super) fn save_as(name: &str) -> io::Result<Option<PathBuf>> {
    let name = name.replace(['/', ':', '\0'], "_");
    let result = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", "on run argv", "-e", "try", "-e", "return POSIX path of (choose file name with prompt \"Save Agent file as\" default name (item 1 of argv))", "-e", "on error number -128", "-e", "return \"\"", "-e", "end try", "-e", "end run", "--"])
        .arg(name).output()?;
    if !result.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&result.stderr).into_owned(),
        ));
    }
    let path = String::from_utf8(result.stdout).map_err(io::Error::other)?;
    let path = path.strip_suffix('\n').unwrap_or(&path);
    Ok((!path.is_empty()).then(|| PathBuf::from(path)))
}
