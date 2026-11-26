use crate::{AgentResolution, Connection, saved};
use eframe::egui;
use std::{
    collections::VecDeque,
    io,
    sync::mpsc::{Receiver, TryRecvError},
};

type Task = Receiver<io::Result<String>>;
pub const DEFAULT_FOLDER: &str = "/agent/home/workspace";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Resolution,
    Smooth,
    Folder,
}
impl Field {
    fn name(self) -> &'static str {
        match self {
            Self::Resolution => "resolution",
            Self::Smooth => "smooth",
            Self::Folder => "folder",
        }
    }
}

pub struct Snapshot {
    pub resolution: Option<AgentResolution>,
    pub smooth: bool,
    pub folder: Option<String>,
}
impl Snapshot {
    fn parse(data: &str) -> io::Result<Self> {
        let invalid = || io::Error::other("The server returned invalid tab preferences.");
        let mut lines = data.lines();
        if lines.next() != Some("RECTPREF1") {
            return Err(invalid());
        }
        let resolution = match lines.next() {
            Some("-") => None,
            Some(value) => Some(AgentResolution::parse(value).ok_or_else(invalid)?),
            None => return Err(invalid()),
        };
        let smooth = match lines.next() {
            Some("on") => true,
            Some("off") => false,
            _ => return Err(invalid()),
        };
        let folder = match lines.next() {
            Some("-") => None,
            Some(value) => {
                let path = saved::decode(value, 8192)
                    .filter(|path| valid_folder(path))
                    .ok_or_else(invalid)?;
                Some(path)
            }
            None => return Err(invalid()),
        };
        if lines.next().is_some() {
            return Err(invalid());
        }
        Ok(Self {
            resolution,
            smooth,
            folder,
        })
    }
}

fn valid_folder(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= 4096
        && !path.contains('\0')
        && !path.split('/').any(|part| matches!(part, "." | ".."))
}

pub struct State {
    pub ready: bool,
    pub received: bool,
    pub folder: String,
    dirty: [bool; 3],
    started: bool,
    job: Option<Task>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            ready: false,
            received: false,
            folder: DEFAULT_FOLDER.into(),
            dirty: [false; 3],
            started: false,
            job: None,
        }
    }
}
impl State {
    pub fn changed(&mut self, field: Field) {
        self.dirty[field as usize] = true;
    }
    pub fn changed_locally(&self, field: Field) -> bool {
        self.dirty[field as usize]
    }
    pub fn poll(
        &mut self,
        connection: &Connection,
        target: &str,
        ctx: &egui::Context,
    ) -> Option<io::Result<Snapshot>> {
        if !self.started {
            self.started = true;
            let repaint = ctx.clone();
            match saved::run(
                &connection.host,
                connection.key.as_deref(),
                &format!("tab-preferences {target} get"),
                move || repaint.request_repaint(),
            ) {
                Ok(job) => self.job = Some(job),
                Err(error) => {
                    self.ready = true;
                    return Some(Err(error));
                }
            }
        }
        let result = take(&mut self.job)?;
        self.ready = true;
        let result = result.and_then(|data| Snapshot::parse(&data));
        self.received = result.is_ok();
        Some(result)
    }
}

struct Edit {
    connection: Connection,
    target: String,
    field: Field,
    value: String,
}

#[derive(Default)]
pub struct Writes {
    waiting: VecDeque<Edit>,
    job: Option<Task>,
}
impl Writes {
    pub fn push(&mut self, connection: &Connection, target: String, field: Field, value: String) {
        self.waiting.retain(|edit| {
            !(edit.connection.host == connection.host
                && edit.connection.key == connection.key
                && edit.target == target
                && edit.field == field)
        });
        self.waiting.push_back(Edit {
            connection: connection.clone(),
            target,
            field,
            value,
        });
    }
    pub fn pending(&self) -> bool {
        self.job.is_some() || !self.waiting.is_empty()
    }
    pub fn tick(&mut self, ctx: &egui::Context) -> Option<String> {
        let mut error = None;
        if let Some(result) = take(&mut self.job) {
            error = match result {
                Ok(data) if data.trim() == "OK" => None,
                Ok(_) => Some("The server did not confirm the tab preference.".into()),
                Err(error) => Some(error.to_string()),
            };
        }
        if self.job.is_none()
            && let Some(edit) = self.waiting.pop_front()
        {
            let repaint = ctx.clone();
            match saved::run(
                &edit.connection.host,
                edit.connection.key.as_deref(),
                &format!(
                    "tab-preferences {} {} {}",
                    edit.target,
                    edit.field.name(),
                    edit.value
                ),
                move || repaint.request_repaint(),
            ) {
                Ok(job) => self.job = Some(job),
                Err(problem) => error = Some(problem.to_string()),
            }
        }
        if self.pending() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        error
    }
}

fn take(task: &mut Option<Task>) -> Option<io::Result<String>> {
    let result = match task.as_ref()?.try_recv() {
        Err(TryRecvError::Empty) => return None,
        Err(TryRecvError::Disconnected) => {
            Err(io::Error::other("Tab preference request disconnected"))
        }
        Ok(result) => result,
    };
    *task = None;
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_shared_modes_and_unicode_folders_and_rejects_invalid_data() {
        let folder = "/agent/home/workspace/日本語 files";
        let snapshot =
            Snapshot::parse(&format!("RECTPREF1\nqhd\non\n{}\n", saved::encode(folder))).unwrap();
        assert!(snapshot.resolution == Some(AgentResolution::Qhd));
        assert!(snapshot.smooth);
        assert_eq!(snapshot.folder.as_deref(), Some(folder));
        for data in [
            "RECTPREF1\nbad\noff\n-",
            "RECTPREF1\n-\nmaybe\n-",
            "RECTPREF1\n-\noff\n2f2e2e",
            "RECTPREF1\n-\noff\n-\nextra",
        ] {
            assert!(Snapshot::parse(data).is_err());
        }
    }
    #[test]
    fn coalesces_same_field_without_losing_other_fields() {
        let mut writes = Writes::default();
        let connection = Connection {
            host: "server".into(),
            key: None,
        };
        writes.push(&connection, "profile".into(), Field::Smooth, "on".into());
        writes.push(&connection, "profile".into(), Field::Folder, "2f".into());
        writes.push(&connection, "profile".into(), Field::Smooth, "off".into());
        assert_eq!(writes.waiting.len(), 2);
        assert_eq!(writes.waiting.back().unwrap().value, "off");
    }

    #[test]
    fn modes_round_trip_and_loading_does_not_erase_local_choices() {
        for (mode, _) in AgentResolution::OPTIONS {
            assert!(AgentResolution::parse(mode.code()) == Some(mode));
        }
        let mut state = State::default();
        state.started = true;
        state.changed(Field::Resolution);
        let (send, receive) = std::sync::mpsc::channel();
        state.job = Some(receive);
        send.send(Ok("RECTPREF1\nuhd\non\n-\n".into())).unwrap();
        let snapshot = state
            .poll(&Connection::default(), "profile", &egui::Context::default())
            .unwrap()
            .unwrap();
        assert!(state.ready && state.changed_locally(Field::Resolution));
        assert!(snapshot.resolution == Some(AgentResolution::Uhd));
    }

    #[test]
    fn quit_waits_until_the_server_acknowledges_writes() {
        let (send, receive) = std::sync::mpsc::channel();
        let mut writes = Writes {
            job: Some(receive),
            ..Default::default()
        };
        assert!(writes.pending());
        assert!(writes.tick(&egui::Context::default()).is_none());
        assert!(writes.pending());
        send.send(Ok("OK\n".into())).unwrap();
        assert!(writes.tick(&egui::Context::default()).is_none());
        assert!(!writes.pending());
    }
}
