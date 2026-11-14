use std::{
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender},
    },
    thread,
};

#[derive(Clone, Default)]
pub struct State {
    pub url: String,
    pub back: bool,
    pub forward: bool,
}

pub struct Navigation {
    child: Child,
    commands: SyncSender<String>,
    state: Arc<Mutex<Option<State>>>,
}

impl Navigation {
    pub fn start(
        host: &str,
        key: Option<&Path>,
        token: &str,
        repaint: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        let mut child = crate::session::ssh_command(
            host,
            key,
            &format!("exec ~/.local/bin/rectangle-session navigation {token}"),
        )?
        .stderr(Stdio::null())
        .spawn()?;
        let mut output = child.stdin.take().unwrap();
        let mut input = BufReader::new(child.stdout.take().unwrap());
        let (commands, receiver) = mpsc::sync_channel::<String>(8);
        thread::spawn(move || {
            for command in receiver {
                if writeln!(output, "{command}").is_err() {
                    break;
                }
            }
        });
        let state = Arc::new(Mutex::new(None));
        let shared = Arc::clone(&state);
        thread::spawn(move || {
            let mut line = String::new();
            loop {
                line.clear();
                if !matches!((&mut input).take(8193).read_line(&mut line), Ok(1..=8192))
                    || !line.ends_with('\n')
                {
                    break;
                }
                if let Some(state) = parse(&line) {
                    *shared.lock().unwrap() = Some(state);
                    repaint();
                }
            }
            *shared.lock().unwrap() = None;
            repaint();
        });
        Ok(Self {
            child,
            commands,
            state,
        })
    }

    pub fn state(&self) -> Option<State> {
        self.state.lock().unwrap().clone()
    }

    pub fn send(&self, command: String) -> bool {
        self.commands.try_send(command).is_ok()
    }
}

impl Drop for Navigation {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn parse(line: &str) -> Option<State> {
    let (flags, url) = line.strip_suffix('\n')?.split_once(' ')?;
    let [back @ (b'0' | b'1'), forward @ (b'0' | b'1')] = flags.as_bytes() else {
        return None;
    };
    if !url.is_empty() {
        let parsed = url::Url::parse(url).ok()?;
        if !matches!(parsed.scheme(), "http" | "https") || url.chars().any(char::is_control) {
            return None;
        }
    }
    Some(State {
        url: url.into(),
        back: *back == b'1',
        forward: *forward == b'1',
    })
}
