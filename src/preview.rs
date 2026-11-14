use std::{
    io,
    net::{Shutdown, SocketAddrV4, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

pub struct Preview {
    url: String,
    stop: Arc<AtomicBool>,
}

impl Preview {
    pub fn start(host: &str, key: Option<&Path>, token: &str) -> io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let url = format!("http://{}/", listener.local_addr()?);
        let stop = Arc::new(AtomicBool::new(false));
        let running = Arc::clone(&stop);
        let host = host.to_owned();
        let key = key.map(Path::to_path_buf);
        let remote = format!("exec ~/.local/bin/rectangle-session agent-preview {token}");
        thread::Builder::new()
            .name("preview-listener".into())
            .spawn(move || {
                let mut workers: Vec<thread::JoinHandle<()>> = Vec::new();
                while !running.load(Ordering::Relaxed) {
                    workers.retain(|worker| !worker.is_finished());
                    match listener.accept() {
                        Ok((socket, _)) if workers.len() < 24 => {
                            let command =
                                crate::session::ssh_command(&host, key.as_deref(), &remote);
                            let stop = Arc::clone(&running);
                            if let Ok(worker) = thread::Builder::new().spawn(move || {
                                if let Ok(command) = command {
                                    let _ = relay(socket, command, &stop);
                                }
                            }) {
                                workers.push(worker);
                            }
                        }
                        Ok(_) => {}
                        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(25));
                        }
                        Err(_) => break,
                    }
                }
            })?;
        Ok(Self { url, stop })
    }

    pub fn open_brave(&self) -> io::Result<()> {
        open_brave(&self.url)
    }
}

pub fn published_url(report: &str, container_port: u16) -> Option<String> {
    let mut lines = report.lines();
    if lines.next()? != "Published on the host" {
        return None;
    }
    let prefix = format!("{container_port}/tcp -> ");
    lines.take_while(|line| !line.is_empty()).find_map(|line| {
        let endpoint: SocketAddrV4 = line.strip_prefix(&prefix)?.parse().ok()?;
        let ip = endpoint.ip();
        (!ip.is_loopback() && !ip.is_unspecified() && !ip.is_multicast() && endpoint.port() != 0)
            .then(|| format!("http://{endpoint}/"))
    })
}

pub fn open_brave(url: &str) -> io::Result<()> {
    let mut child = Command::new(brave_path()?).arg(url).spawn()?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn brave_path() -> io::Result<PathBuf> {
    ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(|base| PathBuf::from(base).join("BraveSoftware/Brave-Browser/Application/brave.exe"))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Brave was not found on this computer.",
            )
        })
}

fn relay(socket: TcpStream, mut command: Command, stop: &AtomicBool) -> io::Result<()> {
    let mut input = socket.try_clone()?;
    let mut output = socket.try_clone()?;
    let mut child = command.stderr(Stdio::null()).spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    thread::scope(|scope| {
        let upload = scope.spawn(move || {
            let _ = io::copy(&mut input, &mut stdin);
        });
        let download = scope.spawn(move || {
            let _ = io::copy(&mut stdout, &mut output);
            let _ = output.shutdown(Shutdown::Write);
        });
        while !stop.load(Ordering::Relaxed) && !download.is_finished() {
            thread::sleep(Duration::from_millis(50));
        }
        let _ = socket.shutdown(Shutdown::Both);
        let _ = child.kill();
        let _ = child.wait();
        let _ = upload.join();
        let _ = download.join();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn preview_uses_the_published_lan_address_for_this_port() {
        let report = "Published on the host\n4007/tcp -> 127.0.0.1:4007\n4007/tcp -> 192.168.50.10:4007\n\nInside the container\nTCP   0.0.0.0:4007";
        assert_eq!(
            published_url(report, 4007).as_deref(),
            Some("http://192.168.50.10:4007/")
        );
        assert_eq!(published_url(report, 4008), None);
        let private =
            "Published on the host\n4007/tcp -> 127.0.0.1:4007\n\n4007/tcp -> 192.168.50.10:4007";
        assert_eq!(published_url(private, 4007), None);
    }

    #[test]
    #[ignore = "requires a live SSH Agent with a test HTTP server"]
    fn forwards_http_through_the_restricted_session_key() {
        let host = std::env::var("RECTANGLE_SMOKE_HOST").unwrap();
        let key = std::env::var_os("RECTANGLE_SMOKE_IDENTITY").map(PathBuf::from);
        let token = std::env::var("RECTANGLE_SMOKE_AGENT").unwrap();
        let preview = Preview::start(&host, key.as_deref(), &token).unwrap();
        let addr = preview
            .url
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_owned();
        assert!(addr.starts_with("127.0.0.1:"));
        let mut socket = TcpStream::connect(&*addr).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(12)))
            .unwrap();
        write!(socket, "GET / HTTP/1.0\r\nHost: {addr}\r\n\r\n").unwrap();
        let mut response = Vec::new();
        socket.read_to_end(&mut response).unwrap();
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.0 200"), "{response}");
        assert!(
            response.ends_with("Rectangle preview fixture\n"),
            "{response}"
        );
        drop(preview);
        for _ in 0..20 {
            if TcpStream::connect(&*addr).is_err() {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!("The local preview port was not closed");
    }
}
