use crate::session::ssh_command;
use std::{
    io::{self, BufReader, Read, Write},
    path::Path,
    process::{Child, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender},
    },
    thread,
};

pub const LIMIT: usize = 1024 * 1024;

#[derive(Default)]
pub struct Update {
    pub text: Option<String>,
    pub paste_ready: Option<bool>,
    pub disconnected: bool,
}

pub struct Clipboard {
    child: Child,
    send: SyncSender<String>,
    update: Arc<Mutex<Update>>,
}

impl Clipboard {
    pub fn start(
        host: &str,
        identity: Option<&Path>,
        token: &str,
        repaint: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        Self::start_view(host, identity, token, None, repaint)
    }

    pub fn start_view(
        host: &str,
        identity: Option<&Path>,
        token: &str,
        view: Option<&str>,
        repaint: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        let remote = if let Some(view) = view {
            format!("exec ~/.local/bin/rectangle-session agent-clipboard {token} {view}")
        } else {
            format!("exec ~/.local/bin/rectangle-session clipboard {token}")
        };
        let mut child = ssh_command(host, identity, &remote)?
            .stderr(Stdio::null())
            .spawn()?;
        let mut input = BufReader::new(child.stdout.take().unwrap());
        let mut output = child.stdin.take().unwrap();
        let (send, receive) = mpsc::sync_channel::<String>(1);
        thread::spawn(move || {
            while let Ok(text) = receive.recv() {
                if output
                    .write_all(&(text.len() as u32).to_be_bytes())
                    .and_then(|()| output.write_all(text.as_bytes()))
                    .is_err()
                {
                    break;
                }
            }
        });
        let update = Arc::new(Mutex::new(Update::default()));
        let shared = update.clone();
        thread::spawn(move || {
            let _ = (|| -> io::Result<()> {
                let mut header = [0; 8];
                input.read_exact(&mut header)?;
                if &header != b"RECTCLP1" {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                loop {
                    match read_event(&mut input)? {
                        ClipboardEvent::Text(text) => shared.lock().unwrap().text = Some(text),
                        ClipboardEvent::Paste(ready) => {
                            shared.lock().unwrap().paste_ready = Some(ready);
                        }
                    }
                    repaint();
                }
            })();
            shared.lock().unwrap().disconnected = true;
            repaint();
        });
        Ok(Self {
            child,
            send,
            update,
        })
    }

    pub fn paste(&self, text: String) -> bool {
        !text.is_empty()
            && text.len() <= LIMIT
            && !text.contains('\0')
            && self.send.try_send(text).is_ok()
    }

    pub fn poll(&self) -> Update {
        let mut shared = self.update.lock().unwrap();
        Update {
            text: shared.text.take(),
            paste_ready: shared.paste_ready.take(),
            disconnected: shared.disconnected,
        }
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

enum ClipboardEvent {
    Text(String),
    Paste(bool),
}

fn read_event(input: &mut impl Read) -> io::Result<ClipboardEvent> {
    let mut tag = [0];
    input.read_exact(&mut tag)?;
    match tag[0] {
        b'P' => Ok(ClipboardEvent::Paste(true)),
        b'E' => Ok(ClipboardEvent::Paste(false)),
        b'C' => {
            let mut length = [0; 4];
            input.read_exact(&mut length)?;
            let length = u32::from_be_bytes(length) as usize;
            if length == 0 || length > LIMIT {
                return Err(io::ErrorKind::InvalidData.into());
            }
            let mut text = vec![0; length];
            input.read_exact(&mut text)?;
            let text = String::from_utf8(text).map_err(|_| io::ErrorKind::InvalidData)?;
            if text.contains('\0') {
                return Err(io::ErrorKind::InvalidData.into());
            }
            Ok(ClipboardEvent::Text(text))
        }
        _ => Err(io::ErrorKind::InvalidData.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_unicode_and_rejects_oversized_messages() {
        let sample = "café\nこんにちは 🌍";
        let mut message = vec![b'C'];
        message.extend((sample.len() as u32).to_be_bytes());
        message.extend(sample.bytes());
        message.push(b'P');
        let mut input = &message[..];
        assert!(matches!(read_event(&mut input).unwrap(), ClipboardEvent::Text(s) if s == sample));
        assert!(matches!(
            read_event(&mut input).unwrap(),
            ClipboardEvent::Paste(true)
        ));
        assert!(read_event(&mut &[b'C', 0, 16, 0, 1][..]).is_err());
    }

    #[test]
    #[ignore = "requires a live Rectangle host and SSH key"]
    fn round_trips_unicode_through_firefox() {
        use crate::{Tab, rfb, session::Session};
        use std::time::{Duration, Instant};
        fn until(mut condition: impl FnMut() -> bool) {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !condition() {
                assert!(Instant::now() < deadline, "clipboard smoke test timed out");
                thread::sleep(Duration::from_millis(40));
            }
        }
        let host = std::env::var("RECTANGLE_SMOKE_HOST").unwrap();
        let identity = std::env::var_os("RECTANGLE_SMOKE_IDENTITY").map(std::path::PathBuf::from);
        let session =
            Session::start(&host, identity.as_deref(), rfb::Resolution::FALLBACK, || {}).unwrap();
        until(|| {
            let mut view = session.view.lock().unwrap();
            assert!(view.error.is_none(), "{:?}", view.error);
            view.frame.take().is_some_and(|f| {
                f.rgba
                    .chunks_exact(4)
                    .filter(|p| p[..3] == [108, 225, 186])
                    .take(25)
                    .count()
                    == 25
            })
        });
        let clip = Clipboard::start(&host, identity.as_deref(), &session.token, || {}).unwrap();
        let sample = "Rectangle café 日本語 🌍";
        assert!(clip.paste(sample.into()));
        until(|| {
            let update = clip.poll();
            assert!(!update.disconnected);
            assert_ne!(update.paste_ready, Some(false));
            assert!(update.text.is_none(), "a paste must not echo to Windows");
            update.paste_ready == Some(true)
        });
        assert!(session.send(rfb::pointer(1, 400, 490), false));
        assert!(session.send(rfb::pointer(0, 400, 490), false));
        let tab = Tab {
            session: Some(session),
            ..Default::default()
        };
        assert!(tab.shortcut(b'v'));
        thread::sleep(Duration::from_millis(500));
        assert!(tab.shortcut(b'a'));
        assert!(tab.shortcut(b'c'));
        until(|| {
            let update = clip.poll();
            assert!(!update.disconnected);
            update.text.is_some_and(|text| {
                assert_eq!(text, sample);
                true
            })
        });
    }
}
