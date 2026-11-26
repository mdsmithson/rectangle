use crate::{AgentView, Backend, Job, Rectangle, Snap, Tab, files, saved, settings};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, fs, io, path::Path};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Key {
    pub host: String,
    pub profile: String,
    pub view: String,
    pub editor: Option<(String, String)>,
}
impl Key {
    fn same_profile(&self, other: &Self) -> bool {
        self.host == other.host && self.profile == other.profile
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pane {
    pub key: Key,
    pub title: String,
    pub snap: Option<String>,
    pub display: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    version: u8,
    pub home: bool,
    pub library_host: String,
    pub active: Option<Key>,
    pub tabs: Vec<Pane>,
    pub minimized: Vec<Pane>,
}

#[derive(Default)]
pub struct State {
    pub editor_display: Option<u16>,
    pub layout: Option<Layout>,
    pub remaining: Vec<Pane>,
    pub minimized: Vec<Pane>,
    pub focus_pending: bool,
    pub applying: bool,
    pub frozen: bool,
    applied: Vec<u64>,
    last: Option<Layout>,
    saved: VecDeque<(Backend, String, saved::Entry, Option<Snap>)>,
    opening: Option<Key>,
}

impl State {
    pub fn is_opening(&self, host: &str, profile: &str) -> bool {
        self.opening
            .as_ref()
            .is_some_and(|key| key.host == host && key.profile == profile)
    }
    pub fn closed(&mut self, key: &Key, owner: bool) {
        self.remaining.retain(|pane| {
            if owner {
                !pane.key.same_profile(key)
            } else {
                &pane.key != key
            }
        });
        if owner {
            self.saved
                .retain(|(_, host, entry, _)| host != &key.host || entry.id != key.profile);
        }
        self.focus_pending = false;
    }
    pub fn load() -> io::Result<Self> {
        Self::load_from(&settings::workspace_path()?)
    }
    fn load_from(path: &Path) -> io::Result<Self> {
        let data = match fs::read(path) {
            Ok(data) => data,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        if data.len() > 256 * 1024 {
            return Err(io::Error::other("Workspace settings are too large."));
        }
        let layout: Layout = ron::de::from_bytes(&data).map_err(io::Error::other)?;
        if layout.version != 1 || layout.tabs.len() + layout.minimized.len() > 128 {
            return Err(io::Error::other("Invalid workspace settings."));
        }
        for pane in layout.tabs.iter().chain(&layout.minimized) {
            if !saved::valid_id(&pane.key.profile)
                || pane.key.host.len() > 254
                || !matches!(
                    pane.key.view.as_str(),
                    "main" | "freebuff" | "codex" | "files" | "sh" | "browser" | "edit"
                )
                || pane
                    .snap
                    .as_deref()
                    .is_some_and(|snap| Snap::parse(snap).is_none())
                || pane
                    .display
                    .is_some_and(|display| !(103..=127).contains(&display))
                || pane.key.editor.as_ref().is_some_and(|(folder, name)| {
                    !folder.starts_with('/')
                        || folder.len() > 4096
                        || folder.contains('\0')
                        || name.is_empty()
                        || name.len() > 255
                        || name.contains(['/', '\0'])
                })
            {
                return Err(io::Error::other("Invalid workspace tab."));
            }
        }
        Ok(Self {
            remaining: layout.tabs.clone(),
            minimized: layout.minimized.clone(),
            focus_pending: true,
            last: Some(layout.clone()),
            layout: Some(layout),
            ..Default::default()
        })
    }

    fn save_to(&mut self, path: &Path, layout: Layout) -> io::Result<()> {
        if self.last.as_ref() == Some(&layout) {
            return Ok(());
        }
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| io::Error::other("Missing workspace directory"))?,
        )?;
        let data = ron::ser::to_string(&layout).map_err(io::Error::other)?;
        let temporary = path.with_extension("tmp");
        fs::write(&temporary, data)?;
        fs::rename(temporary, path)?;
        self.last = Some(layout);
        Ok(())
    }

    fn forget_remaining(&mut self, key: &Key) {
        self.remaining.retain(|pane| !pane.key.same_profile(key));
    }
}

impl Rectangle {
    pub(super) fn workspace_key(&self, tab: &Tab) -> Option<Key> {
        let owner = tab
            .agent_owner
            .and_then(|id| self.tabs.iter().find(|owner| owner.id == id))
            .unwrap_or(tab);
        let profile = owner
            .profile
            .as_deref()
            .or_else(|| owner.session.as_ref().map(|session| session.token.as_str()))?;
        Some(Key {
            host: owner.connection.host.clone(),
            profile: profile.into(),
            view: if tab.file_editor.is_some() {
                "edit"
            } else if tab.agent_owner.is_some() {
                tab.agent_view.command()
            } else {
                "main"
            }
            .into(),
            editor: tab.editor_file.clone(),
        })
    }

    fn workspace_pane(&self, tab: &Tab) -> Option<Pane> {
        Some(Pane {
            key: self.workspace_key(tab)?,
            title: tab.title.clone(),
            snap: tab.snap.map(|snap| snap.code().into()),
            display: tab.file_editor,
        })
    }

    pub(super) fn persist_workspace(&mut self) {
        if self.workspace_state.frozen
            || (self.workspace_state.layout.is_none() && !self.startup.is_empty())
        {
            return;
        }
        let layout = self.workspace_snapshot();
        if let Err(error) =
            settings::workspace_path().and_then(|path| self.workspace_state.save_to(&path, layout))
        {
            self.error = Some(format!("Could not remember the workspace: {error}"));
        }
    }

    fn workspace_snapshot(&self) -> Layout {
        let mut tabs: Vec<_> = self
            .tabs
            .iter()
            .filter_map(|tab| self.workspace_pane(tab))
            .collect();
        // Keep unavailable hosts and requests still in flight in the snapshot.
        for pane in &self.workspace_state.remaining {
            if !tabs.iter().any(|current| current.key == pane.key) {
                tabs.push(pane.clone());
            }
        }
        if let Some(layout) = &self.workspace_state.layout {
            tabs.sort_by_key(|pane| {
                layout
                    .tabs
                    .iter()
                    .position(|original| original.key == pane.key)
                    .unwrap_or(usize::MAX)
            });
        }
        let mut minimized = self.workspace_state.minimized.clone();
        if let Some((Job::Park(id, _, true), _)) = &self.job
            && let Some(key) = self
                .tabs
                .iter()
                .find(|tab| tab.id == *id)
                .and_then(|tab| self.workspace_key(tab))
        {
            minimized.retain(|pane| !pane.key.same_profile(&key));
            minimized.extend(
                tabs.iter()
                    .filter(|pane| pane.key.same_profile(&key))
                    .cloned(),
            );
            tabs.retain(|pane| !pane.key.same_profile(&key));
        }
        let actual = self
            .tabs
            .get(self.active)
            .and_then(|tab| self.workspace_key(tab))
            .filter(|key| tabs.iter().any(|pane| &pane.key == key));
        let active = if self.workspace_state.focus_pending {
            self.workspace_state
                .layout
                .as_ref()
                .and_then(|layout| layout.active.clone())
                .or(actual)
        } else {
            actual
        };
        let home = if self.workspace_state.focus_pending {
            self.workspace_state
                .layout
                .as_ref()
                .is_some_and(|layout| layout.home)
        } else {
            self.home
        };
        Layout {
            version: 1,
            home: home || tabs.is_empty(),
            library_host: self.connection(self.library_host).host,
            active,
            tabs,
            minimized,
        }
    }

    pub(super) fn freeze_workspace(&mut self) {
        self.persist_workspace();
        self.workspace_state.frozen = true;
    }

    pub(super) fn remember_minimized(&mut self, index: usize) {
        let Some(key) = self.workspace_key(&self.tabs[index]) else {
            return;
        };
        let panes: Vec<_> = self
            .tabs
            .iter()
            .filter_map(|tab| self.workspace_pane(tab))
            .filter(|pane| pane.key.same_profile(&key))
            .collect();
        self.workspace_state
            .minimized
            .retain(|pane| !pane.key.same_profile(&key));
        self.workspace_state.minimized.extend(panes);
        self.workspace_state.forget_remaining(&key);
        self.workspace_state.focus_pending = false;
    }

    pub(super) fn unminimize_workspace(&mut self, host: &str, profile: &str) {
        let key = Key {
            host: host.into(),
            profile: profile.into(),
            view: "main".into(),
            editor: None,
        };
        let panes: Vec<_> = self
            .workspace_state
            .minimized
            .iter()
            .filter(|pane| pane.key.same_profile(&key))
            .cloned()
            .collect();
        self.workspace_state
            .minimized
            .retain(|pane| !pane.key.same_profile(&key));
        self.workspace_state.remaining.extend(panes.clone());
        if let Some(layout) = &mut self.workspace_state.layout {
            layout.tabs.extend(panes);
        }
    }

    pub(super) fn restore_workspace_host(
        &mut self,
        ctx: &egui::Context,
        backend: Backend,
        entries: &[saved::Entry],
        parked: &[saved::Parked],
        available: bool,
    ) {
        let host = self.connection(backend).host;
        if available {
            self.workspace_state.minimized.retain(|pane| {
                pane.key.host != host
                    || entries.iter().any(|entry| entry.id == pane.key.profile)
                    || parked.iter().any(|entry| {
                        entry.profile.as_deref().unwrap_or(&entry.token) == pane.key.profile
                    })
            });
        }
        let rows: Vec<_> = self
            .workspace_state
            .remaining
            .iter()
            .filter(|pane| pane.key.host == host && pane.key.view == "main")
            .cloned()
            .collect();
        self.workspace_state.applying = true;
        for pane in rows {
            if let Some(entry) = parked
                .iter()
                .find(|entry| entry.profile.as_deref().unwrap_or(&entry.token) == pane.key.profile)
            {
                self.resume_parked_with_focus(ctx, backend, entry.clone(), false);
            } else if let Some(entry) = entries.iter().find(|entry| entry.id == pane.key.profile) {
                self.workspace_state.saved.push_back((
                    backend,
                    host.clone(),
                    entry.clone(),
                    pane.snap.as_deref().and_then(Snap::parse),
                ));
            } else if available {
                self.workspace_state.forget_remaining(&pane.key);
                self.error = Some(format!(
                    "{} is no longer saved or parked on {host}.",
                    pane.title
                ));
            }
        }
        self.workspace_state.applying = false;
        self.restore_workspace_tick(ctx);
    }

    pub(super) fn restore_workspace_tick(&mut self, ctx: &egui::Context) {
        if self.quitting {
            return;
        }
        if self.job.is_none() {
            if let Some(key) = self.workspace_state.opening.take()
                && !self
                    .tabs
                    .iter()
                    .any(|tab| self.workspace_key(tab).as_ref() == Some(&key))
            {
                self.workspace_state.forget_remaining(&key);
            }
            if self.tabs.len() < crate::MAX_TABS
                && let Some((backend, host, entry, snap)) = self.workspace_state.saved.pop_front()
                && host == self.connection(backend).host
            {
                let key = Key {
                    host,
                    profile: entry.id.clone(),
                    view: "main".into(),
                    editor: None,
                };
                if !self
                    .tabs
                    .iter()
                    .any(|tab| self.workspace_key(tab).as_ref() == Some(&key))
                {
                    self.workspace_state.opening = Some(key);
                    if entry.agent_port.is_some() {
                        self.request(
                            ctx,
                            Job::PrepareAgentMedia(backend, entry.clone(), snap),
                            backend,
                            &format!("agent-prepare-media {}", entry.id),
                        );
                    } else {
                        self.workspace_state.applying = true;
                        self.open_tab(ctx, backend, Some(entry), None);
                        self.workspace_state.applying = false;
                    }
                }
            }
        }
        let rows = self.workspace_state.remaining.clone();
        let previous_selection = self.tabs.get(self.active).map(|tab| tab.id);
        let previous_home = self.home;
        self.workspace_state.applying = true;
        for pane in &rows {
            if pane.key.view == "main" || self.tabs.len() >= crate::MAX_TABS {
                continue;
            }
            if self
                .tabs
                .iter()
                .any(|tab| self.workspace_key(tab).as_ref() == Some(&pane.key))
            {
                continue;
            }
            let owner = self.tabs.iter().position(|tab| {
                tab.agent_owner.is_none()
                    && tab.ran
                    && tab.preferences.ready
                    && self
                        .workspace_key(tab)
                        .is_some_and(|key| key.same_profile(&pane.key))
            });
            let Some(owner) = owner else { continue };
            if let Some((directory, name)) = &pane.key.editor {
                self.workspace_state.editor_display = pane.display;
                self.open_file_editor(
                    ctx,
                    owner,
                    files::FileEditor {
                        directory: directory.clone(),
                        name: name.clone(),
                    },
                );
            } else if let Some(kind) = [
                AgentView::Freebuff,
                AgentView::Codex,
                AgentView::Files,
                AgentView::Shell,
                AgentView::Browser,
            ]
            .into_iter()
            .find(|kind| kind.command() == pane.key.view)
            {
                self.open_agent_view(ctx, owner, kind);
            }
        }
        self.workspace_state.applying = false;
        if !self.workspace_state.focus_pending {
            if let Some(index) = self
                .tabs
                .iter()
                .position(|tab| Some(tab.id) == previous_selection)
            {
                self.active = index;
            }
            self.home = previous_home;
        }
        let selected = self.tabs.get(self.active).map(|tab| tab.id);
        let panes: Vec<_> = self
            .tabs
            .iter()
            .filter_map(|tab| Some((tab.id, self.workspace_key(tab)?)))
            .collect();
        for (id, key) in &panes {
            if !self.workspace_state.applied.contains(id)
                && self
                    .workspace_state
                    .remaining
                    .iter()
                    .any(|pane| &pane.key == key)
                && let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == *id)
            {
                tab.snap = None;
            }
        }
        for (id, key) in &panes {
            if self.workspace_state.applied.contains(id) {
                continue;
            }
            if let Some(pane) = self
                .workspace_state
                .remaining
                .iter()
                .find(|pane| &pane.key == key)
                .cloned()
            {
                let snap = pane.snap.as_deref().and_then(Snap::parse).filter(|slot| {
                    !self.tabs.iter().any(|other| {
                        other.id != *id && other.snap.is_some_and(|other| other.overlaps(*slot))
                    })
                });
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == *id) {
                    tab.snap = snap;
                    if tab.agent_owner.is_some() {
                        tab.title = pane.title;
                    }
                }
                self.workspace_state.applied.push(*id);
                self.workspace_state
                    .remaining
                    .retain(|pane| &pane.key != key);
            }
        }
        if let Some(layout) = &self.workspace_state.layout {
            let ordering: Vec<_> = self
                .tabs
                .iter()
                .map(|tab| (tab.id, self.workspace_key(tab)))
                .collect();
            self.tabs.sort_by_key(|tab| {
                ordering
                    .iter()
                    .find(|(id, _)| *id == tab.id)
                    .and_then(|(_, key)| {
                        layout
                            .tabs
                            .iter()
                            .position(|pane| Some(&pane.key) == key.as_ref())
                    })
                    .unwrap_or(usize::MAX)
            });
            if let Some(index) = self.tabs.iter().position(|tab| Some(tab.id) == selected) {
                self.active = index;
            }
            if self.workspace_state.focus_pending {
                if let Some(index) = self
                    .tabs
                    .iter()
                    .position(|tab| self.workspace_key(tab).as_ref() == layout.active.as_ref())
                {
                    self.active = index;
                    if !layout.home {
                        self.home = false;
                    }
                }
                if layout.home {
                    self.home = true;
                } else if layout.active.as_ref().is_none_or(|key| {
                    !self
                        .tabs
                        .iter()
                        .any(|tab| self.workspace_key(tab).as_ref() == Some(key))
                        && !self
                            .workspace_state
                            .remaining
                            .iter()
                            .any(|pane| &pane.key == key)
                }) {
                    self.workspace_state.focus_pending = false;
                    self.home = self.tabs.is_empty();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Connection, startup};

    fn pane(letter: char, snap: Option<Snap>) -> Pane {
        Pane {
            key: Key {
                host: "server".into(),
                profile: letter.to_string().repeat(32),
                view: "main".into(),
                editor: None,
            },
            title: letter.to_string(),
            snap: snap.map(|snap| snap.code().into()),
            display: None,
        }
    }

    fn layout(tabs: Vec<Pane>, minimized: Vec<Pane>, active: Option<Key>) -> Layout {
        Layout {
            version: 1,
            home: false,
            library_host: "server".into(),
            active,
            tabs,
            minimized,
        }
    }

    #[test]
    fn asynchronous_arrival_restores_original_order_selection_and_local_tiles() {
        let left = pane('a', Some(Snap::Left));
        let right = pane('b', Some(Snap::Right));
        let layout = layout(
            vec![left.clone(), right.clone()],
            vec![],
            Some(right.key.clone()),
        );
        let mut app = Rectangle {
            host: "server".into(),
            tabs: vec![
                Tab {
                    id: 2,
                    profile: Some(right.key.profile.clone()),
                    connection: Connection {
                        host: "server".into(),
                        key: None,
                    },
                    snap: Some(Snap::Left),
                    ..Default::default()
                },
                Tab {
                    id: 1,
                    profile: Some(left.key.profile.clone()),
                    connection: Connection {
                        host: "server".into(),
                        key: None,
                    },
                    snap: Some(Snap::Right),
                    ..Default::default()
                },
            ],
            workspace_state: State {
                remaining: layout.tabs.clone(),
                layout: Some(layout),
                focus_pending: true,
                ..Default::default()
            },
            ..Default::default()
        };
        app.restore_workspace_tick(&egui::Context::default());
        assert_eq!(
            app.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(app.tabs[0].snap, Some(Snap::Left));
        assert_eq!(app.tabs[1].snap, Some(Snap::Right));
        assert_eq!(app.tabs[app.active].id, 2);
        assert!(app.workspace_state.remaining.is_empty());
        // A late update must not undo an explicit selection.
        app.select_tab(0);
        app.restore_workspace_tick(&egui::Context::default());
        assert_eq!(app.tabs[app.active].id, 1);
    }

    #[test]
    fn minimizing_keeps_linked_views_out_of_the_next_visible_workspace() {
        let root = pane('a', Some(Snap::Left));
        let mut app = Rectangle {
            host: "server".into(),
            tabs: vec![
                Tab {
                    id: 1,
                    profile: Some(root.key.profile.clone()),
                    connection: Connection {
                        host: "server".into(),
                        key: None,
                    },
                    snap: Some(Snap::Left),
                    ..Default::default()
                },
                Tab {
                    id: 2,
                    agent_owner: Some(1),
                    agent_view: AgentView::Files,
                    connection: Connection {
                        host: "server".into(),
                        key: None,
                    },
                    snap: Some(Snap::Right),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        app.remember_minimized(0);
        app.close_tab(0);
        let snapshot = app.workspace_snapshot();
        assert!(snapshot.tabs.is_empty());
        assert_eq!(snapshot.minimized.len(), 2);
        assert!(
            snapshot
                .minimized
                .iter()
                .any(|pane| pane.key.view == "files")
        );
        assert!(snapshot.home);
        app.workspace_state.layout = Some(snapshot);
        app.finish_startup_host(
            &egui::Context::default(),
            Backend::Primary,
            startup::Snapshot {
                entries: Some(vec![]),
                parked: Some(vec![saved::Parked {
                    token: "b".repeat(32),
                    profile: Some(root.key.profile.clone()),
                    title: "Agent".into(),
                    codex: false,
                    application: false,
                    agent_port: Some(4007),
                    file_manager: false,
                    always_on: true,
                    snap: Some(Snap::Left),
                    expires: 0,
                }]),
                errors: vec![],
            },
        );
        assert!(app.tabs.is_empty());
        assert_eq!(app.parked.len(), 1);
        app.unminimize_workspace("server", &root.key.profile);
        assert!(app.workspace_state.minimized.is_empty());
        assert_eq!(app.workspace_state.remaining.len(), 2);
    }

    #[test]
    fn shutdown_does_not_replace_the_snapshot_with_an_empty_workspace() {
        let pane = pane('a', Some(Snap::TopLeft));
        let snapshot = layout(vec![pane], vec![], None);
        let mut app = Rectangle {
            quitting: true,
            workspace_state: State {
                frozen: true,
                last: Some(snapshot.clone()),
                ..Default::default()
            },
            ..Default::default()
        };
        app.persist_workspace();
        assert_eq!(app.workspace_state.last, Some(snapshot));
    }

    #[test]
    #[ignore = "requires a live Rectangle host; creates and cleans up its own browser"]
    fn parks_and_restores_an_owned_browser_without_resuming_minimized_tabs() {
        use std::time::{Duration, Instant};
        let host = std::env::var("RECTANGLE_SMOKE_HOST").unwrap();
        let key = std::env::var_os("RECTANGLE_SMOKE_IDENTITY").map(std::path::PathBuf::from);
        struct Cleanup {
            host: String,
            key: Option<std::path::PathBuf>,
            token: Option<String>,
            path: Option<std::path::PathBuf>,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Some(token) = &self.token
                    && let Ok(task) = saved::run(
                        &self.host,
                        self.key.as_deref(),
                        &format!("stop-park {token}"),
                        || {},
                    )
                {
                    let _ = task.recv_timeout(Duration::from_secs(15));
                }
                if let Some(path) = &self.path {
                    let _ = fs::remove_file(path);
                }
            }
        }
        let mut cleanup = Cleanup {
            host: host.clone(),
            key: key.clone(),
            token: None,
            path: None,
        };
        let ctx = egui::Context::default();
        let mut app = Rectangle {
            host: host.clone(),
            identity: key.clone(),
            home: true,
            ..Default::default()
        };
        let wait_frame = |app: &mut Rectangle| {
            let deadline = Instant::now() + Duration::from_secs(45);
            loop {
                app.tabs[0].poll(&ctx);
                assert!(app.tabs[0].error.is_none(), "{:?}", app.tabs[0].error);
                if app.tabs[0].ran {
                    break;
                }
                assert!(Instant::now() < deadline, "browser did not produce a frame");
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        let wait_job = |app: &mut Rectangle| {
            let deadline = Instant::now() + Duration::from_secs(30);
            while app.job.is_some() {
                app.poll_job(&ctx);
                assert!(app.error.is_none(), "{:?}", app.error);
                assert!(Instant::now() < deadline, "parking did not finish");
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        let wait_startup = |app: &mut Rectangle| {
            let deadline = Instant::now() + Duration::from_secs(30);
            app.find_parked_on_startup(&ctx);
            while !app.startup.is_empty() {
                app.poll_startup(&ctx);
                assert!(app.error.is_none(), "{:?}", app.error);
                assert!(Instant::now() < deadline, "startup did not finish");
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        app.open_tab(&ctx, Backend::Primary, None, None);
        let token = app.tabs[0].session.as_ref().unwrap().token.clone();
        cleanup.token = Some(token.clone());
        let path = std::env::temp_dir().join(format!("rectangle-workspace-{token}.ron"));
        cleanup.path = Some(path.clone());
        wait_frame(&mut app);
        app.tabs[0].snap = Some(Snap::Left);
        app.park_tab(&ctx, 0);
        wait_job(&mut app);
        assert!(app.tabs.is_empty());
        let minimized = app.workspace_snapshot();
        assert_eq!(minimized.minimized.len(), 1);
        State::default().save_to(&path, minimized).unwrap();

        let mut reopened = Rectangle {
            host: host.clone(),
            identity: key.clone(),
            home: true,
            workspace_state: State::load_from(&path).unwrap(),
            ..Default::default()
        };
        wait_startup(&mut reopened);
        assert!(
            reopened.tabs.is_empty(),
            "minimized browser reopened itself"
        );
        let entry = reopened
            .parked
            .iter()
            .find(|entry| entry.token == token)
            .unwrap()
            .clone();
        reopened.resume_parked(&ctx, Backend::Primary, entry);
        wait_frame(&mut reopened);
        reopened.restore_workspace_tick(&ctx);
        assert_eq!(reopened.tabs[0].snap, Some(Snap::Left));
        let visible = reopened.workspace_snapshot();
        assert_eq!(visible.tabs.len(), 1);
        assert!(visible.minimized.is_empty());
        State::default().save_to(&path, visible).unwrap();
        reopened.quitting = true;
        reopened.park_tab(&ctx, 0);
        wait_job(&mut reopened);

        let mut restored = Rectangle {
            host,
            identity: key,
            workspace_state: State::load_from(&path).unwrap(),
            ..Default::default()
        };
        wait_startup(&mut restored);
        assert_eq!(restored.tabs.len(), 1);
        wait_frame(&mut restored);
        assert_eq!(restored.tabs[0].snap, Some(Snap::Left));
        assert_eq!(
            restored
                .workspace_key(&restored.tabs[restored.active])
                .unwrap()
                .profile,
            token
        );
    }
    #[test]
    fn saves_selection_tiles_and_minimized_views_and_rejects_invalid_versions() {
        let dir = std::env::temp_dir().join(format!("rectangle-workspace-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("workspace.ron");
        let key = Key {
            host: "Emerald@server".into(),
            profile: "a".repeat(32),
            view: "main".into(),
            editor: None,
        };
        let pane = Pane {
            key: key.clone(),
            title: "Agent".into(),
            snap: Some("left".into()),
            display: None,
        };
        let layout = Layout {
            version: 1,
            home: false,
            library_host: key.host.clone(),
            active: Some(key),
            tabs: vec![pane.clone()],
            minimized: vec![Pane {
                snap: Some("right".into()),
                ..pane
            }],
        };
        State::default().save_to(&path, layout.clone()).unwrap();
        let loaded = State::load_from(&path).unwrap();
        assert_eq!(loaded.layout, Some(layout));
        assert_eq!(loaded.remaining.len(), 1);
        assert_eq!(loaded.minimized.len(), 1);
        fs::write(
            &path,
            "(version:99,home:true,library_host:\"\",active:None,tabs:[],minimized:[])",
        )
        .unwrap();
        assert!(State::load_from(&path).is_err());
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
