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

    pub fn path(&self) -> Option<String> {
        self.path.lock().unwrap().clone()
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

pub fn shell_command(host: &str, token: &str, path: &str) -> String {
    let docker = "docker exec -it -w";
    let target = format!("rectangle-tab-{token} /bin/sh");
    let remote = if path
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
    {
        format!("{docker} {path} {target}")
    } else if path
        .chars()
        .any(|c| c.is_control() || "\\\"$`%!".contains(c))
    {
        // CMD and PowerShell expand different characters inside double quotes.
        // Keep those characters out of the pasted command and decode on the host.
        let encoded = crate::saved::encode(path);
        format!(
            "python3 -c 'import os,sys; a=sys.argv[1:]; a[4]=os.fsdecode(bytes.fromhex(a[4])); os.execvp(a[0],a)' {docker} {encoded} {target}"
        )
    } else {
        let escaped = path.replace('\'', "'\\''");
        format!("{docker} '{escaped}' {target}")
    };
    format!("ssh -t {host} \"{remote}\"")
}

#[test]
fn shell_command_quotes_the_folder_for_windows_and_the_remote_shell() {
    assert_eq!(
        shell_command("user@host", "token", "/agent/home/workspace"),
        r#"ssh -t user@host "docker exec -it -w /agent/home/workspace rectangle-tab-token /bin/sh""#
    );
    assert_eq!(
        shell_command("user@host", "token", "/Music/Sam's album"),
        r#"ssh -t user@host "docker exec -it -w '/Music/Sam'\''s album' rectangle-tab-token /bin/sh""#
    );
    let path = "/Music/$x`x\"x%x!\nx";
    let command = shell_command("user@host", "token", path);
    assert!(command.contains(&crate::saved::encode(path)));
    assert!(!command.contains(path));
}
