use crate::rfb::{self, Frame, Resolution};
use std::{
    io::{self, BufReader, Read, Write},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct View {
    pub layout: Option<(u32, u32)>,
    pub frame: Option<Frame>,
    pub error: Option<String>,
}

pub struct Session {
    child: Child,
    sender: SyncSender<Vec<u8>>,
    pub view: Arc<Mutex<View>>,
    pub token: String,
}

pub enum StartPage<'a> {
    Url(&'a str),
    Search(&'a str),
    Agent(&'a str, u16, bool),
}

pub fn ssh_command(
    host: &str,
    identity: Option<&std::path::Path>,
    remote_command: &str,
) -> io::Result<Command> {
    make_ssh_command(host, identity, remote_command, false)
}

pub fn monitor(host: &str, identity: Option<&std::path::Path>, disk: bool) -> io::Result<()> {
    let request = if disk { "host-ncdu" } else { "host-btop" };
    let command = make_ssh_command(
        host,
        identity,
        &format!("exec ~/.local/bin/rectangle-session {request}"),
        true,
    )?;
    #[cfg(windows)]
    let mut command = command;
    #[cfg(not(windows))]
    let mut command = {
        let mut terminal = Command::new("x-terminal-emulator");
        terminal
            .arg("-e")
            .arg(command.get_program())
            .args(command.get_args());
        terminal
    };
    let mut child = command.spawn()?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn make_ssh_command(
    host: &str,
    identity: Option<&std::path::Path>,
    remote_command: &str,
    terminal: bool,
) -> io::Result<Command> {
    if host.is_empty()
        || host.len() > 254
        || host.starts_with('-')
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@.-_".contains(&b))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Enter an SSH host alias or user@host",
        ));
    }
    #[cfg(windows)]
    let program = std::path::Path::new(
        &std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()),
    )
    .join("System32")
    .join("OpenSSH")
    .join("ssh.exe");
    #[cfg(not(windows))]
    let program = "ssh";
    let mut command = Command::new(program);
    if let Some(identity) = identity {
        command
            .args(["-o", "IdentitiesOnly=yes", "-i"])
            .arg(identity);
    }
    command.args([
        if terminal { "-tt" } else { "-T" },
        "-C",
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "ClearAllForwardings=yes",
        "-o",
        "ForwardAgent=no",
        "-o",
        "ForwardX11=no",
        "-o",
        "ConnectTimeout=8",
        "-o",
        "ServerAliveInterval=10",
        "-o",
        "ServerAliveCountMax=2",
        "--",
        host,
        remote_command,
    ]);
    if !terminal {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(if terminal { 0x0000_0010 } else { 0x0800_0000 });
    }
    Ok(command)
}

impl Session {
    #[cfg(test)]
    pub fn start(
        host: &str,
        identity: Option<&std::path::Path>,
        size: Resolution,
        repaint: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        Self::start_profile(
            host,
            identity,
            size,
            None,
            StartPage::Url(""),
            crate::settings::WebsiteTheme::Default,
            repaint,
        )
    }

    pub fn start_profile(
        host: &str,
        identity: Option<&std::path::Path>,
        size: Resolution,
        saved: Option<&str>,
        startup_page: StartPage<'_>,
        theme: crate::settings::WebsiteTheme,
        repaint: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        let mut id = [0; 16];
        getrandom::fill(&mut id).map_err(|e| io::Error::other(e.to_string()))?;
        let token: String = id.iter().map(|value| format!("{value:02x}")).collect();
        let mut remote = format!(
            "exec ~/.local/bin/rectangle-session {}x{} {token}",
            size.0, size.1
        );
        if theme != crate::settings::WebsiteTheme::Default {
            remote.push_str(" theme:");
            remote.push_str(theme.as_str());
        }
        if let Some(id) = saved {
            if !crate::saved::valid_id(id) {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            remote.push(' ');
            remote.push_str(id);
        }
        match startup_page {
            StartPage::Url("") => {}
            StartPage::Url(page) => {
                remote.push_str(&format!(" url:{}", crate::saved::encode(page)))
            }
            StartPage::Search(query) => {
                remote.push_str(&format!(" search:{}", crate::saved::encode(query)));
            }
            StartPage::Agent(image, port, file_manager) => {
                if image != "demo" && !crate::saved::valid_id(image) {
                    return Err(io::ErrorKind::InvalidInput.into());
                }
                remote.push_str(&format!(" agent:{image}:{port}"));
                if file_manager {
                    remote.push_str(":files");
                }
            }
        }
        Self::connect(host, identity, &remote, token, repaint)
    }

    pub fn resume(
        host: &str,
        identity: Option<&std::path::Path>,
        token: &str,
        repaint: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        if !crate::saved::valid_id(token) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Self::connect(
            host,
            identity,
            &format!("exec ~/.local/bin/rectangle-session resume {token}"),
            token.to_owned(),
            repaint,
        )
    }

    pub fn agent_view(
        host: &str,
        identity: Option<&std::path::Path>,
        token: &str,
        kind: crate::AgentView,
        repaint: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        let kind = kind.command();
        Self::connect(
            host,
            identity,
            &format!("exec ~/.local/bin/rectangle-session agent-view {token} {kind}"),
            token.to_owned(),
            repaint,
        )
    }

    fn connect(
        host: &str,
        identity: Option<&std::path::Path>,
        remote: &str,
        token: String,
        repaint: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        let mut command = ssh_command(host, identity, remote)?;
        let mut child = command.spawn()?;
        let mut input = BufReader::new(child.stdout.take().unwrap());
        let mut output = child.stdin.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let diagnostic = Arc::new(Mutex::new(Vec::<u8>::new()));
        let log = diagnostic.clone();
        thread::spawn(move || {
            let mut buf = [0; 512];
            while let Ok(n @ 1..) = stderr.read(&mut buf) {
                let mut bytes = log.lock().unwrap();
                let room = 1200usize.saturating_sub(bytes.len());
                bytes.extend_from_slice(&buf[..n.min(room)]);
            }
        });
        let (sender, receiver) = mpsc::sync_channel::<Vec<u8>>(128);
        let requests = sender.clone();
        let view = Arc::new(Mutex::new(View::default()));
        let display = view.clone();
        thread::spawn(move || {
            let result: io::Result<()> = (|| {
                let mut frame = rfb::connect(&mut input, &mut output)?;
                output.write_all(&rfb::request(&frame, false))?;
                let mut decoder = rfb::Decoder::default();
                thread::spawn(move || {
                    while let Ok(packet) = receiver.recv() {
                        if output.write_all(&packet).is_err() {
                            break;
                        }
                    }
                });
                loop {
                    let started = Instant::now();
                    let previous = (frame.width, frame.height);
                    rfb::update(&mut input, &mut frame, &mut decoder)?;
                    let changed = {
                        let mut view = display.lock().unwrap();
                        let layout_changed = view.layout != frame.layout;
                        view.layout = frame.layout;
                        if let Some(damage) = frame.dirty {
                            if let Some(pending) = &mut view.frame
                                && (pending.width, pending.height) == (frame.width, frame.height)
                            {
                                let stride = usize::from(frame.width) * 4;
                                for y in damage.y..damage.bottom {
                                    let row = (y * stride + damage.x * 4)
                                        ..(y * stride + damage.right * 4);
                                    pending.rgba[row.clone()].copy_from_slice(&frame.rgba[row]);
                                }
                                pending.layout = frame.layout;
                                pending.dirty =
                                    Some(pending.dirty.map_or(damage, |dirty| damage.union(dirty)));
                            } else {
                                view.frame = Some(frame.clone());
                            }
                            true
                        } else {
                            layout_changed
                        }
                    };
                    if changed {
                        repaint();
                    }
                    // Cap requests around 60 Hz without adding a fixed delay to a slow frame.
                    thread::sleep(Duration::from_millis(17).saturating_sub(started.elapsed()));
                    requests
                        .send(
                            rfb::request(&frame, previous == (frame.width, frame.height)).to_vec(),
                        )
                        .map_err(io::Error::other)?;
                }
            })();
            if let Err(error) = result {
                thread::sleep(Duration::from_millis(100));
                let log = diagnostic.lock().unwrap();
                let detail: String = String::from_utf8_lossy(&log)
                    .chars()
                    .filter(|c| !c.is_control() || *c == '\n')
                    .collect();
                let mut state = display.lock().unwrap();
                state.frame = None;
                state.error = Some(if detail.trim().is_empty() {
                    error.to_string()
                } else {
                    format!("{error}\n{}", detail.trim())
                });
                drop(state);
                repaint();
            }
        });
        Ok(Self {
            child,
            sender,
            view,
            token,
        })
    }

    pub fn send(&self, packet: impl Into<Vec<u8>>, droppable: bool) -> bool {
        match self.sender.try_send(packet.into()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => droppable,
            Err(TrySendError::Disconnected(_)) => false,
        }
    }

    pub fn resize(&self, size: Resolution) -> bool {
        let layout = self.view.lock().unwrap().layout;
        layout
            .and_then(|layout| Frame::resize_request(size, layout))
            .is_some_and(|packet| self.sender.try_send(packet).is_ok())
    }

    pub fn supports_resize(&self) -> bool {
        self.view.lock().unwrap().layout.is_some()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a live Rectangle host and an animated test page"]
    fn measures_live_tight_video_updates() {
        let host = std::env::var("RECTANGLE_SMOKE_HOST").unwrap();
        let identity = std::env::var_os("RECTANGLE_SMOKE_IDENTITY").map(std::path::PathBuf::from);
        let marker = std::env::var_os("RECTANGLE_STREAM_MARKER").unwrap();
        let session =
            Session::start(&host, identity.as_deref(), Resolution(1920, 1080), || {}).unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let view = session.view.lock().unwrap();
            assert!(view.error.is_none(), "{:?}", view.error);
            if view.frame.is_some() {
                break;
            }
            assert!(Instant::now() < deadline);
            drop(view);
            thread::sleep(Duration::from_millis(20));
        }
        let video = std::env::var_os("RECTANGLE_STREAM_RAW").is_none();
        assert!(session.send(rfb::encodings(video), false));
        std::fs::write(&marker, &session.token).unwrap();
        thread::sleep(Duration::from_secs(4));
        let start = Instant::now();
        let mut updates = 0;
        while start.elapsed() < Duration::from_secs(6) {
            let mut view = session.view.lock().unwrap();
            assert!(view.error.is_none(), "{:?}", view.error);
            updates += u32::from(view.frame.take().is_some());
            drop(view);
            thread::sleep(Duration::from_millis(2));
        }
        eprintln!(
            "{}: {:.1} display updates/s",
            if video { "Tight" } else { "Raw" },
            f64::from(updates) / start.elapsed().as_secs_f64()
        );
        assert!(updates >= 30, "animation was not streamed");
    }

    #[test]
    #[ignore = "requires a live host with the resizable browser image"]
    fn resizes_live_desktop_in_both_directions() {
        let host = std::env::var("RECTANGLE_SMOKE_HOST").unwrap();
        let key = std::env::var_os("RECTANGLE_SMOKE_IDENTITY").map(std::path::PathBuf::from);
        let session = Session::start(&host, key.as_deref(), Resolution::FALLBACK, || {}).unwrap();
        let wait_for = |size: Resolution| {
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            loop {
                {
                    let view = session.view.lock().unwrap();
                    assert!(view.error.is_none(), "{:?}", view.error);
                    if view.layout.is_some()
                        && view.frame.as_ref().is_some_and(|frame| {
                            (frame.width, frame.height) == (size.0, size.1)
                                && frame.rgba.chunks_exact(4).any(|px| px[3] != 0)
                        })
                    {
                        return;
                    }
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "no pixels at {size:?}"
                );
                thread::sleep(Duration::from_millis(40));
            }
        };
        wait_for(Resolution::FALLBACK);
        for size in [
            Resolution(526, 377),
            Resolution(1904, 1877),
            Resolution(3814, 1891),
        ] {
            assert!(session.resize(size));
            wait_for(size);
        }
    }

    #[test]
    #[ignore = "requires an installed remote Rectangle session and a trusted SSH key"]
    fn renders_and_sends_input_over_ssh() {
        let host = std::env::var("RECTANGLE_SMOKE_HOST").unwrap();
        let identity = std::env::var_os("RECTANGLE_SMOKE_IDENTITY").map(std::path::PathBuf::from);
        let requested = match std::env::var("RECTANGLE_SMOKE_4K").as_deref() {
            Ok("1") => Resolution(3840, 2160),
            _ => Resolution::FALLBACK,
        };
        let session = Session::start(&host, identity.as_deref(), requested, || {}).unwrap();
        let started = std::time::Instant::now();
        let deadline = started + Duration::from_secs(25);
        let mut before = None;
        while std::time::Instant::now() < deadline {
            let mut view = session.view.lock().unwrap();
            assert!(view.error.is_none(), "{:?}", view.error);
            if let Some(frame) = view.frame.take()
                && frame
                    .rgba
                    .chunks_exact(4)
                    .filter(|p| p[..3] == [108, 225, 186])
                    .take(25)
                    .count()
                    >= 25
            {
                before = Some(frame);
                break;
            }
            drop(view);
            thread::sleep(Duration::from_millis(50));
        }
        let before = before.expect("did not receive the Rectangle start page");
        assert_eq!(Resolution(before.width, before.height), requested);
        if requested == Resolution(3840, 2160) {
            let edge = (usize::from(before.width) * 180 + usize::from(before.width) - 64) * 4;
            assert_eq!(
                &before.rgba[edge..edge + 3],
                &[13, 18, 26],
                "Firefox is maximized across the guest desktop"
            );
        }
        eprintln!(
            "{}x{} start page rendered in {:.2}s",
            before.width,
            before.height,
            started.elapsed().as_secs_f32()
        );
        let stride = usize::from(before.width) * 4;
        let (left, top) = before
            .rgba
            .chunks_exact(stride)
            .enumerate()
            .find_map(|(y, row)| {
                let mut run = 0;
                for (x, pixel) in row.chunks_exact(4).enumerate() {
                    run = if pixel[..3] == [24, 35, 46] {
                        run + 1
                    } else {
                        0
                    };
                    if run == 100 {
                        return Some((x + 1 - run, y));
                    }
                }
                None
            })
            .expect("the start page's text field is visible");
        let x = u16::try_from(left + 90).unwrap();
        let y = u16::try_from(top + 18).unwrap();
        assert!(session.send(rfb::pointer(1, x, y), false));
        assert!(session.send(rfb::pointer(0, x, y), false));
        for key in b"rectangle" {
            assert!(session.send(rfb::key(u32::from(*key), true), false));
            assert!(session.send(rfb::key(u32::from(*key), false), false));
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            let mut view = session.view.lock().unwrap();
            assert!(view.error.is_none(), "{:?}", view.error);
            if view.frame.take().is_some_and(|frame| {
                (top + 10..top + 34)
                    .map(|y| {
                        let from = y * stride + (left + 10) * 4;
                        let to = from + 60 * 4;
                        frame.rgba[from..to]
                            .iter()
                            .zip(&before.rgba[from..to])
                            .filter(|(a, b)| a != b)
                            .count()
                    })
                    .sum::<usize>()
                    > 75
            }) {
                eprintln!("Text field repainted after remote input");
                return;
            }
            drop(view);
            thread::sleep(Duration::from_millis(50));
        }
        panic!("no screen change after input");
    }
}
