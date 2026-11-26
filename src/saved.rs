use crate::session::ssh_command;
use std::{
    io::{self, Read, Write},
    path::Path,
    sync::mpsc::{self, Receiver},
    thread,
};

#[derive(Clone)]
pub struct Entry {
    pub codex: bool,
    pub application: bool,
    pub id: String,
    pub title: String,
    pub startup_page: String,
    pub agent_port: Option<u16>,
    pub file_manager: bool,
}

#[derive(Clone)]
pub struct Parked {
    pub codex: bool,
    pub application: bool,
    pub agent_port: Option<u16>,
    pub file_manager: bool,
    pub always_on: bool,
    pub snap: Option<crate::tiling::Snap>,
    pub token: String,
    pub title: String,
    pub profile: Option<String>,
    pub expires: u64,
}

pub fn parse_parked(output: &str) -> io::Result<Vec<Parked>> {
    let mut lines = output.lines();
    if lines.next() != Some("RECTPARK1") {
        return Err(io::Error::other("The host did not return parked sessions"));
    }
    Ok(lines
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            let [token, name, profile, expires, layout @ ..] = fields.as_slice() else {
                return None;
            };
            if !valid_id(token) || !valid_id(profile) {
                return None;
            }
            let name = decode(name, 160)?;
            title(&name)?;
            Some(Parked {
                codex: layout.iter().skip(2).any(|item| *item == "codex"),
                application: layout.iter().skip(2).any(|item| *item == "application"),
                agent_port: layout.get(1).and_then(|port| port.parse().ok()),
                always_on: layout.iter().skip(2).any(|item| *item == "always-on"),
                file_manager: layout.iter().skip(2).any(|item| *item == "files"),
                snap: layout
                    .first()
                    .and_then(|slot| crate::tiling::Snap::parse(slot)),
                token: (*token).into(),
                title: name,
                profile: (*profile != *token).then(|| (*profile).into()),
                expires: expires.parse().ok()?,
            })
        })
        .collect())
}

#[cfg(test)]
#[test]
fn parked_list_preserves_always_on_and_reads_legacy_records() {
    let id = "1234567890abcdef1234567890abcdef";
    let entries = parse_parked(&format!(
        "RECTPARK1\n{id} 4167656e74 {id} 0 - 8080 always-on\n{id} 4167656e74 {id} 0 - 8080\n{id} 42726f77736572 {id} 0 left - always-on\n"
    ))
    .unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].agent_port, Some(8080));
    assert!(entries[0].always_on);
    assert!(!entries[1].always_on);
    assert!(entries[2].always_on);
    assert_eq!(entries[2].agent_port, None);
    assert_eq!(entries[2].snap, Some(crate::tiling::Snap::Left));
}

#[cfg(test)]
#[test]
fn codex_sessions_keep_their_app_when_saved_and_parked() {
    let id = "1234567890abcdef1234567890abcdef";
    let saved = parse(&format!(
        "RECTSAVED2\n{id} 436f646578 agent:codex:4007:files\n"
    ))
    .unwrap();
    assert!(saved[0].codex && saved[0].file_manager);
    assert_eq!(saved[0].agent_port, Some(4007));
    let parked = parse_parked(&format!(
        "RECTPARK1\n{id} 436f646578 {id} 0 - 4007 codex always-on files\n"
    ))
    .unwrap();
    assert!(parked[0].codex && parked[0].always_on && parked[0].file_manager);
}

#[test]
fn application_sessions_keep_the_desktop_when_saved_and_parked() {
    let id = "1234567890abcdef1234567890abcdef";
    let saved = parse(&format!(
        "RECTSAVED2\n{id} 47616d6573 agent:application:4010\n"
    ))
    .unwrap();
    assert!(saved[0].application && !saved[0].codex && !saved[0].file_manager);
    assert_eq!(saved[0].agent_port, Some(4010));

    let parked = parse_parked(&format!(
        "RECTPARK1\n{id} 47616d6573 {id} 0 - 4010 application always-on\n"
    ))
    .unwrap();
    assert!(parked[0].application && parked[0].always_on);
    assert_eq!(parked[0].agent_port, Some(4010));
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn title(input: &str) -> Option<&str> {
    let text = input.trim();
    (!text.is_empty() && text.len() <= 80 && !text.chars().any(char::is_control)).then_some(text)
}

pub fn encode(title: &str) -> String {
    title.bytes().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn decode(encoded: &str, max_len: usize) -> Option<String> {
    if encoded.len() > max_len || encoded.len() % 2 != 0 || !encoded.is_ascii() {
        return None;
    }
    let bytes: Vec<u8> = (0..encoded.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&encoded[i..i + 2], 16).ok())
        .collect::<Option<_>>()?;
    String::from_utf8(bytes).ok()
}

pub fn parse(output: &str) -> io::Result<Vec<Entry>> {
    let mut lines = output.lines();
    if lines.next() != Some("RECTSAVED2") {
        return Err(io::Error::other(
            "This server does not support saved sessions",
        ));
    }
    let mut list = Vec::new();
    for line in lines.take(24) {
        let item = (|| {
            let (id, data) = line.split_once(' ')?;
            if !valid_id(id) {
                return None;
            }
            let (name, page) = data.split_once(' ').unwrap_or((data, ""));
            let name = decode(name, 160)?;
            title(&name)?;
            let file_manager = page.starts_with("agent:") && page.ends_with(":files");
            let agent_port = if let Some(agent) = page.strip_prefix("agent:") {
                let agent = agent.strip_suffix(":files").unwrap_or(agent);
                let (image, port) = agent.split_once(':')?;
                if !matches!(image, "demo" | "codex" | "application") && !valid_id(image) {
                    return None;
                }
                Some(port.parse::<u16>().ok().filter(|port| *port >= 1024)?)
            } else {
                None
            };
            Some(Entry {
                codex: page.starts_with("agent:codex:"),
                application: page.starts_with("agent:application:"),
                id: id.to_owned(),
                title: name,
                startup_page: decode(page, 4096).unwrap_or_default(),
                agent_port,
                file_manager,
            })
        })();
        if let Some(item) = item {
            list.push(item);
        }
    }
    Ok(list)
}

pub fn run(
    host: &str,
    key: Option<&Path>,
    request: &str,
    repaint: impl FnOnce() + Send + 'static,
) -> io::Result<Receiver<io::Result<String>>> {
    run_with_input(host, key, request, None, repaint)
}

pub fn run_with_input(
    host: &str,
    key: Option<&Path>,
    request: &str,
    input: Option<Vec<u8>>,
    repaint: impl FnOnce() + Send + 'static,
) -> io::Result<Receiver<io::Result<String>>> {
    let mut cmd = ssh_command(
        host,
        key,
        &format!("exec ~/.local/bin/rectangle-session {request}"),
    )?;
    if input.is_none() {
        cmd.stdin(std::process::Stdio::null());
    }
    run_command(cmd, input, repaint)
}

fn run_command(
    mut cmd: std::process::Command,
    input: Option<Vec<u8>>,
    repaint: impl FnOnce() + Send + 'static,
) -> io::Result<Receiver<io::Result<String>>> {
    let (send, receive) = mpsc::channel();
    thread::Builder::new()
        .name("rectangle-request".into())
        .spawn(move || {
            // Creating ssh.exe can wait on Windows security scans. Keep process
            // creation off the UI thread as well as the connection and response.
            let mut child = match cmd.spawn() {
                Ok(child) => child,
                Err(error) => {
                    let _ = send.send(Err(error));
                    repaint();
                    return;
                }
            };
            let mut stderr = child.stderr.take().unwrap().take(4096);
            let diagnostic = thread::spawn(move || {
                let mut bytes = Vec::new();
                let _ = stderr.read_to_end(&mut bytes);
                String::from_utf8_lossy(&bytes).trim().to_owned()
            });
            let mut data = String::new();
            let result = (|| {
                if let Some(input) = input {
                    child.stdin.take().unwrap().write_all(&input)?;
                }
                child
                    .stdout
                    .take()
                    .unwrap()
                    .take(110_001)
                    .read_to_string(&mut data)?;
                if data.len() > 110_000 {
                    return Err(io::Error::other("Saved session response too large"));
                }
                if child.wait()?.success() {
                    Ok(data)
                } else {
                    let error = diagnostic.join().unwrap_or_default();
                    Err(io::Error::other(if error.is_empty() {
                        "Saved session request failed".into()
                    } else {
                        error
                    }))
                }
            })();
            if result.is_err() {
                let _ = child.kill();
                let _ = child.wait();
            }
            let _ = send.send(result);
            repaint();
        })?;
    Ok(receive)
}

#[test]
fn request_reports_process_creation_failure_and_repaints() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let repainted = Arc::new(AtomicBool::new(false));
    let signal = repainted.clone();
    let command = std::process::Command::new("rectangle-missing-ssh-test-executable");
    let task = run_command(command, None, move || signal.store(true, Ordering::Release)).unwrap();
    assert_eq!(
        task.recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !repainted.load(Ordering::Acquire) {
        assert!(std::time::Instant::now() < deadline);
        thread::yield_now();
    }
    assert!(run("-invalid-host", None, "list-v2", || {}).is_err());
}

#[test]
#[ignore = "requires a live Rectangle host and SSH key"]
fn background_requests_read_the_host_library_and_status() {
    let host = std::env::var("RECTANGLE_SMOKE_HOST").unwrap();
    let key = std::env::var_os("RECTANGLE_SMOKE_IDENTITY").map(std::path::PathBuf::from);
    let library = run(&host, key.as_deref(), "list-v2", || {}).unwrap();
    let status = run(&host, key.as_deref(), "proton-status", || {}).unwrap();
    let timeout = std::time::Duration::from_secs(20);
    parse(&library.recv_timeout(timeout).unwrap().unwrap()).unwrap();
    let response = status.recv_timeout(timeout).unwrap().unwrap();
    assert!(response.starts_with("RECTPROTON1\n"));
}
