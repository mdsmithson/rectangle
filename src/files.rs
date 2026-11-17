use crate::{Connection, MUTED, saved};
use eframe::egui::{self, Color32, RichText};
use std::{
    io,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, TryRecvError},
    time::Duration,
};

mod smb;
mod transfer;
use transfer::{Job, Work};
const PAGE: usize = 150;
const HOME: &str = "/agent/home";

#[derive(Clone)]
struct Entry {
    kind: char,
    bytes: u64,
    name: String,
    created: String,
    modified: String,
}
impl Entry {
    fn kind(&self) -> &str {
        match self.kind {
            'd' => "Folder",
            'f' => Path::new(&self.name)
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("File"),
            _ => "Other",
        }
    }
    fn size(&self) -> String {
        if self.kind == 'd' {
            "—".into()
        } else {
            transfer::size(self.bytes)
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Layout {
    Large,
    Medium,
    Small,
    List,
    Details,
}
impl Layout {
    const ALL: [(Self, &'static str); 5] = [
        (Self::Large, "Large icons"),
        (Self::Medium, "Medium icons"),
        (Self::Small, "Small icons"),
        (Self::List, "List view"),
        (Self::Details, "Detail view"),
    ];
    fn label(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(layout, _)| *layout == self)
            .unwrap()
            .1
    }
}

#[derive(Clone)]
struct Drag {
    token: String,
    directory: String,
    name: String,
}
enum Prompt {
    Create {
        folder: bool,
        name: String,
        focus: bool,
    },
    Rename(Entry, String),
    Delete(Entry),
}
enum Action {
    Open(String),
    Prompt(Prompt),
    Work(Work),
    Download(Entry),
    Edit(Entry),
}
pub struct FileEditor {
    pub directory: String,
    pub name: String,
}

pub struct Explorer {
    path: String,
    back: Vec<String>,
    forward: Vec<String>,
    offset: usize,
    total: usize,
    entries: Vec<Entry>,
    layout: Layout,
    descending: bool,
    listing: Option<Receiver<io::Result<String>>>,
    job: Option<Job>,
    open_after_create: Option<FileEditor>,
    prompt: Option<Prompt>,
    status: Option<(bool, String)>,
    refresh: bool,
    shares: smb::Shares,
}

impl Default for Explorer {
    fn default() -> Self {
        Self {
            path: format!("{HOME}/workspace"),
            back: Vec::new(),
            forward: Vec::new(),
            offset: 0,
            total: 0,
            entries: Vec::new(),
            layout: Layout::Medium,
            descending: false,
            listing: None,
            job: None,
            open_after_create: None,
            prompt: None,
            status: None,
            refresh: true,
            shares: smb::Shares::default(),
        }
    }
}

impl Explorer {
    pub fn uploading(&self) -> bool {
        self.job.is_some()
    }
    fn reset(&mut self) {
        self.offset = 0;
        self.total = 0;
        self.entries.clear();
        self.status = None;
        self.listing = None;
        self.refresh = true;
        self.prompt = None;
    }
    fn open(&mut self, path: String) {
        if path != self.path {
            self.back.push(std::mem::replace(&mut self.path, path));
            self.forward.clear();
            self.reset();
        }
    }
    fn history(&mut self, back: bool) {
        let (from, to) = if back {
            (&mut self.back, &mut self.forward)
        } else {
            (&mut self.forward, &mut self.back)
        };
        if let Some(path) = from.pop() {
            to.push(std::mem::replace(&mut self.path, path));
            self.reset();
        }
    }
    pub fn address(&self, host: &str, token: &str) -> String {
        address(host, token, &self.path)
    }
    pub fn navigate(&mut self, host: &str, token: &str, value: &str) -> io::Result<()> {
        if self.job.is_some() {
            return Err(io::Error::other("Finish the active file operation first."));
        }
        let path = if value.starts_with('/') {
            value.to_owned()
        } else {
            let invalid = || io::Error::other("Enter an Agent folder URL or an absolute path.");
            let parsed = url::Url::parse(value).map_err(|_| invalid())?;
            if parsed.scheme() != "rectangle"
                || parsed.host_str() != Some("files")
                || parsed.query().is_some()
                || parsed.fragment().is_some()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.port().is_some()
            {
                return Err(invalid());
            }
            let mut parts = parsed
                .path_segments()
                .into_iter()
                .flatten()
                .map(decode_segment);
            if parts.next().transpose()?.as_deref() != Some(host)
                || parts.next().transpose()?.as_deref() != Some(token)
            {
                return Err(io::Error::other(
                    "This address belongs to a different host or Agent.",
                ));
            }
            let parts = parts.collect::<io::Result<Vec<_>>>()?;
            if parts.iter().any(|part| part.contains('/')) {
                return Err(invalid());
            }
            format!("/{}", parts.join("/"))
        };
        if path.len() > 4096
            || path.contains('\0')
            || path.split('/').any(|part| matches!(part, "." | ".."))
        {
            return Err(io::Error::other("Invalid Agent folder path."));
        }
        self.open(format!(
            "/{}",
            path.split('/')
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("/")
        ));
        Ok(())
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        connection: &Connection,
        token: &str,
        agent_name: &str,
        interactive: bool,
        accepts_drop: bool,
        pane: egui::Rect,
    ) -> (bool, Option<FileEditor>) {
        self.shares.tick(ui.ctx(), connection, token);
        let mut activity = false;
        let mut editor = None;
        if let Some(job) = &self.job {
            match job.result.try_recv() {
                Err(TryRecvError::Empty) => {
                    ui.ctx().request_repaint_after(Duration::from_millis(100))
                }
                result => {
                    let result = result.unwrap_or_else(|_| {
                        Err(io::Error::other("Operation stopped before confirmation."))
                    });
                    if let Some(file) = self.open_after_create.take()
                        && result.is_ok()
                    {
                        editor = Some(file);
                    }
                    self.job = None;
                    self.status = Some(match result {
                        Ok(()) => (true, "Done.".into()),
                        Err(error) => (false, error.to_string()),
                    });
                    self.refresh = true;
                }
            }
        }
        if let Some(listing) = &self.listing {
            match listing.try_recv() {
                Err(TryRecvError::Empty) => {}
                result => {
                    self.listing = None;
                    match result
                        .map_err(|_| io::Error::other("Directory listing disconnected."))
                        .and_then(|r| r)
                        .and_then(|data| parse_listing(&data))
                    {
                        Ok((total, entries)) => {
                            self.total = total;
                            if self.offset > 0 && self.offset >= total {
                                self.offset = total.saturating_sub(1) / PAGE * PAGE;
                                self.refresh = true;
                            }
                            self.entries = entries;
                        }
                        Err(error) => {
                            self.entries.clear();
                            self.status = Some((false, error.to_string()));
                        }
                    }
                }
            }
        }
        let enabled =
            interactive && self.listing.is_none() && self.job.is_none() && self.prompt.is_none();
        let mut action = None;
        let writable = writable(&self.path);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(enabled && !self.back.is_empty(), egui::Button::new("‹"))
                .on_hover_text("Back")
                .clicked()
            {
                self.history(true);
                activity = true;
            }
            if ui
                .add_enabled(enabled && !self.forward.is_empty(), egui::Button::new("›"))
                .on_hover_text("Forward")
                .clicked()
            {
                self.history(false);
                activity = true;
            }
            if ui
                .add_enabled(
                    enabled && self.path != "/",
                    egui::Button::new("Parent folder"),
                )
                .clicked()
            {
                let (parent, _) = self.path.rsplit_once('/').unwrap();
                action = Some(Action::Open(
                    if parent.is_empty() { "/" } else { parent }.into(),
                ));
            }
            if ui
                .add_enabled(enabled, egui::Button::new("Refresh"))
                .clicked()
            {
                self.shares.refresh();
                self.refresh = true;
                self.status = None;
                activity = true;
            }
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(if self.descending {
                        "↓ Z–A"
                    } else {
                        "↑ A–Z"
                    }),
                )
                .on_hover_text("Toggle name sort")
                .clicked()
            {
                self.descending = !self.descending;
                self.reset();
                activity = true;
            }
            ui.add_enabled_ui(interactive, |ui| {
                egui::ComboBox::from_id_salt("file layout")
                    .selected_text(self.layout.label())
                    .show_ui(ui, |ui| {
                        for (layout, label) in Layout::ALL {
                            ui.selectable_value(&mut self.layout, layout, label);
                        }
                    });
            });
            for (folder, label) in [(true, "New Folder"), (false, "New File")] {
                if ui
                    .add_enabled(enabled && writable, egui::Button::new(label))
                    .clicked()
                {
                    action = Some(Action::Prompt(Prompt::Create {
                        folder,
                        name: String::new(),
                        focus: true,
                    }));
                }
            }
        });
        ui.label(RichText::new(&self.path).monospace());
        if writable {
            ui.label(RichText::new("Drop local files in this pane to upload.").color(MUTED));
        }
        if let Some(job) = &self.job {
            job.show(ui, interactive);
        } else if let Some((ok, message)) = &self.status {
            ui.colored_label(if *ok { MUTED } else { Color32::LIGHT_RED }, message);
        }
        self.shares.dialog(ui, connection, token, interactive);
        if let Some(mut prompt) = self.prompt.take() {
            let mut keep = true;
            if let Prompt::Create {
                folder,
                name,
                focus,
            } = &mut prompt
            {
                let title = if *folder { "New Folder" } else { "New File" };
                egui::Window::new(title).id(ui.id().with("create agent item"))
                    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                    .collapsible(false).resizable(false).enabled(interactive)
                    .show(ui.ctx(), |ui| {
                        ui.label(if *folder { "Folder name" } else { "File name" });
                        let response = ui.text_edit_singleline(name);
                        if std::mem::take(focus) {
                            response.request_focus();
                        }
                        let valid = portable_name(name);
                        if !valid && !name.is_empty() {
                            ui.colored_label(Color32::LIGHT_RED, "Use a Linux and Windows compatible name without reserved characters, spaces at the ends, or a trailing dot.");
                        }
                        let enter = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        ui.horizontal(|ui| {
                            if ui.add_enabled(valid, egui::Button::new(if *folder { "Create folder" } else { "Create and edit" })).clicked() || (valid && enter) {
                                action = Some(Action::Work(Work::Create {
                                    directory: self.path.clone(), name: name.clone(), folder: *folder,
                                }));
                                keep = false;
                            }
                            if ui.button("Cancel").clicked() || ui.input(|input| input.key_pressed(egui::Key::Escape)) { keep = false; }
                        });
                    });
            } else {
                ui.add_enabled_ui(interactive, |ui| {
                    ui.group(|ui| {
                        match &mut prompt {
                            Prompt::Create { .. } => unreachable!(),
                            Prompt::Rename(entry, new_name) => {
                                ui.label(format!("Rename “{}”", entry.name));
                                ui.text_edit_singleline(new_name);
                                if ui
                                    .add_enabled(
                                        valid_name(new_name) && *new_name != entry.name,
                                        egui::Button::new("Rename"),
                                    )
                                    .clicked()
                                {
                                    action = Some(Action::Work(Work::Rename {
                                        directory: self.path.clone(),
                                        name: entry.name.clone(),
                                        to_directory: self.path.clone(),
                                        to_name: new_name.clone(),
                                    }));
                                    keep = false;
                                }
                            }
                            Prompt::Delete(entry) => {
                                ui.label(if entry.kind == 'd' {
                                    format!("Permanently delete “{}” and its contents?", entry.name)
                                } else {
                                    format!("Permanently delete “{}”?", entry.name)
                                });
                                if ui.button("Yes, delete").clicked() {
                                    action = Some(Action::Work(Work::Delete {
                                        directory: self.path.clone(),
                                        name: entry.name.clone(),
                                    }));
                                    keep = false;
                                }
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    })
                });
            }
            if keep {
                self.prompt = Some(prompt);
            }
        }
        ui.separator();
        if self.listing.is_some() {
            ui.spinner();
        }
        if self.total > PAGE {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        enabled && self.offset > 0,
                        egui::Button::new("Previous page"),
                    )
                    .clicked()
                {
                    self.offset = self.offset.saturating_sub(PAGE);
                    self.refresh = true;
                    activity = true;
                }
                if ui
                    .add_enabled(
                        enabled && self.offset.saturating_add(PAGE) < self.total,
                        egui::Button::new("Next page"),
                    )
                    .clicked()
                {
                    self.offset += PAGE;
                    self.refresh = true;
                    activity = true;
                }
            });
        }
        let path = &self.path;
        let scroll = if matches!(self.layout, Layout::List | Layout::Details) {
            egui::ScrollArea::both()
        } else {
            egui::ScrollArea::vertical()
        };
        scroll
            .id_salt(("files", path, self.offset))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut item = |ui: &mut egui::Ui, entry: &Entry, icon: Option<(f32, f32)>| {
                    let glyph = if entry.kind == 'd' { "▣" } else { "▤" };
                    let response = if let Some((width, scale)) = icon {
                        let label =
                            egui::RichText::new(format!("{glyph}\n{}", entry.name)).size(scale);
                        ui.add_enabled(
                            enabled,
                            egui::Button::new(label)
                                .wrap()
                                .sense(egui::Sense::click_and_drag())
                                .min_size(egui::vec2(width, scale * 3.1)),
                        )
                    } else {
                        ui.add_enabled(
                            enabled,
                            egui::Button::new(format!("{glyph} {}", entry.name)).frame(false),
                        )
                    };
                    if enabled
                        && entry.kind == 'd'
                        && (response.double_clicked() || (icon.is_none() && response.clicked()))
                    {
                        action = Some(Action::Open(join(path, &entry.name)));
                    }
                    if enabled && writable && icon.is_some() {
                        response.dnd_set_drag_payload(Drag {
                            token: token.into(),
                            directory: path.clone(),
                            name: entry.name.clone(),
                        });
                        if entry.kind == 'd'
                            && let Some(source) = response.dnd_release_payload::<Drag>()
                            && source.token == token
                            && (source.directory != *path || source.name != entry.name)
                        {
                            action = Some(Action::Work(Work::Rename {
                                directory: source.directory.clone(),
                                name: source.name.clone(),
                                to_directory: join(path, &entry.name),
                                to_name: source.name.clone(),
                            }));
                        }
                    }
                    if enabled {
                        response.context_menu(|ui| {
                            if writable && entry.kind == 'd' {
                                self.shares.context_menu(
                                    ui,
                                    &join(path, &entry.name),
                                    &entry.name,
                                    agent_name,
                                );
                                ui.separator();
                            }
                            if ui.button("Copy full path").clicked() {
                                ui.ctx().copy_text(address(
                                    connection.host.trim(),
                                    token,
                                    &join(path, &entry.name),
                                ));
                                ui.close();
                            }
                            for (allowed, label, next) in [
                                (
                                    writable,
                                    "Rename",
                                    Action::Prompt(Prompt::Rename(
                                        entry.clone(),
                                        entry.name.clone(),
                                    )),
                                ),
                                (
                                    writable && entry.kind == 'f',
                                    "Edit in Vim",
                                    Action::Edit(entry.clone()),
                                ),
                                (
                                    writable,
                                    "Delete…",
                                    Action::Prompt(Prompt::Delete(entry.clone())),
                                ),
                                (
                                    writable && entry.kind == 'f',
                                    "Download…",
                                    Action::Download(entry.clone()),
                                ),
                            ] {
                                if ui.add_enabled(allowed, egui::Button::new(label)).clicked() {
                                    action = Some(next);
                                    ui.close();
                                }
                            }
                        });
                    }
                };
                match self.layout {
                    Layout::Large | Layout::Medium | Layout::Small => {
                        let icon = match self.layout {
                            Layout::Large => (126.0, 20.0),
                            Layout::Medium => (100.0, 16.0),
                            _ => (82.0, 12.0),
                        };
                        ui.horizontal_wrapped(|ui| {
                            for entry in &self.entries {
                                item(ui, entry, Some(icon));
                            }
                        });
                    }
                    Layout::List => {
                        for entry in &self.entries {
                            ui.horizontal(|ui| {
                                item(ui, entry, None);
                                ui.label(format!(
                                    "Created {}  ·  Modified {}  ·  {}  ·  {}",
                                    entry.created,
                                    entry.modified,
                                    entry.kind(),
                                    entry.size()
                                ));
                            });
                        }
                    }
                    Layout::Details => {
                        egui::Grid::new("file details")
                            .striped(true)
                            .show(ui, |ui| {
                                for label in [
                                    "Name",
                                    "Date created (UTC)",
                                    "Date modified (UTC)",
                                    "Type",
                                    "Size",
                                ] {
                                    ui.strong(label);
                                }
                                ui.end_row();
                                for entry in &self.entries {
                                    item(ui, entry, None);
                                    for label in [
                                        entry.created.as_str(),
                                        entry.modified.as_str(),
                                        entry.kind(),
                                        &entry.size(),
                                    ] {
                                        ui.label(label);
                                    }
                                    ui.end_row();
                                }
                            });
                    }
                }
            });
        if enabled
            && accepts_drop
            && pane.contains(ui.ctx().pointer_latest_pos().unwrap_or(pane.center()))
        {
            let files: Vec<PathBuf> = ui.input(|i| {
                i.raw
                    .dropped_files
                    .iter()
                    .map(|f| f.path().to_path_buf())
                    .collect()
            });
            if !files.is_empty() {
                if writable {
                    action = Some(Action::Work(Work::Upload {
                        directory: self.path.clone(),
                        files,
                    }));
                } else {
                    self.status =
                        Some((false, "Open a folder under /agent/home to upload.".into()));
                }
            }
        }
        if let Some(action) = action {
            activity = true;
            let directory = self.path.clone();
            let work = match action {
                Action::Open(path) => {
                    self.open(path);
                    None
                }
                Action::Prompt(prompt) => {
                    self.prompt = Some(prompt);
                    None
                }
                Action::Work(work) => Some(work),
                Action::Download(entry) => match transfer::save_as(&entry.name) {
                    Ok(Some(destination)) => Some(Work::Download {
                        directory,
                        name: entry.name,
                        destination,
                    }),
                    Ok(None) => None,
                    Err(error) => {
                        self.status = Some((false, error.to_string()));
                        None
                    }
                },
                Action::Edit(entry) => {
                    editor = Some(FileEditor {
                        directory,
                        name: entry.name,
                    });
                    None
                }
            };
            if let Some(work) = work {
                if let Work::Create {
                    directory,
                    name,
                    folder: false,
                } = &work
                {
                    self.open_after_create = Some(FileEditor {
                        directory: directory.clone(),
                        name: name.clone(),
                    });
                }
                self.status = None;
                self.job = Some(transfer::start(ui.ctx(), connection, token, work));
            }
        }
        if self.refresh && self.listing.is_none() && self.job.is_none() {
            self.refresh = false;
            let ctx = ui.ctx().clone();
            let command = format!(
                "agent-files {token} {} {}{}",
                saved::encode(&self.path),
                self.offset,
                if self.descending { " desc" } else { "" }
            );
            match saved::run(
                connection.host.trim(),
                connection.key.as_deref(),
                &command,
                move || ctx.request_repaint(),
            ) {
                Ok(job) => self.listing = Some(job),
                Err(error) => self.status = Some((false, error.to_string())),
            }
        }
        (activity, editor)
    }
}

fn writable(path: &str) -> bool {
    path == HOME
        || path
            .strip_prefix(HOME)
            .is_some_and(|rest| rest.starts_with('/'))
}
fn valid_name(name: &str) -> bool {
    !matches!(name, "" | "." | "..") && name.len() <= 255 && !name.contains(['/', '\0'])
}

fn portable_name(name: &str) -> bool {
    if !valid_name(name)
        || name.trim().is_empty()
        || name != name.trim()
        || name.ends_with('.')
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"\\|?*".contains(c))
    {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap()
        .trim_end()
        .to_ascii_uppercase();
    !matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) && !["COM", "LPT"].iter().any(|p| {
        stem.strip_prefix(p).is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}
fn join(path: &str, name: &str) -> String {
    format!("{}/{name}", path.trim_end_matches('/'))
}

fn address(host: &str, token: &str, path: &str) -> String {
    let mut url = url::Url::parse("rectangle://files/").unwrap();
    url.path_segments_mut()
        .unwrap()
        .clear()
        .push(host)
        .push(token)
        .extend(path.split('/').filter(|part| !part.is_empty()));
    url.into()
}
fn decode_segment(value: &str) -> io::Result<String> {
    let invalid = || io::Error::other("Invalid escape in Agent folder URL.");
    let mut input = value.bytes();
    let mut output = Vec::new();
    while let Some(byte) = input.next() {
        output.push(if byte == b'%' {
            let hex = |b: Option<u8>| {
                b.and_then(|b| (b as char).to_digit(16))
                    .map(|b| b as u8)
                    .ok_or_else(invalid)
            };
            hex(input.next())? * 16 + hex(input.next())?
        } else {
            byte
        });
    }
    String::from_utf8(output).map_err(|_| invalid())
}
fn parse_listing(data: &str) -> io::Result<(usize, Vec<Entry>)> {
    let invalid = || io::Error::other("Invalid File Manager directory listing.");
    let mut lines = data.lines();
    let (version, total) = lines
        .next()
        .and_then(|line| line.split_once(' '))
        .ok_or_else(invalid)?;
    if !matches!(version, "RECTFILES1" | "RECTFILES2") {
        return Err(invalid());
    }
    let total = total.parse().map_err(|_| invalid())?;
    let mut entries = Vec::new();
    for line in lines {
        let mut fields = line.split_ascii_whitespace();
        let kind = match fields.next() {
            Some("d") => 'd',
            Some("f") => 'f',
            Some("o") => 'o',
            _ => return Err(invalid()),
        };
        let bytes = fields
            .next()
            .and_then(|field| field.parse().ok())
            .ok_or_else(invalid)?;
        let name = fields
            .next()
            .and_then(|field| saved::decode(field, 510))
            .ok_or_else(invalid)?;
        if !valid_name(&name) || entries.len() >= PAGE {
            return Err(invalid());
        }
        let mut date = || {
            fields
                .next()
                .and_then(|field| saved::decode(field, 64))
                .unwrap_or_else(|| "—".into())
        };
        let (created, modified) = (date(), date());
        entries.push(Entry {
            kind,
            bytes,
            name,
            created,
            modified,
        });
    }
    Ok((total, entries))
}

#[test]
fn new_name_rejects_reserved_and_nonportable_names() {
    for invalid in [
        "",
        "  ",
        "folder/name",
        "folder\\name",
        "bad?",
        "file.",
        " aux.txt",
        "com¹.log",
        "LPT9",
    ] {
        assert!(!portable_name(invalid), "{invalid:?}");
    }
    assert!(portable_name("Project notes.md"));
    assert!(portable_name(".gitignore"));
}

#[test]
fn folder_address_round_trips_and_cannot_switch_agents() {
    let mut explorer = Explorer::default();
    let host = "alice@host.example";
    let input = "/agent/home/A & B?#/日本語";
    let uri = address(host, "abcd", input);
    explorer.navigate(host, "abcd", &uri).unwrap();
    assert_eq!(explorer.path, input);
    assert!(explorer.navigate(host, "efgh", &uri).is_err());
    explorer.history(true);
    assert_eq!(explorer.path, "/agent/home/workspace");
    explorer.history(false);
    assert_eq!(explorer.path, input);
}
