use std::{
    io::{self, BufRead, BufReader, Read},
    path::Path,
    process::{Child, Stdio},
    sync::{Arc, Mutex},
    thread,
};

pub struct WorkingDirectory {
    child: Child,
    path: Arc<Mutex<Option<String>>>,
}

impl WorkingDirectory {
    pub fn start(
        host: &str,
        key: Option<&Path>,
        token: &str,
        view: &str,
        repaint: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        let mut child = crate::session::ssh_command(
            host,
            key,
            &format!("exec ~/.local/bin/rectangle-session agent-cwd {token} {view}"),
        )?
        .stderr(Stdio::null())
        .spawn()?;
        let mut input = BufReader::new(child.stdout.take().unwrap());
        let path = Arc::new(Mutex::new(None));
        let shared = path.clone();
        thread::spawn(move || {
            let mut line = String::new();
            loop {
                line.clear();
                if !matches!((&mut input).take(8193).read_line(&mut line), Ok(1..=8193))
                    || !line.ends_with('\n')
                {
                    break;
                }
                *shared.lock().unwrap() = crate::saved::decode(line.trim_end_matches('\n'), 4096)
                    .filter(|path| path.starts_with('/'));
                repaint();
            }
            *shared.lock().unwrap() = None;
            repaint();
        });
        Ok(Self { child, path })
    }

    pub fn address(&self, host: &str, token: &str, view: &str) -> Option<String> {
        let path = self.path.lock().unwrap();
        Some(address(host, token, view, path.as_deref()?))
    }
}

impl Drop for WorkingDirectory {
    fn drop(&mut self) {
        // Close the SSH channel's input, which also stops the container watcher.
        drop(self.child.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn address(host: &str, token: &str, view: &str, path: &str) -> String {
    let mut url = url::Url::parse("rectangle://terminal/").unwrap();
    url.path_segments_mut()
        .unwrap()
        .clear()
        .push(host)
        .push(token)
        .push(view)
        .extend(path[1..].split('/'));
    url.into()
}

#[test]
fn terminal_address_identifies_the_view_and_escapes_the_folder() {
    assert_eq!(
        address("user@host", "token", "sh", "/agent/home/A #%"),
        "rectangle://terminal/user@host/token/sh/agent/home/A%20%23%25"
    );
    assert_eq!(
        address("user@host", "token", "main", "/"),
        "rectangle://terminal/user@host/token/main/"
    );
}
