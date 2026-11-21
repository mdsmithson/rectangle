#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod audio;
mod clipboard;
mod design;
mod files;
mod images;
mod mark;
mod memory;
mod navigation;
mod preview;
mod proton;
mod rfb;
mod saved;
mod session;
mod settings;
mod terminal;
mod tiling;
mod update;
use tiling::Snap;

use eframe::egui::{self, Color32, Event, Key, PointerButton, Pos2, Rect, RichText, Sense, Vec2};
use session::Session;
use std::time::{Duration, Instant};

const INK: Color32 = Color32::from_rgb(237, 239, 245);
const MUTED: Color32 = Color32::from_rgb(158, 168, 182);
const MINT: Color32 = Color32::from_rgb(108, 225, 186);
const MAX_TABS: usize = 6;
const IDLE_LIMIT: Duration = Duration::from_secs(30 * 60);
const IDLE_MESSAGE: &str = "Container stopped after 30 minutes of inactivity.";

#[derive(Default)]
struct Rectangle {
    agent_images: Vec<images::AgentImage>,
    dragging_tab: Option<u64>,
    parked: Vec<saved::Parked>,
    startup_parked: Vec<(Backend, std::sync::mpsc::Receiver<std::io::Result<String>>)>,
    startup_entries: Vec<(Backend, saved::Parked)>,
    startup_errors: Vec<(Backend, String)>,
    secondary: Connection,
    secondary_entries: Vec<saved::Entry>,
    library_loaded: Option<Backend>,
    library_host: Backend,
    new_tab_address: String,
    browser_address: String,
    address_tab: Option<u64>,
    address_rect: Option<Rect>,
    address_headers: Vec<(u64, Rect)>,
    terminal_directory: Option<(u64, Option<terminal::WorkingDirectory>)>,
    proton: proton::Proton,
    primary_memory: memory::HostMeter,
    primary_disk: memory::HostMeter,
    marble: Option<egui::TextureHandle>,
    host: String,
    identity: Option<std::path::PathBuf>,
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    error: Option<String>,
    home: bool,
    startup_page: String,
    website_theme: settings::WebsiteTheme,
    settings_edit: Option<SettingsEdit>,
    entries: Vec<saved::Entry>,
    job: Option<(Job, std::sync::mpsc::Receiver<std::io::Result<String>>)>,
    dialog: Option<Dialog>,
    quitting: bool,
    updater: update::Updater,
    pending_update: Option<std::path::PathBuf>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Backend {
    #[default]
    Primary,
    Secondary,
}
impl Backend {
    fn name(self, host: &str) -> &str {
        match self {
            Self::Primary => host
                .trim()
                .split_once('@')
                .filter(|(user, server)| !user.is_empty() && !server.is_empty())
                .map_or("Primary", |(user, _)| user),
            Self::Secondary => "Secondary",
        }
    }
}
#[derive(Default, Clone)]
struct Connection {
    host: String,
    key: Option<std::path::PathBuf>,
}
struct SettingsEdit {
    host: String,
    key: String,
    secondary: String,
    secondary_key: String,
    startup_page: String,
    website_theme: settings::WebsiteTheme,
    section: SettingsSection,
    error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsSection {
    Updates,
    Startup,
    Connections,
    Proton,
}

enum TabAction {
    Select,
    Close,
    Rename,
    Clone,
    Snap(Option<Snap>),
    Park,
    Mute,
    Volume(u8),
    Navigate(&'static str),
    ToggleVideoMode,
    AgentResolution(AgentResolution),
    ToggleAlwaysOn,
    AgentMemory,
    OpenAgentView(AgentView),
    Preview,
    Info(AgentInfo),
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum AgentView {
    #[default]
    Freebuff,
    Codex,
    Files,
    Shell,
}

impl AgentView {
    const ALL: [Self; 3] = [Self::Files, Self::Shell, Self::Freebuff];

    fn command(self) -> &'static str {
        match self {
            Self::Freebuff => "freebuff",
            Self::Codex => "codex",
            Self::Files => "files",
            Self::Shell => "sh",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Freebuff => "Freebuff Terminal",
            Self::Codex => "ChatGPT Codex",
            Self::Files => "File Manager",
            Self::Shell => "SH Terminal",
        }
    }
}

impl Tab {
    fn terminal_view(&self) -> Option<String> {
        self.agent_port?;
        match (self.agent_view, self.agent_owner, self.file_editor) {
            (AgentView::Files, _, _) => None,
            (_, _, Some(display)) => Some(format!("edit{display}")),
            (_, None, _) => Some("main".into()),
            (view, Some(_), _) => Some(view.command().into()),
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum AgentResolution {
    #[default]
    MatchTile,
    MatchHost,
    Uhd,
    Qhd,
    FullHd,
    Dci2k,
    OneK,
}

impl AgentResolution {
    const OPTIONS: [(Self, &str); 7] = [
        (Self::Uhd, "4K UHD (3840 × 2160)"),
        (Self::Qhd, "QHD (2560 × 1440)"),
        (Self::FullHd, "Full HD (1920 × 1080)"),
        (Self::Dci2k, "2K (2048 × 1080)"),
        (Self::OneK, "1K (1024 × 576)"),
        (Self::MatchTile, "Match Tile"),
        (Self::MatchHost, "Match Host"),
    ];

    fn target(self, tile: rfb::Resolution, host: Option<egui::Vec2>) -> rfb::Resolution {
        match self {
            Self::MatchTile => tile,
            Self::MatchHost => host.map_or(tile, |size| rfb::Resolution::fit(size.x, size.y)),
            Self::Uhd => rfb::Resolution(3840, 2160),
            Self::Qhd => rfb::Resolution(2560, 1440),
            Self::FullHd => rfb::Resolution(1920, 1080),
            Self::Dci2k => rfb::Resolution(2048, 1080),
            Self::OneK => rfb::Resolution(1024, 576),
        }
    }
}

enum Job {
    AgentInfo(AgentInfo, String),
    AgentMemory(u64),
    SetAgentMemory(AgentMemoryDraft),
    OpenPreview(u64),
    BuildAgent(Backend, String, String, u16, Option<Snap>),
    CloneTab(Backend),
    List(Backend),
    ParkedList(Backend),
    OpenSaved(Backend, saved::Entry, Option<Snap>),
    PrepareAgentMedia(Backend, saved::Entry, Option<Snap>),
    Park(u64, Backend),
    StopParked(Backend),
    Close(u64, Backend),
    IdleSave(u64, Backend),
    AlwaysOn(u64, bool),
    Forget(Backend),
    StartupPage(Backend, saved::Entry),
}
enum Dialog {
    AgentInfo(AgentInfo, String, String),
    AgentMemory(AgentMemoryDraft),
    Rename(u64, String),
    Clone(u64, String),
    Close(u64, String),
    Forget(Backend, saved::Entry),
    StopParked(Backend, saved::Parked),
    StartupPage(Backend, saved::Entry, String, Option<String>),
    NewTab(NewTab),
}

#[derive(Clone, Copy)]
struct AgentMemory {
    usage: u64,
    limit: u64,
    host_total: u64,
}

impl AgentMemory {
    const MIB: u64 = 1_048_576;

    fn parse(data: &str) -> std::io::Result<Self> {
        let invalid = || std::io::Error::other("Unexpected Agent memory data from the host.");
        let mut fields = data.split_ascii_whitespace();
        if fields.next() != Some("RECTAGENTMEM1") {
            return Err(invalid());
        }
        let mut number = || {
            fields
                .next()
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or_else(invalid)
        };
        let (usage, limit, host_total) = (number()?, number()?, number()?);
        if limit == 0 || host_total == 0 || fields.next().is_some() {
            return Err(invalid());
        }
        Ok(Self {
            usage,
            limit,
            host_total,
        })
    }

    fn check_limit(self, mib: u64) -> Result<(), &'static str> {
        if !(256..=1_048_576).contains(&mib) {
            return Err("Enter a whole number from 256 to 1,048,576 MiB.");
        }
        let bytes = mib * Self::MIB;
        if bytes < self.limit && bytes <= self.usage {
            return Err("Set a limit above the Agent’s current memory usage.");
        }
        if bytes > self.limit && bytes > self.host_total {
            return Err("The limit cannot exceed the host’s total RAM.");
        }
        Ok(())
    }
}

struct AgentMemoryDraft {
    tab: u64,
    data: AgentMemory,
    mib: u64,
    message: Option<(String, bool)>,
}

#[derive(Clone, Copy)]
enum AgentInfo {
    Ports,
    Health,
}

impl AgentInfo {
    fn label(self) -> &'static str {
        match self {
            Self::Ports => "Open ports",
            Self::Health => "Container health",
        }
    }

    fn command(self) -> &'static str {
        match self {
            Self::Ports => "agent-ports",
            Self::Health => "agent-health",
        }
    }
}

#[derive(Default)]
struct NewTab {
    slot: Option<Snap>,
    backend: Backend,
    image: usize,
    codex: bool,
    name: String,
    port: String,
    port_host: Option<Backend>,
    port_lookup: Option<std::sync::mpsc::Receiver<std::io::Result<String>>>,
    port_error: Option<String>,
    dockerfile: Option<String>,
    copy_name: String,
    error: Option<String>,
}

#[derive(Default)]
struct Tab {
    navigation: Option<navigation::Navigation>,
    page_rect: Option<Rect>,
    preview: Option<preview::Preview>,
    agent_port: Option<u16>,
    codex: bool,
    agent_view: AgentView,
    agent_owner: Option<u64>,
    agent_parent_title: Option<String>,
    file_editor: Option<u16>,
    explorer: files::Explorer,
    resize_target: Option<(rfb::Resolution, Instant)>,
    resize_sent: Option<rfb::Resolution>,
    snap: Option<Snap>,
    audio: Option<audio::Audio>,
    audio_error: Option<String>,
    muted: bool,
    volume: Option<u8>,
    video_mode: bool,
    agent_resolution: AgentResolution,
    always_on: bool,
    backend: Backend,
    connection: Connection,
    id: u64,
    title: String,
    profile: Option<String>,
    session: Option<Session>,
    texture: Option<egui::TextureHandle>,
    size: [u16; 2],
    error: Option<String>,
    started: Option<Instant>,
    last_input: Option<Instant>,
    ran: bool,
    idle_stopped: bool,
    cursor: Option<Pos2>,
    buttons: u8,
    captured: bool,
    clipboard: Option<clipboard::Clipboard>,
    clipboard_error: Option<&'static str>,
    copy_until: Option<Instant>,
    paste_pending: bool,
    paste_canceled: bool,
    memory: Option<memory::Meter>,
    memory_failed: bool,
}

fn main() -> eframe::Result {
    let host = std::env::args().nth(1).unwrap_or_default();
    let identity = std::env::args_os().nth(2).map(std::path::PathBuf::from);
    let (host, identity, error) = match settings::load() {
        Ok((host, identity)) => (host, identity, None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (host, identity, None),
        Err(error) => (
            host,
            identity,
            Some(format!("Could not load SSH settings: {error}")),
        ),
    };
    let secondary = settings::load_secondary()
        .map(|(host, key)| Connection { host, key })
        .unwrap_or_default();
    let (startup_page, error) = match settings::load_startup_page() {
        Ok(page) => (page, error),
        Err(err) => (
            String::new(),
            error.or_else(|| Some(format!("Could not load the startup page: {err}"))),
        ),
    };
    let (website_theme, error) = match settings::load_website_theme() {
        Ok(theme) => (theme, error),
        Err(err) => (
            settings::WebsiteTheme::Default,
            error.or_else(|| Some(format!("Could not load the website theme: {err}"))),
        ),
    };
    let (agent_images, error) = match settings::agent_images() {
        Ok(images) => (images, error),
        Err(err) => (
            Vec::new(),
            error.or_else(|| Some(format!("Could not load Agent images: {err}"))),
        ),
    };
    let options = eframe::NativeOptions {
        persistence_path: settings::window_path().ok(),
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 790.0])
            .with_min_inner_size([640.0, 420.0])
            .with_icon(egui::IconData {
                rgba: mark::icon(64),
                width: 64,
                height: 64,
            }),
        ..Default::default()
    };
    eframe::run_native(
        "Rectangle",
        options,
        Box::new(move |cc| {
            design::theme(&cc.egui_ctx);
            let mut app = Rectangle {
                marble: Some(design::marble(&cc.egui_ctx)),
                host,
                identity,
                startup_page,
                website_theme,
                agent_images,
                secondary,
                home: true,
                error,
                ..Default::default()
            };
            app.find_parked_on_startup(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}

impl Tab {
    fn stop_for_idle(&mut self) {
        self.close();
        self.idle_stopped = true;
        self.error = Some(IDLE_MESSAGE.into());
    }

    fn close(&mut self) {
        self.navigation = None;
        self.page_rect = None;
        self.preview = None;
        self.resize_target = None;
        self.resize_sent = None;
        self.audio = None;
        self.clipboard = None;
        self.memory = None;
        self.session = None;
        self.texture = None;
        self.started = None;
        self.last_input = None;
        self.buttons = 0;
        self.captured = false;
        self.cursor = None;
    }

    fn blur(&mut self) {
        self.clipboard = None;
        self.copy_until = None;
        self.paste_pending = false;
        if self.buttons != 0
            && let Some(session) = &self.session
        {
            let _ = session.send(rfb::pointer(0, 0, 0), false);
        }
        self.buttons = 0;
        self.captured = false;
        self.cursor = None;
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(session) = &self.session else { return };
        let (frame, error) = {
            let mut view = session.view.lock().unwrap();
            (view.frame.take(), view.error.take())
        };
        if let Some(error) = error {
            self.close();
            self.error = Some(error);
            return;
        }
        if let Some(frame) = frame {
            if !self.ran {
                self.ran = true;
                self.last_input = Some(Instant::now());
            }
            if self.agent_port.is_none() && self.navigation.is_none() {
                let repaint = ctx.clone();
                self.navigation = navigation::Navigation::start(
                    self.connection.host.trim(),
                    self.connection.key.as_deref(),
                    &session.token,
                    move || repaint.request_repaint(),
                )
                .ok();
            }
            let size = [frame.width, frame.height];
            let width = usize::from(frame.width);
            if let Some(texture) = &mut self.texture
                && self.size == size
                && let Some(dirty) = frame.dirty
            {
                let w = dirty.right - dirty.x;
                let h = dirty.bottom - dirty.y;
                let mut pixels = Vec::with_capacity(w * h);
                for y in dirty.y..dirty.bottom {
                    let start = (y * width + dirty.x) * 4;
                    pixels.extend(
                        frame.rgba[start..start + w * 4]
                            .chunks_exact(4)
                            .map(|px| Color32::from_rgba_unmultiplied(px[0], px[1], px[2], px[3])),
                    );
                }
                texture.set_partial(
                    [dirty.x, dirty.y],
                    egui::ColorImage::new([w, h], pixels),
                    egui::TextureOptions::LINEAR,
                );
            } else {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [width, usize::from(frame.height)],
                    &frame.rgba,
                );
                if let Some(texture) = &mut self.texture {
                    texture.set(image, egui::TextureOptions::LINEAR);
                } else {
                    self.texture = Some(ctx.load_texture(
                        "remote display",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
            self.size = size;
        }
        if self.texture.is_some() && self.memory.is_none() && !self.memory_failed {
            let repaint = ctx.clone();
            match memory::Meter::start(
                self.connection.host.trim(),
                self.connection.key.as_deref(),
                &session.token,
                move || repaint.request_repaint(),
            ) {
                Ok(meter) => self.memory = Some(meter),
                Err(_) => self.memory_failed = true,
            }
        }
        if self.texture.is_none() {
            if self
                .started
                .is_some_and(|t| t.elapsed() > Duration::from_secs(30))
            {
                self.close();
                self.error =
                    Some("The remote browser did not send a display within 30 seconds.".into());
            } else {
                ctx.request_repaint_after(Duration::from_millis(200));
            }
        }
    }

    fn pointer(&self, pos: Pos2, rect: Rect, buttons: u8, droppable: bool) -> bool {
        let fraction = (pos - rect.min) / rect.size();
        let x =
            (fraction.x * f32::from(self.size[0])).clamp(0.0, f32::from(self.size[0] - 1)) as u16;
        let y =
            (fraction.y * f32::from(self.size[1])).clamp(0.0, f32::from(self.size[1] - 1)) as u16;
        self.session
            .as_ref()
            .unwrap()
            .send(rfb::pointer(buttons, x, y), droppable)
    }

    fn input(&mut self, rect: Rect, events: Vec<Event>) -> bool {
        for event in events {
            let activity = match &event {
                Event::PointerMoved(pos) => rect.contains(*pos),
                Event::PointerButton { pos, pressed, .. } => *pressed && rect.contains(*pos),
                Event::MouseWheel { delta, .. } => {
                    *delta != Vec2::ZERO && self.cursor.is_some_and(|pos| rect.contains(pos))
                }
                Event::Text(_)
                | Event::Key { pressed: true, .. }
                | Event::Copy
                | Event::Cut
                | Event::Paste(_) => self.captured,
                _ => false,
            };
            if activity {
                self.last_input = Some(Instant::now());
            }
            let ok = match event {
                Event::PointerMoved(pos) => {
                    self.cursor = Some(pos);
                    !rect.contains(pos) && self.buttons == 0
                        || self.pointer(pos, rect, self.buttons, true)
                }
                Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } => {
                    if pressed {
                        self.captured = rect.contains(pos);
                        self.paste_canceled |= !self.captured;
                    }
                    let bit = match button {
                        PointerButton::Primary => 1,
                        PointerButton::Middle => 2,
                        PointerButton::Secondary => 4,
                        _ => continue,
                    };
                    if pressed && rect.contains(pos) {
                        self.buttons |= bit;
                        self.pointer(pos, rect, self.buttons, false)
                    } else if !pressed && self.buttons & bit != 0 {
                        self.buttons &= !bit;
                        self.pointer(pos, rect, self.buttons, false)
                    } else {
                        true
                    }
                }
                Event::WindowFocused(false) => {
                    self.paste_canceled = true;
                    self.buttons = 0;
                    self.cursor
                        .is_none_or(|pos| self.pointer(pos, rect, 0, false))
                }
                Event::MouseWheel { delta, .. } => {
                    if let Some(pos) = self.cursor.filter(|p| rect.contains(*p)) {
                        if delta.y != 0.0 {
                            let bit = if delta.y > 0.0 { 8 } else { 16 };
                            self.pointer(pos, rect, self.buttons | bit, false)
                                && self.pointer(pos, rect, self.buttons, false)
                        } else {
                            true
                        }
                    } else {
                        true
                    }
                }
                Event::Text(text) if self.captured => {
                    let mut packet = Vec::new();
                    for c in text.chars().filter(|c| !c.is_control()).take(256) {
                        let c = c as u32;
                        let symbol = if c <= 0xff { c } else { c | 0x0100_0000 };
                        packet.extend(rfb::key(symbol, true));
                        packet.extend(rfb::key(symbol, false));
                    }
                    packet.is_empty() || self.session.as_ref().unwrap().send(packet, false)
                }
                Event::Copy | Event::Cut if self.captured => {
                    self.copy_until = Some(Instant::now() + Duration::from_secs(2));
                    self.shortcut(if matches!(event, Event::Cut) {
                        b'x'
                    } else {
                        b'c'
                    })
                }
                Event::Paste(text) if self.captured => {
                    if !self.paste_pending {
                        if text.len() > clipboard::LIMIT {
                            self.clipboard_error = Some("Clipboard text is over 1 MiB.");
                        } else if self.clipboard.as_ref().is_some_and(|clip| clip.paste(text)) {
                            self.paste_pending = true;
                            self.paste_canceled = false;
                            self.clipboard_error = None;
                        } else {
                            self.clipboard_error = Some("Clipboard paste is unavailable.");
                        }
                    }
                    true
                }
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if self.captured => {
                    let modified = modifiers.ctrl || modifiers.alt;
                    let Some(symbol) = keysym(key, modified) else {
                        continue;
                    };
                    let mods = [
                        (modifiers.ctrl, 0xffe3),
                        (modifiers.alt, 0xffe9),
                        (modifiers.shift, 0xffe1),
                    ];
                    let mut packet = Vec::new();
                    for (on, m) in mods {
                        if on {
                            packet.extend(rfb::key(m, true));
                        }
                    }
                    packet.extend(rfb::key(symbol, true));
                    packet.extend(rfb::key(symbol, false));
                    for (on, m) in mods.into_iter().rev() {
                        if on {
                            packet.extend(rfb::key(m, false));
                        }
                    }
                    self.session.as_ref().unwrap().send(packet, false)
                }
                _ => true,
            };
            if !ok {
                return false;
            }
        }
        true
    }

    fn shortcut(&self, key: u8) -> bool {
        let symbol = u32::from(key);
        self.session.as_ref().unwrap().send(
            [
                rfb::key(0xffe3, true),
                rfb::key(symbol, true),
                rfb::key(symbol, false),
                rfb::key(0xffe3, false),
            ]
            .concat(),
            false,
        )
    }

    fn poll_clipboard(&mut self, ctx: &egui::Context, focused: bool) -> bool {
        let Some(clip) = &self.clipboard else {
            return true;
        };
        let update = clip.poll();
        if let Some(text) = update.text
            && ((focused && self.captured)
                || self.copy_until.is_some_and(|until| Instant::now() < until))
        {
            ctx.copy_text(text);
            self.copy_until = None;
        }
        if update.disconnected {
            self.clipboard = None;
            self.paste_pending = false;
            self.clipboard_error =
                Some("Clipboard connection ended. Switch tabs or reconnect to retry.");
            return true;
        }
        if let Some(ready) = update.paste_ready
            && std::mem::take(&mut self.paste_pending)
            && !self.paste_canceled
            && self.captured
            && focused
        {
            if ready {
                return self.shortcut(b'v');
            }
            self.clipboard_error = Some("Could not set the remote clipboard.");
        }
        true
    }

    fn memory_badge(&self, ctx: &egui::Context) -> (Option<(f32, Color32)>, String) {
        let now = Instant::now();
        let status = self.memory.as_ref().map(memory::Meter::status);
        let Some(memory::Status::Live(reading)) = status
            .filter(|status| matches!(status, memory::Status::Live(reading) if now.saturating_duration_since(reading.received) < Duration::from_secs(4)))
        else {
            let label = if self.session.is_some() && !self.memory_failed
                && matches!(status, None | Some(memory::Status::Connecting)) {
                "Tab RAM: measuring…"
            } else {
                "Tab RAM: unavailable"
            };
            return (None, label.into());
        };
        ctx.request_repaint_after(Duration::from_secs(1));
        let (status, color) = match reading.level(now) {
            memory::Level::Normal => ("", MINT),
            memory::Level::NearLimit => (" · Near limit", Color32::GOLD),
            memory::Level::Pressure => (" · Memory pressure", Color32::GOLD),
            memory::Level::Reclaim => (" · At limit: reclaiming", Color32::LIGHT_RED),
            memory::Level::Throttling => (" · Memory throttling", Color32::LIGHT_RED),
            memory::Level::Killed => (" · Process killed: out of memory", Color32::LIGHT_RED),
        };
        let sample = reading.sample;
        let gib = (1u64 << 30) as f64;
        (
            Some((reading.ratio().clamp(0.0, 1.0), color)),
            format!(
                "Tab RAM: {:.2} / {:.2} GiB ({:.0}%){status}\nMemory stalls (10 s): {}\nThrottling events: {} · Hard-limit events: {} · OOM kills: {}",
                sample.used as f64 / gib,
                sample.limit as f64 / gib,
                reading.ratio() * 100.0,
                sample
                    .stall
                    .map_or_else(|| "unavailable".into(), |stall| format!("{stall:.2}%")),
                sample.high,
                sample.max,
                sample.oom_kills
            ),
        )
    }
}

impl Rectangle {
    fn snap_tab(&mut self, id: u64, snap: Option<Snap>) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        if let Some(slot) = snap {
            for tab in &mut self.tabs {
                if tab.id != id && tab.snap.is_some_and(|other| slot.overlaps(other)) {
                    tab.snap = None;
                }
            }
        }
        self.tabs[index].snap = snap;
        self.select_tab(index);
    }

    fn place_new_tab(&mut self, count: usize, slot: Option<Snap>) {
        if self.tabs.len() > count
            && let Some(slot) = slot
        {
            let id = self.tabs.last().unwrap().id;
            self.place_in_vacant_tile(id, slot);
        }
    }

    fn place_in_vacant_tile(&mut self, id: u64, slot: Snap) {
        if self
            .tabs
            .iter()
            .all(|tab| tab.id == id || !tab.snap.is_some_and(|existing| existing.overlaps(slot)))
        {
            self.snap_tab(id, Some(slot));
        }
    }

    fn find_parked_on_startup(&mut self, ctx: &egui::Context) {
        for backend in [Backend::Primary, Backend::Secondary] {
            let connection = self.connection(backend);
            let host = connection.host.trim();
            if host.is_empty() {
                continue;
            }
            let repaint = ctx.clone();
            match saved::run(host, connection.key.as_deref(), "parked-list", move || {
                repaint.request_repaint()
            }) {
                Ok(task) => self.startup_parked.push((backend, task)),
                Err(err) => self.startup_errors.push((backend, err.to_string())),
            }
        }
        if self.startup_parked.is_empty() {
            self.finish_startup(ctx);
        }
    }

    fn poll_startup(&mut self, ctx: &egui::Context) {
        use std::sync::mpsc::TryRecvError;
        if self.startup_parked.is_empty() {
            return;
        }
        self.startup_parked.retain(|(backend, task)| {
            match task.try_recv() {
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => self
                    .startup_errors
                    .push((*backend, "parked sessions request disconnected".into())),
                Ok(result) => match result.and_then(|data| saved::parse_parked(&data)) {
                    Ok(entries) => self
                        .startup_entries
                        .extend(entries.into_iter().map(|entry| (*backend, entry))),
                    Err(err) => self.startup_errors.push((*backend, err.to_string())),
                },
            }
            false
        });
        if self.startup_parked.is_empty() {
            self.finish_startup(ctx);
        }
    }

    fn finish_startup(&mut self, ctx: &egui::Context) {
        let startup_errors = std::mem::take(&mut self.startup_errors);
        let mut errors: Vec<_> = startup_errors
            .iter()
            .map(|(backend, err)| format!("{}: {err}", backend.name(&self.host)))
            .collect();
        errors.extend(self.error.take());
        let mut entries = std::mem::take(&mut self.startup_entries);
        entries
            .sort_by_key(|(backend, entry)| (*backend as u8, entry.expires, entry.token.clone()));
        for (backend, entry) in entries {
            // These containers are already running. The new-tab limit shouldn't leave any parked.
            self.resume_parked(ctx, backend, entry);
            errors.extend(self.error.take());
        }
        if let Some(index) = self.tabs.iter().position(|tab| tab.snap.is_some()) {
            self.select_tab(index);
        }
        if self.tabs.is_empty() {
            let backend = self.library_host;
            if !startup_errors
                .iter()
                .any(|(failed_backend, _)| *failed_backend == backend)
            {
                self.refresh(ctx);
                errors.extend(self.error.take());
            }
        }
        if !errors.is_empty() {
            self.error = Some(errors.join("\n"));
        }
    }

    fn resume_parked(&mut self, ctx: &egui::Context, backend: Backend, entry: saved::Parked) {
        let connection = self.connection(backend);
        let volume = settings::tab_volume(
            connection.host.trim(),
            entry.profile.as_deref().unwrap_or(&entry.token),
        );
        let repaint = ctx.clone();
        match Session::resume(
            connection.host.trim(),
            connection.key.as_deref(),
            &entry.token,
            move || repaint.request_repaint(),
        ) {
            Ok(session) => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.blur();
                }
                self.next_id += 1;
                self.tabs.push(Tab {
                    id: self.next_id,
                    backend,
                    connection,
                    title: entry.title,
                    volume,
                    profile: entry.profile,
                    agent_port: entry.agent_port,
                    codex: entry.codex,
                    agent_resolution: if entry.codex {
                        AgentResolution::Qhd
                    } else {
                        AgentResolution::MatchTile
                    },
                    agent_view: if entry.file_manager {
                        AgentView::Files
                    } else if entry.codex {
                        AgentView::Codex
                    } else {
                        AgentView::Freebuff
                    },
                    always_on: entry.always_on,
                    snap: entry.snap.filter(|slot| {
                        !self
                            .tabs
                            .iter()
                            .any(|tab| tab.snap.is_some_and(|existing| existing.overlaps(*slot)))
                    }),
                    session: Some(session),
                    started: Some(Instant::now()),
                    ..Default::default()
                });
                self.active = self.tabs.len() - 1;
                self.home = false;
                self.parked.retain(|parked| parked.token != entry.token);
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn expire_idle_tabs(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let linked_activity: Vec<_> = self
            .tabs
            .iter()
            .filter_map(|tab| {
                tab.agent_owner.map(|owner| {
                    (
                        owner,
                        if tab.explorer.uploading() {
                            Some(now)
                        } else {
                            tab.last_input
                        },
                    )
                })
            })
            .collect();
        for (owner, input) in linked_activity {
            if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == owner) {
                tab.last_input = tab.last_input.max(input);
            }
        }
        let mut save = None;
        for tab in &mut self.tabs {
            if tab.always_on
                || tab.agent_owner.is_some()
                || tab.explorer.uploading()
                || matches!(&self.dialog, Some(Dialog::Close(id, _)) if *id == tab.id)
                || matches!(&self.job, Some((Job::AlwaysOn(id, _), _)) if *id == tab.id)
            {
                continue;
            }
            let Some(last) = tab.last_input else { continue };
            let remaining = IDLE_LIMIT.saturating_sub(now.saturating_duration_since(last));
            if !remaining.is_zero() {
                ctx.request_repaint_after(remaining);
            } else if tab.profile.is_some() {
                if self.job.is_none()
                    && save.is_none()
                    && let (Some(session), Some(profile)) = (&tab.session, &tab.profile)
                {
                    save = Some((
                        tab.id,
                        tab.backend,
                        format!(
                            "save {} {profile} {}",
                            session.token,
                            saved::encode(&tab.title),
                        ),
                    ));
                }
            } else {
                tab.stop_for_idle();
            }
        }
        if let Some((id, backend, cmd)) = save {
            self.request(ctx, Job::IdleSave(id, backend), backend, &cmd);
            if self.job.is_none()
                && let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id)
            {
                tab.last_input = Some(now);
                ctx.request_repaint_after(IDLE_LIMIT);
            }
        }
    }

    fn sync_audio(&mut self, ctx: &egui::Context) {
        for tab in &mut self.tabs {
            if tab.agent_port.is_some() {
                continue;
            }
            let live = tab.session.as_ref().filter(|session| {
                let view = session.view.lock().unwrap();
                view.error.is_none() && (tab.ran || view.frame.is_some())
            });
            let Some(session) = live.filter(|_| !tab.muted) else {
                tab.audio = None;
                continue;
            };
            if tab.audio.is_some() || tab.audio_error.is_some() {
                continue;
            }
            let repaint = ctx.clone();
            match audio::Audio::start(
                tab.connection.host.trim(),
                tab.connection.key.as_deref(),
                &session.token,
                tab.volume.unwrap_or(100),
                move || repaint.request_repaint(),
            ) {
                Ok(audio) => tab.audio = Some(audio),
                Err(error) => tab.audio_error = Some(error.to_string()),
            }
        }
    }

    #[cfg(test)]
    fn add_tab(&mut self, ctx: &egui::Context) {
        self.open_tab(ctx, Backend::Primary, None, None);
    }

    fn connection(&self, backend: Backend) -> Connection {
        match backend {
            Backend::Primary => Connection {
                host: self.host.clone(),
                key: self.identity.clone(),
            },
            Backend::Secondary => self.secondary.clone(),
        }
    }

    fn open_tab(
        &mut self,
        ctx: &egui::Context,
        backend: Backend,
        entry: Option<saved::Entry>,
        address: Option<session::StartPage<'_>>,
    ) {
        let connection = self.connection(backend);
        if self.tabs.len() >= MAX_TABS {
            return;
        }
        if connection.host.trim().is_empty() {
            self.edit_settings();
            return;
        }
        if let Some(index) = entry.as_ref().and_then(|entry| {
            self.tabs.iter().position(|tab| {
                tab.backend == backend
                    && tab.profile.as_ref() == Some(&entry.id)
                    && tab.session.is_some()
            })
        }) {
            self.select_tab(index);
            return;
        }
        let repaint = ctx.clone();
        let pixels_per_point = ctx.pixels_per_point();
        let mut size = ctx
            .input(|input| input.viewport().monitor_size)
            .map(|points| {
                rfb::Resolution::fit(points.x * pixels_per_point, points.y * pixels_per_point)
            })
            .unwrap_or(rfb::Resolution::FALLBACK);
        let agent_port = match &address {
            Some(session::StartPage::Agent(_, port, _)) => Some(*port),
            _ => entry.as_ref().and_then(|entry| entry.agent_port),
        };
        let file_manager = matches!(&address, Some(session::StartPage::Agent(_, _, true)))
            || entry.as_ref().is_some_and(|entry| entry.file_manager);
        let codex = matches!(&address, Some(session::StartPage::Agent("codex", _, _)))
            || entry.as_ref().is_some_and(|entry| entry.codex);
        if file_manager {
            size = rfb::Resolution::FALLBACK;
        } else if codex {
            size = rfb::Resolution(2560, 1440);
        }
        let start = if agent_port.is_some() && entry.is_some() {
            session::StartPage::Url("")
        } else {
            address.unwrap_or(session::StartPage::Url(&self.startup_page))
        };
        match Session::start_profile(
            connection.host.trim(),
            connection.key.as_deref(),
            size,
            entry.as_ref().map(|entry| entry.id.as_str()),
            start,
            if agent_port.is_some() {
                settings::WebsiteTheme::Default
            } else {
                self.website_theme
            },
            move || repaint.request_repaint(),
        ) {
            Ok(session) => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.blur();
                }
                self.next_id += 1;
                let title = entry.as_ref().map_or_else(
                    || format!("Tab {}", self.next_id),
                    |entry| entry.title.clone(),
                );
                let volume = entry
                    .as_ref()
                    .and_then(|entry| settings::tab_volume(connection.host.trim(), &entry.id));
                self.tabs.push(Tab {
                    id: self.next_id,
                    title,
                    volume,
                    backend,
                    agent_port,
                    codex,
                    agent_resolution: if codex {
                        AgentResolution::Qhd
                    } else {
                        AgentResolution::MatchTile
                    },
                    agent_view: if file_manager {
                        AgentView::Files
                    } else if codex {
                        AgentView::Codex
                    } else {
                        AgentView::Freebuff
                    },
                    connection,
                    profile: entry.map(|entry| entry.id),
                    session: Some(session),
                    started: Some(Instant::now()),
                    ..Default::default()
                });
                self.active = self.tabs.len() - 1;
                self.home = false;
                self.error = None;
                ctx.request_repaint();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn close_tab(&mut self, index: usize) {
        let closed = self.tabs[index].id;
        let active = self.tabs.get(self.active).map(|tab| tab.id);
        self.tabs
            .retain(|tab| tab.id != closed && tab.agent_owner != Some(closed));
        self.active = self
            .tabs
            .iter()
            .position(|tab| Some(tab.id) == active)
            .unwrap_or_else(|| {
                index
                    .saturating_sub(1)
                    .min(self.tabs.len().saturating_sub(1))
            });
        self.home |= self.tabs.is_empty();
    }

    fn agent_owner_index(&self, index: usize) -> usize {
        self.tabs[index]
            .agent_owner
            .and_then(|id| self.tabs.iter().position(|tab| tab.id == id))
            .unwrap_or(index)
    }

    fn open_file_editor(&mut self, ctx: &egui::Context, index: usize, editor: files::FileEditor) {
        let owner = &self.tabs[self.agent_owner_index(index)];
        if self.tabs.len() >= MAX_TABS {
            self.error = Some("Close a tab before opening Vim.".into());
            return;
        }
        let (Some(session), Some(port)) = (&owner.session, owner.agent_port) else {
            return;
        };
        let Some(display) = (103..=127).find(|display| {
            !self.tabs.iter().any(|tab| {
                tab.agent_owner == Some(owner.id)
                    && tab.file_editor == Some(*display)
                    && tab.session.is_some()
            })
        }) else {
            self.error =
                Some("Close a Vim tab before opening another editor for this Agent.".into());
            return;
        };
        let repaint = ctx.clone();
        match Session::agent_editor(
            owner.connection.host.trim(),
            owner.connection.key.as_deref(),
            &session.token,
            &editor,
            display,
            move || repaint.request_repaint(),
        ) {
            Ok(session) => {
                self.error = None;
                self.next_id += 1;
                self.tabs.push(Tab {
                    id: self.next_id,
                    backend: owner.backend,
                    connection: owner.connection.clone(),
                    title: format!("SH Terminal · {}", editor.name),
                    agent_port: Some(port),
                    codex: owner.codex,
                    agent_view: AgentView::Shell,
                    agent_owner: Some(owner.id),
                    agent_parent_title: Some(owner.title.clone()),
                    file_editor: Some(display),
                    always_on: owner.always_on,
                    session: Some(session),
                    started: Some(Instant::now()),
                    ..Default::default()
                });
                self.select_tab(self.tabs.len() - 1);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn open_agent_view(&mut self, ctx: &egui::Context, index: usize, kind: AgentView) {
        let owner = &self.tabs[self.agent_owner_index(index)];
        let owner_id = owner.id;
        if let Some(existing) = self.tabs.iter().position(|tab| {
            (tab.id == owner_id || tab.agent_owner == Some(owner_id))
                && tab.agent_view == kind
                && tab.file_editor.is_none()
                && tab.session.is_some()
        }) {
            self.select_tab(existing);
            return;
        }
        if self.tabs.len() >= MAX_TABS {
            self.error = Some("Close a tab before opening another view.".into());
            return;
        }
        let (Some(session), Some(port)) = (&owner.session, owner.agent_port) else {
            return;
        };
        let connection = owner.connection.clone();
        let repaint = ctx.clone();
        match Session::agent_view(
            connection.host.trim(),
            connection.key.as_deref(),
            &session.token,
            kind,
            move || repaint.request_repaint(),
        ) {
            Ok(session) => {
                self.next_id += 1;
                self.tabs.push(Tab {
                    id: self.next_id,
                    backend: owner.backend,
                    connection,
                    title: kind.label().into(),
                    agent_port: Some(port),
                    codex: owner.codex,
                    agent_resolution: if owner.codex {
                        AgentResolution::Qhd
                    } else {
                        AgentResolution::MatchTile
                    },
                    agent_view: kind,
                    agent_owner: Some(owner_id),
                    agent_parent_title: Some(owner.title.clone()),
                    always_on: owner.always_on,
                    session: Some(session),
                    started: Some(Instant::now()),
                    ..Default::default()
                });
                self.select_tab(self.tabs.len() - 1);
                ctx.request_repaint();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn select_tab(&mut self, index: usize) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.blur();
        }
        self.active = index;
        self.home = false;
        self.tabs[index].clipboard_error = None;
        if self.tabs[index].terminal_view().is_some() && self.tabs[index].session.is_some() {
            self.address_tab = Some(self.tabs[index].id);
            self.browser_address.clear();
        }
    }

    fn edit_settings(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.blur();
        }
        let path = |key: &Option<std::path::PathBuf>| {
            key.as_ref()
                .map_or_else(String::new, |p| p.to_string_lossy().into_owned())
        };
        self.settings_edit = Some(SettingsEdit {
            host: self.host.clone(),
            key: path(&self.identity),
            secondary: self.secondary.host.clone(),
            secondary_key: path(&self.secondary.key),
            startup_page: self.startup_page.clone(),
            website_theme: self.website_theme,
            section: if self.host.is_empty() && self.secondary.host.is_empty() {
                SettingsSection::Connections
            } else {
                SettingsSection::Startup
            },
            error: None,
        });
    }

    fn request(&mut self, ctx: &egui::Context, job: Job, backend: Backend, request: &str) {
        self.request_with_input(ctx, job, backend, request, None);
    }

    fn request_with_input(
        &mut self,
        ctx: &egui::Context,
        job: Job,
        backend: Backend,
        request: &str,
        input: Option<Vec<u8>>,
    ) {
        if self.job.is_some() {
            return;
        }
        self.error = None;
        let repaint = ctx.clone();
        let connection = self.connection(backend);
        match saved::run_with_input(
            connection.host.trim(),
            connection.key.as_deref(),
            request,
            input,
            move || repaint.request_repaint(),
        ) {
            Ok(task) => self.job = Some((job, task)),
            Err(error) => {
                self.quitting = false;
                self.error = Some(error.to_string());
            }
        }
    }

    fn refresh(&mut self, ctx: &egui::Context) {
        let backend = self.library_host;
        if self.job.is_none() && !self.connection(backend).host.trim().is_empty() {
            self.library_loaded = Some(backend);
            self.request(ctx, Job::List(backend), backend, "list-v2");
        }
    }

    fn poll_job(&mut self, ctx: &egui::Context) {
        use std::sync::mpsc::TryRecvError;
        let Some((_, rx)) = &self.job else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err(std::io::Error::other("Saved session request disconnected"))
            }
        };
        let (job, _) = self.job.take().unwrap();
        let idle_id = match &job {
            Job::IdleSave(id, _) => Some(*id),
            _ => None,
        };
        let result = match job {
            Job::CloneTab(backend) => result.and_then(|data| {
                let mut entries = saved::parse(&data)?;
                let entry = entries
                    .pop()
                    .filter(|_| entries.is_empty())
                    .ok_or_else(|| {
                        std::io::Error::other("The server did not return the cloned tab.")
                    })?;
                match backend {
                    Backend::Primary => &mut self.entries,
                    Backend::Secondary => &mut self.secondary_entries,
                }
                .push(entry.clone());
                if !self.quitting {
                    self.open_tab(ctx, backend, Some(entry), None);
                }
                Ok(())
            }),
            Job::OpenPreview(id) => result.and_then(|report| {
                if !self.quitting
                    && let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id)
                    && let Some(session) = &tab.session
                    && let Some(port) = tab.agent_port
                {
                    if let Some(url) = preview::published_url(&report, port) {
                        preview::open_brave(&url)?;
                        tab.preview = None;
                    } else if let Some(preview) = &tab.preview {
                        preview.open_brave()?;
                    } else {
                        let preview = preview::Preview::start(
                            tab.connection.host.trim(),
                            tab.connection.key.as_deref(),
                            &session.token,
                        )?;
                        preview.open_brave()?;
                        tab.preview = Some(preview);
                    }
                }
                Ok(())
            }),
            Job::AgentInfo(info, title) => result.map(|data| {
                if !self.quitting {
                    self.dialog = Some(Dialog::AgentInfo(info, title, data));
                }
            }),
            Job::AgentMemory(tab) => result.and_then(|data| {
                let data = AgentMemory::parse(&data)?;
                if !self.quitting {
                    self.dialog = Some(Dialog::AgentMemory(AgentMemoryDraft {
                        tab,
                        mib: data.limit.div_ceil(AgentMemory::MIB),
                        data,
                        message: None,
                    }));
                }
                Ok(())
            }),
            Job::SetAgentMemory(mut draft) => {
                match result.and_then(|data| AgentMemory::parse(&data)) {
                    Ok(data) => {
                        draft.data = data;
                        draft.mib = data.limit.div_ceil(AgentMemory::MIB);
                        draft.message = Some((
                            "Memory limit applied. The Agent is still running.".into(),
                            false,
                        ));
                    }
                    Err(error) => draft.message = Some((error.to_string(), true)),
                }
                if !self.quitting {
                    self.dialog = Some(Dialog::AgentMemory(draft));
                }
                Ok(())
            }
            Job::BuildAgent(backend, image, name, port, slot) => result.map(|_| {
                if !self.quitting {
                    let count = self.tabs.len();
                    self.launch_agent(ctx, backend, &image, &name, port, false);
                    self.place_new_tab(count, slot);
                }
            }),
            Job::OpenSaved(backend, entry, slot) => result.and_then(|data| {
                let list = saved::parse_parked(&data)?;
                if !self.quitting {
                    if let Some(index) = self.tabs.iter().position(|tab| {
                        tab.backend == backend
                            && tab.profile.as_deref() == Some(&entry.id)
                            && tab.session.is_some()
                    }) {
                        if let Some(slot) = slot {
                            let id = self.tabs[index].id;
                            self.place_in_vacant_tile(id, slot);
                        } else {
                            self.select_tab(index);
                        }
                    } else if let Some(parked) = list
                        .into_iter()
                        .find(|item| item.profile.as_deref() == Some(&entry.id))
                    {
                        let count = self.tabs.len();
                        self.resume_parked(ctx, backend, parked);
                        self.place_new_tab(count, slot);
                    } else if entry.agent_port.is_some() {
                        let command = format!("agent-prepare-media {}", entry.id);
                        self.request(
                            ctx,
                            Job::PrepareAgentMedia(backend, entry, slot),
                            backend,
                            &command,
                        );
                    } else {
                        let count = self.tabs.len();
                        self.open_tab(ctx, backend, Some(entry), None);
                        self.place_new_tab(count, slot);
                    }
                }
                Ok(())
            }),
            Job::PrepareAgentMedia(backend, entry, slot) => {
                if !self.quitting {
                    let problem = match result {
                        Ok(reply) if reply.trim() == "RECTSMBPREP1" => None,
                        Ok(_) => Some("The server did not confirm SMB preparation.".into()),
                        Err(error)
                            if error
                                .to_string()
                                .contains("Invalid Rectangle display resolution.") =>
                        {
                            None
                        }
                        Err(error) => Some(error.to_string()),
                    };
                    let count = self.tabs.len();
                    self.open_tab(ctx, backend, Some(entry), None);
                    self.place_new_tab(count, slot);
                    if let Some(error) = problem {
                        self.error = Some(format!("SMB is still pending: {error}"));
                    }
                }
                Ok(())
            }
            Job::List(backend) => {
                let result =
                    result
                        .and_then(|data| saved::parse(&data))
                        .map(|list| match backend {
                            Backend::Primary => self.entries = list,
                            Backend::Secondary => self.secondary_entries = list,
                        });
                if result.is_ok() && !self.quitting && backend == self.library_host {
                    self.request(ctx, Job::ParkedList(backend), backend, "parked-list");
                }
                result
            }
            Job::ParkedList(backend) => {
                result
                    .and_then(|data| saved::parse_parked(&data))
                    .map(|list| {
                        if self.library_host == backend {
                            self.parked = list;
                        }
                    })
            }
            Job::Park(id, backend) => result.and_then(|data| {
                if data.trim() != "OK" {
                    return Err(std::io::Error::other("The host did not confirm parking."));
                }
                if let Some(index) = self.tabs.iter().position(|tab| tab.id == id) {
                    self.close_tab(index);
                }
                if self.quitting {
                    ctx.request_repaint();
                } else {
                    self.home = true;
                    self.library_host = backend;
                    self.parked.clear();
                    self.refresh(ctx);
                }
                Ok(())
            }),
            Job::StopParked(backend) => result.and_then(|data| {
                if data.trim() != "OK" {
                    return Err(std::io::Error::other(
                        "The host did not confirm stopping the parked session.",
                    ));
                }
                self.request(ctx, Job::ParkedList(backend), backend, "parked-list");
                Ok(())
            }),
            Job::Close(id, backend) => result.and_then(|data| {
                if data.trim() != "OK" {
                    return Err(std::io::Error::other(
                        "The server did not confirm the close",
                    ));
                }
                if let Some(index) = self.tabs.iter().position(|tab| tab.id == id) {
                    self.close_tab(index);
                }
                if !self.quitting {
                    self.request(ctx, Job::List(backend), backend, "list-v2");
                }
                Ok(())
            }),
            Job::IdleSave(id, backend) => result.and_then(|data| {
                if data.trim() != "OK" {
                    return Err(std::io::Error::other(
                        "The server did not confirm the idle session was saved",
                    ));
                }
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
                    tab.stop_for_idle();
                }
                if !self.quitting {
                    self.request(ctx, Job::List(backend), backend, "list-v2");
                }
                Ok(())
            }),
            Job::AlwaysOn(id, enabled) => result.and_then(|data| {
                if data.trim() != "OK" {
                    return Err(std::io::Error::other(
                        "The server did not confirm Always on.",
                    ));
                }
                for tab in self
                    .tabs
                    .iter_mut()
                    .filter(|tab| tab.id == id || tab.agent_owner == Some(id))
                {
                    tab.always_on = enabled;
                    tab.last_input = Some(Instant::now());
                }
                Ok(())
            }),
            Job::Forget(backend) => result.and_then(|data| {
                if data.trim() != "OK" {
                    return Err(std::io::Error::other("The server did not confirm deletion"));
                }
                self.request(ctx, Job::List(backend), backend, "list-v2");
                Ok(())
            }),
            Job::StartupPage(backend, entry) => result.and_then(|data| {
                if data.trim() != "OK" {
                    return Err(std::io::Error::other(
                        "The server did not confirm the startup page change",
                    ));
                }
                let entries = match backend {
                    Backend::Primary => &mut self.entries,
                    Backend::Secondary => &mut self.secondary_entries,
                };
                if let Some(item) = entries.iter_mut().find(|item| item.id == entry.id) {
                    item.startup_page = entry.startup_page;
                }
                Ok(())
            }),
        };
        if let Err(error) = result {
            if let Some(id) = idle_id
                && let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id)
            {
                tab.last_input = Some(Instant::now());
                ctx.request_repaint_after(IDLE_LIMIT);
            }
            self.error = Some(error.to_string());
            self.quitting = false;
        }
    }

    fn park_tab(&mut self, ctx: &egui::Context, index: usize) {
        let index = self.agent_owner_index(index);
        if self.job.is_none()
            && let Some(tab) = self.tabs.get_mut(index)
            && let Some(session) = &tab.session
        {
            let cmd = format!(
                "park {} {} {}",
                session.token,
                saved::encode(&tab.title),
                tab.snap.map_or("-", Snap::code)
            );
            let (id, backend) = (tab.id, tab.backend);
            tab.blur();
            self.request(ctx, Job::Park(id, backend), backend, &cmd);
        }
    }

    fn park_on_quit(&mut self, ctx: &egui::Context) {
        if self.job.is_some() {
            return;
        }
        // A stopped tab has no browser to park. Process the rest one at a time.
        while let Some(index) = self.tabs.len().checked_sub(1) {
            let tab = &mut self.tabs[index];
            if tab.agent_owner.is_some() {
                self.close_tab(index);
                continue;
            }
            tab.poll(ctx);
            if tab.session.is_none() {
                self.close_tab(index);
                continue;
            }
            if tab.ran {
                self.park_tab(ctx, index);
            }
            return;
        }
        if let Some(candidate) = self.pending_update.take()
            && let Err(error) = update::install(&candidate)
        {
            self.error = Some(format!("Could not restart to install the update: {error}"));
            self.quitting = false;
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn ask_close(&mut self, ctx: &egui::Context, index: usize) {
        if self
            .tabs
            .get(index)
            .is_some_and(|tab| tab.agent_owner.is_some())
        {
            self.close_tab(index);
            return;
        }
        if self.job.is_none()
            && self.dialog.is_none()
            && let Some(tab) = self.tabs.get_mut(index)
        {
            tab.blur();
            if tab.idle_stopped {
                self.close_tab(index);
                return;
            }
            if let Some(profile) = &tab.profile {
                if let Some(session) = &tab.session {
                    let (id, backend) = (tab.id, tab.backend);
                    let name = saved::title(&tab.title).expect("tab has a valid title");
                    let cmd = format!("save {} {profile} {}", session.token, saved::encode(name));
                    self.request(ctx, Job::Close(id, backend), backend, &cmd);
                } else {
                    // The remote browser is already gone. Keep its saved profile in Home.
                    self.close_tab(index);
                }
            } else {
                self.dialog = Some(Dialog::Close(tab.id, tab.title.clone()));
            }
        }
    }

    fn launch_agent(
        &mut self,
        ctx: &egui::Context,
        backend: Backend,
        id: &str,
        name: &str,
        port: u16,
        file_manager: bool,
    ) {
        let count = self.tabs.len();
        self.open_tab(
            ctx,
            backend,
            None,
            Some(session::StartPage::Agent(id, port, file_manager)),
        );
        if self.tabs.len() > count {
            self.tabs.last_mut().unwrap().title = name.into();
        }
    }

    fn start_new_tab(&mut self, ctx: &egui::Context, draft: NewTab) {
        let count = self.tabs.len();
        let slot = draft.slot;
        if draft.image == 0 {
            self.open_tab(ctx, draft.backend, None, None);
        } else if let Some(name) = saved::title(&draft.name)
            && let Ok(port) = draft.port.trim().parse::<u16>()
            && port >= 1024
        {
            if draft.image <= 2 {
                self.launch_agent(
                    ctx,
                    draft.backend,
                    if draft.codex { "codex" } else { "demo" },
                    name,
                    port,
                    draft.image == 2,
                );
            } else if let Some(image) = self.agent_images.get(draft.image - 3).cloned() {
                self.request_with_input(
                    ctx,
                    Job::BuildAgent(draft.backend, image.id.clone(), name.to_owned(), port, slot),
                    draft.backend,
                    &format!("agent-build {}", image.id),
                    Some(image.dockerfile.into_bytes()),
                );
            }
        } else {
            self.dialog = Some(Dialog::NewTab(NewTab {
                error: Some("Enter a tab name and a TCP port from 1024 to 65535.".into()),
                ..draft
            }));
        }
        self.place_new_tab(count, slot);
    }

    fn suggest_agent_port(&self, ctx: &egui::Context, draft: &mut NewTab) {
        if draft.port_host != Some(draft.backend) {
            draft.port_host = Some(draft.backend);
            draft.port_error = None;
            draft.port_lookup = None;
            let connection = self.connection(draft.backend);
            if connection.host.trim().is_empty() {
                draft.port_error =
                    Some("Configure SSH for this server to find a free port.".into());
                return;
            }
            let repaint = ctx.clone();
            match saved::run(
                connection.host.trim(),
                connection.key.as_deref(),
                "agent-next-port",
                move || repaint.request_repaint(),
            ) {
                Ok(job) => draft.port_lookup = Some(job),
                Err(error) => draft.port_error = Some(error.to_string()),
            }
        }
        if let Some(job) = &draft.port_lookup {
            match job.try_recv() {
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                response => {
                    draft.port_lookup = None;
                    match response {
                        Ok(Ok(response)) => {
                            if let Some(port) = response
                                .trim()
                                .strip_prefix("RECTAGENTPORT1 ")
                                .and_then(|port| port.parse::<u16>().ok())
                                .filter(|p| *p >= 4000)
                            {
                                if draft.port.trim().is_empty() {
                                    draft.port = port.to_string();
                                }
                            } else {
                                draft.port_error =
                                    Some("The server did not return a free port.".into());
                            }
                        }
                        Ok(Err(error)) => {
                            draft.port_error = Some(
                                if error
                                    .to_string()
                                    .contains("Invalid Rectangle display resolution.")
                                {
                                    "Update this server to enable port suggestions, or enter a port manually.".into()
                                } else {
                                    error.to_string()
                                },
                            )
                        }
                        Err(_) => draft.port_error = Some("The port lookup disconnected.".into()),
                    }
                }
            }
        }
    }

    fn show_new_tab(&mut self, ctx: &egui::Context, draft: &mut NewTab) -> u8 {
        let mut choice = 0;
        egui::Window::new("New tab · Choose image")
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false).resizable(false)
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 70.0).clamp(325.0, 615.0));
                ui.horizontal(|ui| {
                    ui.label("Run on");
                    for backend in [Backend::Primary, Backend::Secondary] {
                        let label = if self.connection(backend).host.is_empty() {
                            format!("{} (not configured)", backend.name(""))
                        } else { backend.name(&self.host).to_owned() };
                        if ui.selectable_value(&mut draft.backend, backend, label).changed() {
                            draft.port.clear();
                            draft.port_host = None;
                        }
                    }
                });
                let names: Vec<_> = ["Browser", "Agent", "File Manager"].into_iter().chain(self.agent_images.iter().map(|image| image.name.as_str())).collect();
                ui.horizontal(|ui| {
                    ui.label("Image");
                    egui::ComboBox::from_id_salt("new-tab-image")
                        .selected_text(names.get(draft.image).copied().unwrap_or("Browser"))
                        .show_ui(ui, |ui| {
                            for (index, name) in names.iter().enumerate() {
                                if ui.selectable_value(&mut draft.image, index, *name).changed() {
                                    draft.dockerfile = None;
                                    draft.copy_name.clear();
                                    draft.error = None;
                                }
                            }
                        });
                    if ui.button(if draft.image == 0 { "View Dockerfile" } else { "Edit Dockerfile" }).clicked() {
                        draft.dockerfile = Some(match draft.image {
                            0 => images::BROWSER.to_owned(),
                            1 | 2 if draft.codex => images::CODEX.to_owned(),
                            1 | 2 => images::AGENT.to_owned(),
                            index => self.agent_images[index - 3].dockerfile.clone(),
                        });
                    }
                });
                if draft.image != 0 {
                    self.suggest_agent_port(ctx, draft);
                    if draft.image <= 2 {
                        ui.horizontal(|ui| {
                            ui.label("Agent app");
                            let freebuff = ui.selectable_value(&mut draft.codex, false, "Freebuff").changed();
                            let codex = ui.selectable_value(&mut draft.codex, true, "ChatGPT Codex").changed();
                            if freebuff || codex {
                                draft.dockerfile = None;
                            }
                        });
                    }
                    ui.label(if draft.image == 2 { "Agent workspace in a file explorer" } else if draft.codex { "ChatGPT Codex desktop app · Debian 13" } else { "Alpine terminal with Node.js and Freebuff" });
                    ui.horizontal(|ui| {
                        ui.label("Tab name");
                        ui.add(egui::TextEdit::singleline(&mut draft.name).hint_text("Required"));
                    });
                    ui.horizontal(|ui| {
                        ui.label("TCP port on host");
                        ui.add(egui::TextEdit::singleline(&mut draft.port).desired_width(90.0).hint_text("Automatic"));
                        if draft.port_lookup.is_some() { ui.spinner(); }
                        if ui.add_enabled(draft.port_lookup.is_none(), egui::Button::new("Find free port")).clicked() {
                            draft.port.clear();
                            draft.port_host = None;
                            ctx.request_repaint();
                        }
                    });
                    if let Some(error) = &draft.port_error { ui.label(RichText::new(error).color(Color32::LIGHT_RED)); }
                    ui.label(RichText::new("Check Open ports for the host binding. Agent bypasses the browser VPN.").small().color(MUTED));
                }
                if let Some(file) = &mut draft.dockerfile {
                    ui.separator();
                    ui.label(if draft.image == 0 { "Browser Dockerfile · read only" } else { "Agent Dockerfile · save changes as a named copy" });
                    egui::ScrollArea::both().max_height((ctx.content_rect().height() - 240.0).clamp(100.0, 330.0)).show(ui, |ui| {
                        ui.add(egui::TextEdit::multiline(file).code_editor().desired_width(ui.available_width()).desired_rows(9).interactive(draft.image != 0));
                    });
                    if draft.image != 0 {
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut draft.copy_name).hint_text("New image name"));
                            if ui.button("Save as new image").clicked() { choice = 2; }
                        });
                    }
                    if ui.button("Close Dockerfile").clicked() { draft.dockerfile = None; }
                }
                if let Some(err) = &draft.error { ui.label(RichText::new(err).color(Color32::LIGHT_RED)); }
                ui.separator();
                ui.horizontal(|ui| {
                    let name = if draft.image > 2 { "Build and open" } else { "Open tab" };
                    let source = match draft.image { 0 => images::BROWSER, 1 | 2 if draft.codex => images::CODEX, 1 | 2 => images::AGENT, index => &self.agent_images[index - 3].dockerfile };
                    let edits_saved = draft.dockerfile.as_ref().is_none_or(|draft| draft == source);
                    let valid_agent = draft.image == 0 || (saved::title(&draft.name).is_some() && draft.port.trim().parse::<u16>().is_ok_and(|port| port >= 1024));
                    if ui.add_enabled(self.job.is_none() && edits_saved && valid_agent && self.tabs.len() < MAX_TABS, egui::Button::new(name))
                        .on_disabled_hover_text("Save Dockerfile changes as a new image before opening.").clicked() { choice = 1; }
                    if ui.button("Cancel").clicked() { choice = 3; }
                });
            });
        choice
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(mut draft) = self.settings_edit.take() {
            use SettingsSection::{Connections, Proton, Startup, Updates};
            let update_hosts: Vec<_> = [Backend::Primary, Backend::Secondary]
                .into_iter()
                .map(|backend| self.connection(backend))
                .filter(|connection| !connection.host.trim().is_empty())
                .collect();
            let mut save = false;
            let mut cancel = false;
            let mut update = None;
            let key_path = |draft: &str| {
                (!draft.trim().is_empty()).then(|| std::path::PathBuf::from(draft.trim()))
            };
            egui::Window::new("Settings")
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    let width = (ctx.content_rect().width() - 72.0).clamp(300.0, 500.0);
                    ui.set_width(width);
                    ui.horizontal(|ui| {
                        for (section, label) in [
                            (Startup, "Startup"),
                            (Connections, "Connections"),
                            (Proton, "Proton VPN"),
                            (Updates, "Updates"),
                        ] {
                            ui.selectable_value(&mut draft.section, section, label);
                        }
                    });
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .id_salt(draft.section as u8)
                        .max_height((ctx.content_rect().height() - 185.0).clamp(160.0, 460.0))
                        .show(ui, |ui| match draft.section {
                            Startup => {
                                ui.heading("Startup page");
                                ui.label(RichText::new("Opens when a new session starts.").color(MUTED));
                                ui.add_space(8.0);
                                ui.label("Website address");
                                ui.add(egui::TextEdit::singleline(&mut draft.startup_page)
                                    .desired_width(width - 24.0)
                                    .hint_text("https://example.com"));
                                ui.label(RichText::new("Leave blank to open the Rectangle welcome page.").color(MUTED));
                                ui.add_space(12.0);
                                ui.heading("Website theme");
                                ui.horizontal(|ui| {
                                    use settings::WebsiteTheme::{Dark, Default, Light};
                                    ui.selectable_value(&mut draft.website_theme, Default, "Firefox default");
                                    ui.selectable_value(&mut draft.website_theme, Light, "Light");
                                    ui.selectable_value(&mut draft.website_theme, Dark, "Dark");
                                });
                                ui.label(RichText::new("Applies when a Browser container starts. Save and reopen running sessions to apply it.").color(MUTED));
                                ui.label(RichText::new("Light and Dark require a Rectangle 0.1.4 or newer server.").color(MUTED));
                            }
                            Connections => {
                                ui.heading("SSH connections");
                                ui.label(RichText::new("Rectangle starts on Home. Choose a host when opening a tab.").color(MUTED));
                                let primary_label = Backend::Primary.name(&draft.host).to_owned();
                                for (label, example, host, key) in [
                                    (primary_label.as_str(), "user@host", &mut draft.host, &mut draft.key),
                                    ("Secondary", "user@host", &mut draft.secondary, &mut draft.secondary_key),
                                ] {
                                    ui.add_space(8.0);
                                    design::frame(egui::Color32::from_rgb(22, 24, 30), 10, 12).show(ui, |ui| {
                                        ui.strong(label);
                                        ui.label("SSH host or user@host");
                                        ui.add(egui::TextEdit::singleline(host)
                                            .desired_width(width - 48.0)
                                            .hint_text(example));
                                        ui.label("Private key file path (optional)");
                                        ui.add(egui::TextEdit::singleline(key)
                                            .desired_width(width - 48.0)
                                            .hint_text("Use SSH defaults if blank"));
                                    });
                                }
                            }
                            Proton => self.proton.settings(ui, Backend::Primary.name(&self.host), self.host.trim(), self.identity.as_deref()),
                            Updates => { update = self.updater.show(ui, &update_hosts); }
                        });
                    if let Some(error) = &draft.error {
                        ui.label(RichText::new(error).color(Color32::LIGHT_RED));
                    }
                    ui.add_space(6.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        if draft.section != Updates {
                            save = ui.add_enabled(self.job.is_none(), egui::Button::new("Save settings")).clicked();
                        }
                        cancel = ui.button("Close").clicked();
                    });
                });
            if let Some(candidate) = update {
                self.pending_update = Some(candidate);
                self.quitting = true;
                ctx.request_repaint();
                return;
            }
            if save {
                let host = draft.host.trim();
                let secondary = draft.secondary.trim();
                let primary_key = key_path(&draft.key);
                let secondary_key = key_path(&draft.secondary_key);
                let primary_changed = host != self.host || primary_key != self.identity;
                let secondary_changed =
                    secondary != self.secondary.host || secondary_key != self.secondary.key;
                let result = settings::startup_page(&draft.startup_page)
                    .inspect_err(|_| draft.section = Startup)
                    .and_then(|page| {
                        if self.tabs.iter().any(|tab| match tab.backend {
                            Backend::Primary => primary_changed,
                            Backend::Secondary => secondary_changed,
                        }) {
                            draft.section = Connections;
                            return Err(std::io::Error::other(
                                "Close the tabs for the connection you’re changing.",
                            ));
                        }
                        settings::save(
                            host,
                            draft.key.trim(),
                            secondary,
                            draft.secondary_key.trim(),
                            &page,
                            draft.website_theme,
                        )?;
                        Ok(page)
                    });
                match result {
                    Ok(page) => {
                        self.startup_page = page;
                        self.website_theme = draft.website_theme;
                        if primary_changed {
                            self.proton = proton::Proton::default();
                            self.primary_memory = memory::HostMeter::default();
                            self.primary_disk = memory::HostMeter::default();
                            self.entries.clear();
                        }
                        if secondary_changed {
                            self.secondary_entries.clear();
                        }
                        self.host = host.to_owned();
                        self.identity = primary_key;
                        self.secondary = Connection {
                            host: secondary.to_owned(),
                            key: secondary_key,
                        };
                        self.error = None;
                        if self.home && (primary_changed || secondary_changed) {
                            self.refresh(ctx);
                        }
                    }
                    Err(error) => {
                        draft.error = Some(error.to_string());
                        self.settings_edit = Some(draft);
                    }
                }
            } else if !cancel {
                self.settings_edit = Some(draft);
            }
        }
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        if let Dialog::NewTab(mut draft) = dialog {
            match self.show_new_tab(ctx, &mut draft) {
                1 => self.start_new_tab(ctx, draft),
                2 => {
                    let result = images::AgentImage::create(
                        &draft.copy_name,
                        draft.dockerfile.clone().unwrap_or_default(),
                        &self.agent_images,
                    )
                    .and_then(|image| {
                        settings::save_agent_image(&image)?;
                        Ok(image)
                    });
                    match result {
                        Ok(image) => {
                            self.agent_images.push(image);
                            draft.image = self.agent_images.len() + 2;
                            draft.dockerfile = None;
                            draft.copy_name.clear();
                            draft.error = None;
                        }
                        Err(err) => draft.error = Some(err.to_string()),
                    }
                    self.dialog = Some(Dialog::NewTab(draft));
                }
                3 => {}
                _ => self.dialog = Some(Dialog::NewTab(draft)),
            }
            return;
        }
        let mut choice = 0;
        let heading = match dialog {
            Dialog::AgentInfo(info, ..) => info.label(),
            Dialog::AgentMemory(_) => "Agent memory limit",
            Dialog::Rename(..) => "Rename tab",
            Dialog::Clone(..) => "Clone tab",
            Dialog::Close(..) => "Close tab",
            Dialog::Forget(..) => "Delete saved session",
            Dialog::StopParked(..) => "Stop parked session",
            Dialog::StartupPage(..) => "Saved session startup page",
            Dialog::NewTab(_) => unreachable!(),
        };
        egui::Window::new(heading)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                match &mut dialog {
                    Dialog::AgentInfo(info, title, data) => {
                        ui.label(RichText::new(title.as_str()).strong());
                        if matches!(info, AgentInfo::Ports) {
                            ui.label("TCP listeners and bound UDP sockets. A published port may have no service listening.");
                        }
                        egui::ScrollArea::vertical().max_height(350.0).show(ui, |ui| {
                            ui.add(egui::Label::new(RichText::new(data.trim()).monospace()).selectable(true));
                        });
                        if ui.button("Close").clicked() || ctx.input(|input| input.key_pressed(Key::Escape)) {
                            choice = 3;
                        }
                        return;
                    }
                    Dialog::AgentMemory(draft) => {
                        let gib = |bytes| bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                        if let Some(tab) = self.tabs.iter().find(|tab| tab.id == draft.tab) {
                            ui.label(RichText::new(&tab.title).strong());
                        }
                        ui.label(format!(
                            "{:.2} GiB used / {:.2} GiB limit · {:.2} GiB host RAM",
                            gib(draft.data.usage), gib(draft.data.limit), gib(draft.data.host_total)
                        ));
                        ui.horizontal(|ui| {
                            ui.label("New limit");
                            if ui.add(egui::DragValue::new(&mut draft.mib).range(256..=1_048_576).suffix(" MiB")).changed() {
                                draft.message = None;
                            }
                        });
                        ui.label("Decreasing the limit leaves less room for running processes.");
                        let validation = draft.data.check_limit(draft.mib);
                        if let Err(error) = validation {
                            ui.colored_label(Color32::LIGHT_RED, error);
                        }
                        if let Some((message, error)) = &draft.message {
                            ui.colored_label(if *error { Color32::LIGHT_RED } else { INK }, message);
                        }
                        ui.horizontal(|ui| {
                            if ui.add_enabled(self.job.is_none() && validation.is_ok(), egui::Button::new("Apply")).clicked() {
                                choice = 1;
                            }
                            if ui.add_enabled(self.job.is_none(), egui::Button::new("Refresh usage")).clicked() {
                                choice = 5;
                            }
                            if ui.button("Close").clicked() || ctx.input(|input| input.key_pressed(Key::Escape)) {
                                choice = 3;
                            }
                        });
                        return;
                    }
                    Dialog::NewTab(_) => unreachable!(),
                    Dialog::Rename(_, name) | Dialog::Close(_, name) => {
                        ui.text_edit_singleline(name);
                    }
                    Dialog::Clone(_, name) => {
                        ui.label("Name the new tab");
                        ui.text_edit_singleline(name);
                        ui.label("The source pauses during copying. The clone is saved to Home and opens when ready.");
                    }
                    Dialog::Forget(_, entry) => {
                        let data = if entry.agent_port.is_some() { "Agent workspace" } else { "browser data" };
                        ui.label(format!("Delete “{}” and its {data}?", entry.title));
                    }
                    Dialog::StopParked(_, entry) => {
                        ui.label(format!(
                            "Stop “{}”? {}",
                            entry.title,
                            if entry.agent_port.is_some() {
                                "The running terminal will close. An unsaved workspace will be deleted."
                            } else {
                                "Its open page will no longer be resumable."
                            }
                        ));
                        ui.horizontal(|ui| {
                            if ui.button("Stop session").clicked() {
                                choice = 2;
                            }
                            if ui.button("Cancel").clicked() {
                                choice = 3;
                            }
                        });
                        return;
                    }
                    Dialog::StartupPage(_, entry, page, error) => {
                        ui.label(RichText::new(&entry.title).strong());
                        ui.label("Website address");
                        ui.add(
                            egui::TextEdit::singleline(page)
                                .desired_width(360.0)
                                .hint_text("https://example.com"),
                        );
                        ui.label("Leave blank to use the global startup page.");
                        if let Some(error) = error {
                            ui.label(RichText::new(error.as_str()).color(Color32::LIGHT_RED));
                        }
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(self.job.is_none(), egui::Button::new("Save"))
                                .clicked()
                            {
                                choice = 1;
                            }
                            if ui.button("Cancel").clicked() {
                                choice = 3;
                            }
                        });
                        return;
                    }
                }
                if let Dialog::Close(id, _) = &dialog {
                    let agent = self.tabs.iter().any(|tab| tab.id == *id && tab.agent_port.is_some());
                    ui.label(if agent {
                        "Save to Home to keep the Agent workspace and app data. Park to keep the app and port running."
                    } else {
                        "Save to Home to keep cookies and site data on the server."
                    });
                    if agent && ui.button("Park tab").clicked() {
                        choice = 4;
                    }
                }
                ui.horizontal(|ui| {
                    match &dialog {
                        Dialog::Rename(_, name) | Dialog::Close(_, name) => {
                            let label = if matches!(dialog, Dialog::Rename(..)) {
                                "Rename"
                            } else {
                                "Save and close"
                            };
                            if ui
                                .add_enabled(saved::title(name).is_some(), egui::Button::new(label))
                                .clicked()
                            {
                                choice = 1;
                            }
                        }
                        Dialog::Clone(id, name) => {
                            let valid = saved::title(name).is_some_and(|name| {
                                self.tabs.iter().any(|tab| tab.id == *id && tab.session.is_some())
                                    && self.tabs.iter().all(|tab| !tab.title.eq_ignore_ascii_case(name))
                            });
                            if ui.add_enabled(valid && self.job.is_none() && self.tabs.len() < MAX_TABS, egui::Button::new("Clone")).clicked() {
                                choice = 1;
                            }
                        }
                        _ => {}
                    }
                    if !matches!(dialog, Dialog::Rename(..) | Dialog::Clone(..)) && ui.button("Delete").clicked() {
                        choice = 2;
                    }
                    if ui.button("Cancel").clicked() {
                        choice = 3;
                    }
                });
            });
        match (choice, dialog) {
            (0, dialog) => self.dialog = Some(dialog),
            (action @ (1 | 5), Dialog::AgentMemory(draft)) => {
                if let Some(tab) = self.tabs.iter().find(|tab| tab.id == draft.tab)
                    && let Some(session) = &tab.session
                    && tab.agent_port.is_some()
                {
                    let (job, cmd) = if action == 1 {
                        let cmd = format!("agent-memory {} {}", session.token, draft.mib);
                        (Job::SetAgentMemory(draft), cmd)
                    } else {
                        (
                            Job::AgentMemory(draft.tab),
                            format!("agent-memory {}", session.token),
                        )
                    };
                    self.request(ctx, job, tab.backend, &cmd);
                } else {
                    self.error = Some("This Agent is no longer running.".into());
                }
            }
            (3, Dialog::AgentMemory(_)) => {}
            (4, Dialog::Close(id, _)) => {
                if let Some(index) = self.tabs.iter().position(|tab| tab.id == id) {
                    self.park_tab(ctx, index);
                }
            }
            (1, Dialog::Rename(id, name)) => {
                for tab in &mut self.tabs {
                    if tab.id == id {
                        tab.title = name.trim().into();
                    } else if tab.agent_owner == Some(id) {
                        tab.agent_parent_title = Some(name.trim().into());
                    }
                }
            }
            (1, Dialog::Clone(id, name)) => {
                if let Some(tab) = self.tabs.iter().find(|tab| tab.id == id)
                    && let Some(session) = &tab.session
                {
                    let mut request =
                        format!("clone-tab {} {}", session.token, saved::encode(name.trim()));
                    if let Some(page) = tab
                        .navigation
                        .as_ref()
                        .and_then(navigation::Navigation::state)
                        .and_then(|state| settings::startup_page(&state.url).ok())
                        .filter(|page| !page.is_empty())
                    {
                        request.push(' ');
                        request.push_str(&saved::encode(&page));
                    }
                    self.request(ctx, Job::CloneTab(tab.backend), tab.backend, &request);
                }
            }
            (1 | 2, Dialog::Close(id, name)) => {
                let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
                    return;
                };
                let tab = &self.tabs[index];
                if let Some(session) = &tab.session {
                    let profile = &session.token;
                    let cmd = if choice == 1 {
                        format!(
                            "save {} {profile} {}",
                            session.token,
                            saved::encode(name.trim())
                        )
                    } else {
                        format!("discard {} {profile}", session.token)
                    };
                    self.request(ctx, Job::Close(id, tab.backend), tab.backend, &cmd);
                } else if choice == 2 {
                    self.close_tab(index);
                } else {
                    self.error = Some("This tab is no longer running. Reopen a previously saved session from Home.".into());
                    self.quitting = false;
                }
            }
            (2, Dialog::Forget(backend, entry)) => self.request(
                ctx,
                Job::Forget(backend),
                backend,
                &format!("forget {}", entry.id),
            ),
            (2, Dialog::StopParked(backend, entry)) => self.request(
                ctx,
                Job::StopParked(backend),
                backend,
                &format!("stop-park {}", entry.token),
            ),
            (1, Dialog::StartupPage(backend, mut entry, input, _)) => {
                match settings::startup_page(&input) {
                    Ok(page) => {
                        let encoded = if page.is_empty() {
                            "-".into()
                        } else {
                            saved::encode(&page)
                        };
                        let cmd = format!("startup-page {} {encoded}", entry.id);
                        entry.startup_page = page;
                        self.request(ctx, Job::StartupPage(backend, entry), backend, &cmd);
                    }
                    Err(err) => {
                        self.dialog = Some(Dialog::StartupPage(
                            backend,
                            entry,
                            input,
                            Some(err.to_string()),
                        ));
                    }
                }
            }
            _ => self.quitting = false,
        }
    }
}

impl eframe::App for Rectangle {
    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let outer = ui.available_rect_before_wrap();
        if let Some(texture) = &self.marble {
            design::background(ui, texture, outer);
        }
        let inner = outer.shrink(10.0);
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            self.workspace(ui)
        });
    }
}

impl Rectangle {
    fn address_bar(
        &mut self,
        ui: &mut egui::Ui,
        width: f32,
        enabled: bool,
    ) -> (Option<bool>, bool) {
        let id = ui.make_persistent_id(("rectangle address", self.address_tab));
        let focused = ui.memory(|memory| memory.has_focus(id));
        let tab = self
            .address_tab
            .and_then(|id| self.tabs.iter().find(|tab| tab.id == id));
        let files = tab.is_some_and(|tab| tab.agent_view == AgentView::Files);
        let file_command = tab.filter(|_| files).and_then(|tab| {
            let session = tab.session.as_ref()?;
            Some(terminal::shell_command(
                tab.connection.host.trim(),
                &session.token,
                tab.explorer.path(),
            ))
        });
        let view = tab.and_then(Tab::terminal_view);
        let watch = tab
            .zip(view.as_deref())
            .and_then(|(tab, mode)| Some((tab, mode, tab.session.as_ref()?)));
        if self.terminal_directory.as_ref().map(|(id, _)| *id) != watch.map(|(tab, _, _)| tab.id) {
            self.terminal_directory = watch.map(|(tab, mode, session)| {
                let repaint = ui.ctx().clone();
                let directory = terminal::WorkingDirectory::start(
                    tab.connection.host.trim(),
                    tab.connection.key.as_deref(),
                    &session.token,
                    mode,
                    move || repaint.request_repaint(),
                )
                .ok();
                (tab.id, directory)
            });
        }
        let enabled = enabled
            && self.job.is_none()
            && (self.address_tab.is_some() || self.tabs.len() < MAX_TABS);
        if let Some((tab, _, session)) = watch {
            self.browser_address = self
                .terminal_directory
                .as_ref()
                .and_then(|(_, directory)| directory.as_ref())
                .and_then(terminal::WorkingDirectory::path)
                .map(|path| {
                    terminal::shell_command(tab.connection.host.trim(), &session.token, &path)
                })
                .unwrap_or_default();
        } else if !focused && self.address_tab.is_some() {
            self.browser_address = if files {
                file_command.clone()
            } else {
                tab.and_then(|tab| tab.navigation.as_ref())
                    .and_then(navigation::Navigation::state)
                    .map(|page| page.url)
            }
            .unwrap_or_default();
        }
        let linked = self.address_tab.is_some();
        let fill = if linked {
            ui.visuals().selection.bg_fill
        } else {
            ui.visuals().extreme_bg_color
        };
        let address = if linked {
            &mut self.browser_address
        } else {
            &mut self.new_tab_address
        };
        let submit = if focused && enabled && view.is_none() {
            ui.input(|input| {
                input.events.iter().find_map(|event| match event {
                    Event::Key {
                        key: Key::Enter,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } if !modifiers.alt && !modifiers.shift && !modifiers.mac_cmd => {
                        Some(modifiers.ctrl)
                    }
                    _ => None,
                })
            })
        } else {
            None
        };
        let has_address = !address.trim().is_empty()
            && !file_command
                .as_deref()
                .is_some_and(|command| command == address.trim());
        let editor = |ui: &mut egui::Ui, text: &mut dyn egui::TextBuffer| {
            ui.add_enabled(
                enabled,
                egui::TextEdit::singleline(text)
                    .id(id)
                    .background_color(fill)
                    .desired_width(width)
                    .margin(egui::Margin::symmetric(12, 7))
                    .hint_text(if view.is_some() {
                        "SSH command for this terminal"
                    } else if width < 190.0 {
                        "Search or URL"
                    } else {
                        "Search or enter a URL"
                    }),
            )
        };
        let response = if view.is_some() {
            let mut read_only = address.as_str();
            editor(ui, &mut read_only)
        } else {
            editor(ui, address)
        }
        .on_hover_text(if view.is_some() {
            "Copy this command into your local terminal to open a shell in this folder."
        } else if files {
            "Copy this command to open a shell here. To change folders, replace it with an absolute folder path and press Enter."
        } else if self.address_tab.is_some() {
            "Enter navigates this tab. Ctrl+Enter completes a site name."
        } else {
            "Enter opens a new tab. Ctrl+Enter completes a site name."
        });
        self.address_rect = Some(response.rect);
        (
            submit.filter(|_| has_address),
            focused || response.has_focus(),
        )
    }

    fn open_address(&mut self, ctx: &egui::Context, complete: bool) {
        let backend = if self.home {
            self.library_host
        } else {
            self.tabs
                .get(self.active)
                .map_or(self.library_host, |tab| tab.backend)
        };
        if self.connection(backend).host.trim().is_empty() {
            self.error = Some(format!("Configure {} in Settings first.", backend.name("")));
            return;
        }
        let input = if self.address_tab.is_some() {
            self.browser_address.trim()
        } else {
            self.new_tab_address.trim()
        };
        let tab_count = self.tabs.len();
        if input.len() > 2048 || input.chars().any(char::is_control) {
            self.error = Some("Enter a URL or search terms under 2,048 bytes.".into());
            return;
        }
        let wrapped = (complete && settings::is_bare_site_name(input))
            .then(|| format!("https://www.{input}.com"));
        let host = input.split(['/', '?', '#']).next().unwrap_or_default();
        let looks_like_url = input.contains("://")
            || (!input.chars().any(char::is_whitespace)
                && (host.contains('.')
                    || host.starts_with('[')
                    || host.contains(':')
                    || host == "localhost"));
        if let Some(tab) = self
            .address_tab
            .and_then(|id| self.tabs.iter_mut().find(|tab| tab.id == id))
        {
            if tab.terminal_view().is_some() {
                return;
            }
            if tab.agent_view == AgentView::Files {
                if let Some(session) = &tab.session {
                    self.error = tab
                        .explorer
                        .navigate(tab.connection.host.trim(), &session.token, input)
                        .err()
                        .map(|error| error.to_string());
                    if self.error.is_none() {
                        tab.last_input = Some(Instant::now());
                    }
                }
                return;
            }
            let page = if wrapped.is_some() || looks_like_url {
                settings::startup_page(wrapped.as_deref().unwrap_or(input))
            } else {
                let mut search = url::Url::parse("https://duckduckgo.com/").unwrap();
                search.query_pairs_mut().append_pair("q", input);
                Ok(search.into())
            };
            match page {
                Ok(page)
                    if tab.navigation.as_ref().is_some_and(|nav| {
                        nav.state().is_some() && nav.send(format!("navigate {page}"))
                    }) =>
                {
                    tab.last_input = Some(Instant::now());
                    self.error = None;
                }
                Ok(_) => self.error = Some(
                    "Browser controls are unavailable. Save and close this tab, then reopen it."
                        .into(),
                ),
                Err(err) => self.error = Some(err.to_string()),
            }
            return;
        }
        if wrapped.is_some() || looks_like_url {
            match settings::startup_page(wrapped.as_deref().unwrap_or(input)) {
                Ok(page) => self.open_tab(ctx, backend, None, Some(session::StartPage::Url(&page))),
                Err(err) => self.error = Some(err.to_string()),
            }
        } else {
            let query = input.split_whitespace().collect::<Vec<_>>().join(" ");
            self.open_tab(ctx, backend, None, Some(session::StartPage::Search(&query)));
        }
        if self.tabs.len() > tab_count {
            self.new_tab_address.clear();
        }
    }

    fn workspace(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.proton
            .tick(&ctx, self.host.trim(), self.identity.as_deref());
        self.primary_memory.tick(
            &ctx,
            self.host.trim(),
            self.identity.as_deref(),
            memory::HostMetric::Ram,
        );
        self.primary_disk.tick(
            &ctx,
            self.host.trim(),
            self.identity.as_deref(),
            memory::HostMetric::Disk,
        );
        self.poll_job(&ctx);
        if !self.quitting {
            self.pending_update = None;
            if let Some(candidate) = self.updater.poll() {
                self.pending_update = Some(candidate);
                self.settings_edit = None;
                self.dialog = None;
                self.quitting = true;
            }
        }
        if !ui.input(|i| i.viewport().close_requested()) && !self.quitting {
            self.poll_startup(&ctx);
        }
        if ui.input(|input| input.viewport().close_requested()) && !self.tabs.is_empty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.settings_edit = None;
            self.dialog = None;
            self.quitting = true;
        }
        if self.quitting {
            self.park_on_quit(&ctx);
            if self.quitting {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Parking tabs before closing…");
                });
                return;
            }
        }
        let restoring = !self.startup_parked.is_empty();
        let mut block_input = restoring
            || self.dialog.is_some()
            || self.settings_edit.is_some()
            || self.job.as_ref().is_some_and(|(job, _)| {
                matches!(job, Job::Close(..) | Job::IdleSave(..) | Job::CloneTab(..))
            });
        self.dialogs(&ctx);
        if self.quitting {
            return;
        }
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.poll(&ctx);
        }
        if !self.quitting {
            self.expire_idle_tabs(&ctx);
        }
        block_input |= self
            .job
            .as_ref()
            .is_some_and(|(job, _)| matches!(job, Job::IdleSave(..)));
        let (fullscreen, toggle_fullscreen) = ui.input(|i| {
            let viewport = i.viewport();
            (
                viewport.fullscreen.unwrap_or(false),
                i.events.iter().any(|event| {
                    matches!(event, Event::Key {
                    key: Key::F11, pressed: true, repeat: false, modifiers, ..
                } if modifiers.is_none())
                }),
            )
        });
        if toggle_fullscreen {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
        }
        let (new_shortcut, close_shortcut) = ui.input(|i| {
            let has = |wanted| {
                i.events.iter().any(|event| {
                    matches!(event,
                Event::Key { key, pressed: true, repeat: false, modifiers, .. }
                if *key == wanted && modifiers.ctrl && !modifiers.alt && !modifiers.shift)
                })
            };
            (has(Key::T), has(Key::W))
        });
        let close = (!block_input && !self.home && close_shortcut).then_some(self.active);
        let mut add = !block_input && new_shortcut;
        let mut home = false;
        let mut edit = false;
        let mut tab_action = None;
        let pane_headers = !self.home
            && self
                .tabs
                .get(self.active)
                .is_some_and(|tab| tab.snap.is_some());
        let mut dropped_tab = None;
        let mut address_submit = None;
        let mut address_focused = false;
        let mut monitor = false;
        let mut disk_monitor = false;
        if !block_input
            && let Some(pos) = ui.input(|input| {
                input
                    .pointer
                    .any_pressed()
                    .then(|| input.pointer.interact_pos())
                    .flatten()
            })
            && !self.address_rect.is_some_and(|rect| rect.contains(pos))
        {
            let tiled = self
                .tabs
                .get(self.active)
                .is_some_and(|tab| tab.snap.is_some());
            let header = self
                .address_headers
                .iter()
                .find_map(|(id, rect)| rect.contains(pos).then_some(*id));
            let target = header.or_else(|| {
                (!self.home)
                    .then(|| {
                        self.tabs.iter().enumerate().find(|(i, tab)| {
                            (if tiled {
                                tab.snap.is_some()
                            } else {
                                *i == self.active
                            }) && tab.session.is_some()
                                && tab.page_rect.is_some_and(|rect| rect.contains(pos))
                        })
                    })
                    .flatten()
                    .map(|(_, tab)| tab.id)
            });
            if self.address_tab != target {
                self.address_tab = target;
                self.browser_address.clear();
            }
        }
        self.address_headers.clear();
        if self.home
            || !self
                .tabs
                .get(self.active)
                .is_some_and(|tab| Some(tab.id) == self.address_tab && tab.session.is_some())
                && !ui.input(|input| input.pointer.any_pressed())
        {
            if self.address_tab.take().is_some() {
                self.browser_address.clear();
            }
        }
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 7.0);
        design::frame(design::SURFACE, 13, 8).show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Give the right-aligned row a toolbar-sized height, not the whole viewport.
            let row_size = Vec2::new(ui.available_width(), 34.0);
            ui.allocate_ui_with_layout(
                row_size,
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if !self.home
                        && self
                            .tabs
                            .get(self.active)
                            .is_some_and(|tab| tab.backend == Backend::Secondary)
                    {
                        ui.label(
                            RichText::new("Secondary server")
                                .small()
                                .strong()
                                .color(design::ACCENT),
                        );
                    } else {
                        self.proton.badge(ui, !self.host.trim().is_empty());
                    }
                    let compact = ui.available_width() < 560.0;
                    let full_label = if compact {
                        "⛶"
                    } else if fullscreen {
                        "Exit full screen"
                    } else {
                        "Full screen"
                    };
                    if ui
                        .button(full_label)
                        .on_hover_text("Toggle full screen (F11)")
                        .clicked()
                        && !toggle_fullscreen
                    {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        home = ui
                            .selectable_label(self.home || self.tabs.is_empty(), "Home")
                            .clicked();
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            let name = Backend::Primary.name(&self.host);
                            let configured = !self.host.trim().is_empty();
                            monitor = self.primary_memory.show(
                                ui,
                                name,
                                configured,
                                compact,
                                memory::HostMetric::Ram,
                            );
                            disk_monitor = self.primary_disk.show(
                                ui,
                                name,
                                configured,
                                compact,
                                memory::HostMetric::Disk,
                            );
                        });
                        edit = ui.button("Settings").clicked();
                        (address_submit, address_focused) =
                            self.address_bar(ui, ui.available_width(), !block_input);
                    });
                },
            );
        });
        {
            design::frame(Color32::from_rgb(17, 19, 24), 13, 6).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::horizontal()
                    .id_salt("rectangle tabs")
                    .max_height(36.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (index, tab) in self.tabs.iter().enumerate() {
                                if pane_headers && tab.snap.is_some() {
                                    continue;
                                }
                                let (action, drop, header) = Self::tab_header(
                                    ui,
                                    tab,
                                    !self.home && index == self.active,
                                    block_input,
                                    self.job.is_none(),
                                    &mut self.dragging_tab,
                                );
                                if tab.session.is_some() {
                                    self.address_headers.push((tab.id, header));
                                }
                                if let Some(action) = action {
                                    tab_action = Some((index, action));
                                }
                                if drop.is_some() {
                                    dropped_tab = drop;
                                    self.home = false;
                                }
                            }
                            add |= ui
                                .add_enabled(
                                    !block_input && self.tabs.len() < MAX_TABS,
                                    egui::Button::new(
                                        RichText::new("+ New Tab").color(design::ACCENT),
                                    ),
                                )
                                .on_hover_text("Start a separate browser container (Ctrl+T)")
                                .clicked();
                            if !self.home
                                && let Some(tab) = self.tabs.get(self.active)
                                && tab.texture.is_some()
                            {
                                ui.label(
                                    RichText::new(format!("{} × {}", tab.size[0], tab.size[1]))
                                        .color(MUTED)
                                        .small(),
                                )
                                .on_hover_text("Remote display resolution");
                            }
                        });
                    });
            });
        }
        if !block_input {
            if (monitor || disk_monitor)
                && let Err(err) =
                    session::monitor(self.host.trim(), self.identity.as_deref(), disk_monitor)
            {
                self.error = Some(format!(
                    "Could not open the monitor for {}: {err}",
                    Backend::Primary.name(&self.host)
                ));
            }
            if home {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.blur();
                }
                self.home = true;
                self.refresh(&ctx);
            }
            if edit {
                self.edit_settings();
            }
            if let Some((index, action)) = tab_action {
                self.apply_tab_action(&ctx, index, action);
            }
            if let Some(index) = close {
                self.ask_close(&ctx, index);
            }
            if add {
                self.dialog = Some(Dialog::NewTab(NewTab {
                    backend: self.library_host,
                    ..Default::default()
                }));
            }
        }
        if let Some(complete) = address_submit {
            self.open_address(&ctx, complete);
            if self.address_tab.is_some() {
                ctx.memory_mut(|memory| {
                    if let Some(id) = memory.focused() {
                        memory.surrender_focus(id);
                    }
                });
            }
            block_input = true;
        }
        if address_focused && let Some(tab) = self.tabs.get_mut(self.active) {
            tab.blur();
        }
        self.sync_audio(&ctx);
        if let Some(error) = &self.error {
            ui.label(RichText::new(error).color(Color32::LIGHT_RED));
        }
        if restoring {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Checking for parked tabs…");
            });
            return;
        } else if matches!(&self.job, Some((Job::CloneTab(..), _))) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Cloning tab… The source will resume when the copy is ready.");
            });
        } else if matches!(&self.job, Some((Job::PrepareAgentMedia(..), _))) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Preparing shared folders… Large folders may take a while.");
            });
        } else if self.job.is_some() && (self.home || block_input) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Working with the session host…");
            });
        }
        if self.home || self.tabs.is_empty() {
            self.home_library(ui, &ctx, block_input, None);
            return;
        }
        let area = ui.available_rect_before_wrap();
        if area.width() < 30.0 || area.height() < 30.0 {
            return;
        }
        if let Some((id, pos)) = dropped_tab.take().filter(|_| !block_input) {
            if area.contains(pos) {
                self.snap_tab(id, Snap::target(area, pos));
            }
            block_input = true;
        }
        let tiled = self
            .tabs
            .get(self.active)
            .is_some_and(|tab| tab.snap.is_some());
        let panes: Vec<_> = if tiled {
            self.tabs
                .iter()
                .enumerate()
                .filter_map(|(index, tab)| tab.snap.map(|snap| (index, snap.rect(area))))
                .collect()
        } else {
            vec![(self.active, area)]
        };
        let dragging = self.dragging_tab.is_some();
        if !block_input
            && !dragging
            && !address_focused
            && let Some(pos) = ui.input(|i| {
                i.pointer
                    .any_pressed()
                    .then(|| i.pointer.interact_pos())
                    .flatten()
            })
            && let Some((index, _)) = panes
                .iter()
                .find(|(_, pane)| pane.contains(pos) && (!tiled || pos.y >= pane.top() + 29.0))
            && *index != self.active
        {
            self.select_tab(*index);
        }
        let show_cursor =
            !block_input && !dragging && self.dialog.is_none() && self.settings_edit.is_none();
        let mut pane_action = None;
        let mut filled_vacancy = false;
        if tiled {
            let vacant: Vec<_> =
                Snap::vacant(self.tabs.iter().filter_map(|tab| tab.snap)).collect();
            if !vacant.is_empty() && self.library_loaded != Some(self.library_host) && !dragging {
                self.refresh(&ctx);
            }
            for slot in vacant {
                let pane = slot.rect(area);
                let count = self.tabs.len();
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .id_salt(("vacant tile", slot.code()))
                        .max_rect(pane),
                    |ui| {
                        ui.set_clip_rect(ui.clip_rect().intersect(pane));
                        ui.painter().rect_filled(pane, 12, design::SURFACE);
                        self.home_library(ui, &ctx, !show_cursor, Some(slot));
                    },
                );
                if self.tabs.len() != count {
                    filled_vacancy = true;
                    ctx.request_repaint();
                    break;
                }
            }
        }
        if filled_vacancy {
            return;
        }
        let mut edit_file = None;
        for (index, pane) in panes {
            let Some(tab) = self.tabs.get_mut(index) else {
                continue;
            };
            if index != self.active {
                tab.poll(&ctx);
            }
            ui.scope_builder(
                egui::UiBuilder::new().id_salt(tab.id).max_rect(pane),
                |ui| {
                    ui.set_clip_rect(ui.clip_rect().intersect(pane));
                    if tiled {
                        ui.painter().rect_filled(pane, 6, design::SURFACE);
                        let (action, drop, header) = Self::tab_header(
                            ui,
                            tab,
                            index == self.active,
                            block_input,
                            self.job.is_none(),
                            &mut self.dragging_tab,
                        );
                        if tab.session.is_some() {
                            self.address_headers.push((tab.id, header));
                        }
                        if let Some(action) = action {
                            pane_action = Some((index, action));
                        }
                        if drop.is_some() {
                            dropped_tab = drop;
                        }
                    }
                    if tab.agent_view == AgentView::Files
                        && let Some(session) = &tab.session
                        && tab.ran
                    {
                        tab.page_rect = Some(pane);
                        let target = ctx
                            .pointer_latest_pos()
                            .map_or(index == self.active, |pos| pane.contains(pos));
                        let (activity, editor) = tab.explorer.show(
                            ui,
                            &tab.connection,
                            &session.token,
                            tab.agent_parent_title.as_deref().unwrap_or(&tab.title),
                            !block_input,
                            target,
                            pane,
                        );
                        if activity {
                            tab.last_input = Some(Instant::now());
                        }
                        if let Some(editor) = editor {
                            edit_file = Some((index, editor));
                        }
                        return;
                    }
                    Self::browser_pane(
                        ui,
                        &ctx,
                        tab,
                        !show_cursor,
                        show_cursor && !address_focused && index == self.active,
                    );
                },
            );
        }
        if !block_input {
            if let Some((index, editor)) = edit_file {
                self.open_file_editor(&ctx, index, editor);
            }
            if let Some((index, action)) = pane_action {
                self.apply_tab_action(&ctx, index, action);
            }
            if let Some((id, pos)) = dropped_tab
                && area.contains(pos)
            {
                self.snap_tab(id, Snap::target(area, pos));
            }
        }
        if dragging && let Some(pos) = ctx.pointer_hover_pos() {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("Rectangle snap preview"),
            ));
            if let Some(slot) = Snap::target(area, pos) {
                let preview = slot.rect(area);
                painter.rect(
                    preview,
                    8,
                    Color32::from_rgba_unmultiplied(52, 145, 197, 80),
                    egui::Stroke::new(2.0, design::ACCENT),
                    egui::StrokeKind::Inside,
                );
                painter.text(
                    preview.center(),
                    egui::Align2::CENTER_CENTER,
                    slot.label(),
                    egui::FontId::proportional(18.0),
                    INK,
                );
            }
        }
    }

    fn home_library(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        block_input: bool,
        slot: Option<Snap>,
    ) {
        egui::ScrollArea::vertical()
            .id_salt(("rectangle home", slot.map(Snap::code)))
            .max_height(ui.available_height())
            .show(ui, |ui| {
                design::frame(
                    Color32::from_rgb(16, 18, 23),
                    18,
                    if slot.is_some() { 9 } else { 18 },
                )
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new("SESSION LIBRARY")
                            .small()
                            .color(design::ACCENT)
                            .strong(),
                    );
                    ui.label(
                        RichText::new("Saved sessions")
                            .size(if slot.is_some() { 22.0 } else { 30.0 })
                            .strong(),
                    );
                    ui.label(
                        RichText::new("Reopen saved Browser profiles and Agent workspaces.")
                            .color(MUTED),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        for backend in [Backend::Primary, Backend::Secondary] {
                            if ui
                                .add_enabled(
                                    self.job.is_none() && !block_input,
                                    egui::Button::selectable(
                                        self.library_host == backend,
                                        backend.name(&self.host),
                                    ),
                                )
                                .clicked()
                            {
                                self.library_host = backend;
                                self.parked.clear();
                                self.refresh(ctx);
                            }
                        }
                        if ui
                            .add_enabled(!block_input, egui::Button::new("Connect"))
                            .clicked()
                        {
                            self.dialog = Some(Dialog::NewTab(NewTab {
                                backend: self.library_host,
                                slot,
                                ..Default::default()
                            }));
                        }
                    });
                    let backend = self.library_host;
                    let connection = self.connection(backend);
                    let entries = match backend {
                        Backend::Primary => self.entries.clone(),
                        Backend::Secondary => self.secondary_entries.clone(),
                    };
                    if connection.host.trim().is_empty() {
                        ui.label("Open Settings to configure this host.");
                    } else {
                        ui.horizontal(|ui| {
                            ui.label(&connection.host);
                            if ui
                                .add_enabled(
                                    self.job.is_none() && !block_input,
                                    egui::Button::new("Refresh"),
                                )
                                .clicked()
                            {
                                self.refresh(ctx);
                            }
                        });
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        let parked: Vec<_> = self.parked.iter().cloned().collect();
                        if !parked.is_empty() {
                            ctx.request_repaint_after(Duration::from_secs(15));
                            ui.heading("Parked sessions");
                            for entry in parked {
                                ui.push_id(&entry.token, |ui| {
                                    ui.horizontal(|ui| {
                                        let enabled = !block_input && self.job.is_none();
                                        if ui
                                            .add_enabled(
                                                enabled && self.tabs.len() < MAX_TABS,
                                                egui::Button::new("Resume"),
                                            )
                                            .clicked()
                                        {
                                            let count = self.tabs.len();
                                            self.resume_parked(ctx, backend, entry.clone());
                                            self.place_new_tab(count, slot);
                                        }
                                        if ui
                                            .add_enabled(enabled, egui::Button::new("Stop"))
                                            .clicked()
                                        {
                                            self.dialog =
                                                Some(Dialog::StopParked(backend, entry.clone()));
                                        }
                                        let status = if entry.always_on {
                                            "Always on".into()
                                        } else if entry.expires <= now {
                                            "Still running after expiry".into()
                                        } else {
                                            format!(
                                                "{} min left",
                                                (entry.expires - now).div_ceil(60)
                                            )
                                        };
                                        ui.label(format!("{} · {status}", entry.title));
                                    });
                                });
                            }
                            ui.separator();
                        }
                        let mut open = None;
                        let mut forget = None;
                        let mut edit_startup = None;
                        for (is_agent, heading) in [(false, "Browsers"), (true, "Agents")] {
                            ui.heading(heading);
                            if !entries
                                .iter()
                                .any(|entry| entry.agent_port.is_some() == is_agent)
                            {
                                ui.label(if is_agent {
                                    "No saved Agent sessions."
                                } else {
                                    "No saved Browser sessions."
                                });
                            }
                            for entry in entries
                                .iter()
                                .filter(|entry| entry.agent_port.is_some() == is_agent)
                            {
                                ui.push_id(&entry.id, |ui| {
                                    design::frame(Color32::from_rgb(26, 28, 34), 8, 4).show(
                                        ui,
                                        |ui| {
                                            ui.set_width(ui.available_width());
                                            let row = Vec2::new(ui.available_width(), 26.0);
                                            ui.allocate_ui_with_layout(
                                                row,
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.spacing_mut().interact_size.y = 24.0;
                                                    ui.spacing_mut().button_padding.y = 3.0;
                                                    ui.add_enabled_ui(
                                                        !block_input && self.job.is_none(),
                                                        |ui| {
                                                            ui.menu_button("⋯", |ui| {
                                                                if ui.button("Delete").clicked() {
                                                                    forget = Some(entry.clone());
                                                                    ui.close();
                                                                }
                                                            })
                                                            .response
                                                            .on_hover_text("More session actions");
                                                        },
                                                    );
                                                    if !is_agent
                                                        && ui
                                                            .add_enabled(
                                                                !block_input && self.job.is_none(),
                                                                egui::Button::new(
                                                                    if slot.is_some()
                                                                        && ui.available_width()
                                                                            < 380.0
                                                                    {
                                                                        "Startup page"
                                                                    } else {
                                                                        "Edit startup page"
                                                                    },
                                                                ),
                                                            )
                                                            .clicked()
                                                    {
                                                        edit_startup = Some(entry.clone());
                                                    }
                                                    ui.with_layout(
                                                        egui::Layout::left_to_right(
                                                            egui::Align::Center,
                                                        ),
                                                        |ui| {
                                                            if ui
                                                                .add_enabled(
                                                                    !block_input
                                                                        && self.job.is_none()
                                                                        && self.tabs.len()
                                                                            < MAX_TABS,
                                                                    egui::Button::new("Open"),
                                                                )
                                                                .clicked()
                                                            {
                                                                open = Some(entry.clone());
                                                            }
                                                            ui.add(
                                                                egui::Label::new(
                                                                    RichText::new(&entry.title)
                                                                        .strong(),
                                                                )
                                                                .truncate(),
                                                            )
                                                            .on_hover_text(&entry.title);
                                                            if let Some(port) = entry.agent_port {
                                                                ui.label(
                                                                    RichText::new(format!(
                                                                        "Port {port}"
                                                                    ))
                                                                    .small()
                                                                    .color(MUTED),
                                                                );
                                                            }
                                                        },
                                                    );
                                                },
                                            )
                                        },
                                    );
                                });
                            }
                        }
                        if let Some(entry) = open {
                            self.request(
                                ctx,
                                Job::OpenSaved(backend, entry, slot),
                                backend,
                                "parked-list",
                            );
                        }
                        if let Some(entry) = forget {
                            self.dialog = Some(Dialog::Forget(backend, entry));
                        }
                        if let Some(entry) = edit_startup {
                            let page = entry.startup_page.clone();
                            self.dialog = Some(Dialog::StartupPage(backend, entry, page, None));
                        }
                    }
                });
            });
    }

    fn tab_header(
        ui: &mut egui::Ui,
        tab: &Tab,
        selected: bool,
        block_input: bool,
        ready: bool,
        dragging_tab: &mut Option<u64>,
    ) -> (Option<TabAction>, Option<(u64, Pos2)>, Rect) {
        let host = tab.backend.name(&tab.connection.host);
        let mut label = if let Some(parent) = &tab.agent_parent_title {
            format!("{host} - {parent} - {}", tab.title)
        } else {
            format!("{host} · {}", tab.title)
        };
        if tab.muted {
            label.push_str(" (muted)");
        }
        let font = egui::TextStyle::Button.resolve(ui.style());
        let text = ui
            .painter()
            .layout_no_wrap(label.clone(), font.clone(), INK);
        let browser = tab.agent_port.is_none();
        let width = (text.size().x + if browser { 118.0 } else { 46.0 }).min(288.0);
        let width = if ui.layout().is_horizontal() {
            width
        } else {
            width.min(ui.available_width().max(45.0))
        };
        let (_, outer) = ui.allocate_space(Vec2::new(width, 29.0));
        let close_rect = Rect::from_center_size(
            Pos2::new(outer.right() - 15.0, outer.center().y),
            Vec2::splat(22.0),
        );
        let controls_width = if browser && width >= 118.0 { 69.0 } else { 0.0 };
        let name_rect = Rect::from_min_max(
            outer.min + Vec2::new(controls_width, 0.0),
            Pos2::new(close_rect.left() - 3.0, outer.bottom()),
        );
        let (usage, ram) = tab.memory_badge(ui.ctx());
        let response = ui
            .interact(
                name_rect,
                ui.make_persistent_id((tab.id, "tab name")),
                Sense::click_and_drag(),
            )
            .on_hover_text(format!(
                "{label}\n{ram}\nDrag to snap. Right-click for tab options."
            ));
        let close = ui
            .interact(
                close_rect,
                ui.make_persistent_id((tab.id, "tab close")),
                Sense::click(),
            )
            .on_hover_text(format!(
                "{}\n{ram}",
                if tab.profile.is_some() {
                    "Save and close this session"
                } else {
                    "Close this tab"
                }
            ));
        let visuals = ui.visuals();
        let fill = if selected {
            visuals.selection.bg_fill
        } else if response.hovered() || close.hovered() {
            visuals.widgets.hovered.weak_bg_fill
        } else {
            visuals.widgets.inactive.weak_bg_fill
        };
        ui.painter().rect(
            outer,
            8,
            fill,
            egui::Stroke::new(1.0, design::EDGE),
            egui::StrokeKind::Inside,
        );
        if let Some((ratio, color)) = usage.filter(|(ratio, _)| *ratio > 0.0) {
            let inner = outer.shrink(1.0);
            let clip = Rect::from_min_max(
                inner.min,
                Pos2::new(inner.left() + inner.width() * ratio, inner.bottom()),
            );
            ui.painter()
                .with_clip_rect(ui.clip_rect().intersect(clip))
                .rect_filled(inner, 7, fill.lerp_to_gamma(color, 0.23));
        }
        if close.hovered() && !block_input {
            ui.painter()
                .rect_filled(close_rect, 5, visuals.widgets.active.weak_bg_fill);
        }
        ui.painter().text(
            close_rect.center(),
            egui::Align2::CENTER_CENTER,
            "\u{d7}",
            egui::FontId::proportional(15.0),
            INK,
        );
        let text_rect = Rect::from_min_max(name_rect.min + Vec2::new(9.0, 0.0), name_rect.max);
        let text = if text.size().x > text_rect.width() {
            let mut job = egui::text::LayoutJob::simple_singleline(label, font, INK);
            job.wrap = egui::text::TextWrapping::truncate_at_width(text_rect.width());
            ui.painter().layout_job(job)
        } else {
            text
        };
        let painter = ui
            .painter()
            .with_clip_rect(ui.clip_rect().intersect(text_rect));
        painter.galley(
            Pos2::new(text_rect.left(), text_rect.center().y - text.size().y * 0.5),
            text,
            INK,
        );
        let mut action = None;
        if controls_width > 0.0 {
            let state = tab
                .navigation
                .as_ref()
                .and_then(navigation::Navigation::state);
            for (i, (symbol, label, command, enabled)) in [
                (
                    "‹",
                    "Back",
                    "back",
                    state.as_ref().is_some_and(|page| page.back),
                ),
                (
                    "›",
                    "Forward",
                    "forward",
                    state.as_ref().is_some_and(|page| page.forward),
                ),
                ("↻", "Refresh", "reload", tab.session.is_some()),
            ]
            .into_iter()
            .enumerate()
            {
                let area = Rect::from_center_size(
                    Pos2::new(outer.left() + 13.0 + 22.0 * i as f32, outer.center().y),
                    Vec2::splat(21.0),
                );
                let button = ui
                    .interact(
                        area,
                        ui.make_persistent_id((tab.id, command)),
                        Sense::click(),
                    )
                    .on_hover_text(format!("{label}\n{ram}"));
                if enabled && button.hovered() && !block_input {
                    ui.painter()
                        .rect_filled(area, 5, visuals.widgets.active.weak_bg_fill);
                }
                ui.painter().text(
                    area.center(),
                    egui::Align2::CENTER_CENTER,
                    symbol,
                    egui::FontId::proportional(17.0),
                    if enabled { INK } else { MUTED },
                );
                if button.clicked() {
                    action = Some(if enabled {
                        TabAction::Navigate(command)
                    } else {
                        TabAction::Select
                    });
                }
            }
        }
        let mut dropped_tab = None;
        if !block_input && response.drag_started_by(PointerButton::Primary) {
            *dragging_tab = Some(tab.id);
        }
        if response.drag_stopped() && *dragging_tab == Some(tab.id) {
            *dragging_tab = None;
            if let Some(pos) = response
                .interact_pointer_pos()
                .or_else(|| ui.ctx().pointer_latest_pos())
            {
                dropped_tab = Some((tab.id, pos));
            }
        }
        if response.clicked() {
            action = Some(TabAction::Select);
        }
        if response.double_clicked() {
            action = Some(TabAction::Rename);
        }
        if !block_input {
            response.context_menu(|ui| {
                if tab.agent_port.is_some() {
                    for kind in AgentView::ALL
                        .into_iter()
                        .map(|kind| {
                            if tab.codex && kind == AgentView::Freebuff {
                                AgentView::Codex
                            } else {
                                kind
                            }
                        })
                        .filter(|kind| *kind != tab.agent_view)
                    {
                        if ui
                            .add_enabled(
                                tab.ran && tab.session.is_some() && ready,
                                egui::Button::new(format!("Open {}", kind.label())),
                            )
                            .clicked()
                        {
                            action = Some(TabAction::OpenAgentView(kind));
                            ui.close();
                        }
                    }
                    ui.separator();
                    let mut always_on = tab.always_on;
                    if ui
                        .add_enabled(
                            tab.ran && tab.session.is_some() && ready,
                            egui::Checkbox::new(&mut always_on, "Always on"),
                        )
                        .on_hover_text(
                            "Keep the Agent running while idle or after Rectangle closes.",
                        )
                        .changed()
                    {
                        action = Some(TabAction::ToggleAlwaysOn);
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            tab.session.is_some() && ready,
                            egui::Button::new("Open preview in Brave"),
                        )
                        .clicked()
                    {
                        action = Some(TabAction::Preview);
                        ui.close();
                    }
                    for info in [AgentInfo::Ports, AgentInfo::Health] {
                        if ui
                            .add_enabled(
                                tab.session.is_some() && ready,
                                egui::Button::new(info.label()),
                            )
                            .clicked()
                        {
                            action = Some(TabAction::Info(info));
                            ui.close();
                        }
                    }
                    if ui
                        .add_enabled(
                            tab.ran && tab.session.is_some() && ready,
                            egui::Button::new("Memory limit…"),
                        )
                        .clicked()
                    {
                        action = Some(TabAction::AgentMemory);
                        ui.close();
                    }
                    ui.separator();
                }
                ui.menu_button("Snap tab", |ui| {
                    for slot in Snap::ALL {
                        if ui.button(slot.label()).clicked() {
                            action = Some(TabAction::Snap(Some(slot)));
                            ui.close();
                        }
                    }
                    if ui.button("Unsnap").clicked() {
                        action = Some(TabAction::Snap(None));
                        ui.close();
                    }
                });
                if ui
                    .add_enabled(
                        tab.ran && tab.session.is_some() && ready,
                        egui::Button::new("Park tab"),
                    )
                    .clicked()
                {
                    action = Some(TabAction::Park);
                    ui.close();
                }
                if tab.agent_port.is_some() {
                    ui.menu_button("Resolution", |ui| {
                        for (resolution, label) in AgentResolution::OPTIONS {
                            let response =
                                ui.selectable_label(tab.agent_resolution == resolution, label);
                            let response = match resolution {
                                AgentResolution::MatchTile => {
                                    response.on_hover_text("Follow the size of this tile.")
                                }
                                AgentResolution::MatchHost => response.on_hover_text(
                                    "Use the resolution of the monitor showing Rectangle.",
                                ),
                                _ => response,
                            };
                            if response.clicked() {
                                action = Some(TabAction::AgentResolution(resolution));
                                ui.close();
                            }
                        }
                    });
                } else {
                    if ui
                        .selectable_label(tab.video_mode, "Smooth video (up to 1080p)")
                        .on_hover_text(
                            "Stream fewer display pixels for smoother motion in large panes.",
                        )
                        .clicked()
                    {
                        action = Some(TabAction::ToggleVideoMode);
                        ui.close();
                    }
                    let mut volume = tab.volume.unwrap_or(100);
                    if ui
                        .add(
                            egui::Slider::new(&mut volume, 0..=100)
                                .text("Volume")
                                .suffix("%"),
                        )
                        .changed()
                    {
                        action = Some(TabAction::Volume(volume));
                    }
                    if ui
                        .button(if tab.muted { "Unmute tab" } else { "Mute tab" })
                        .clicked()
                    {
                        action = Some(TabAction::Mute);
                        ui.close();
                    }
                }
                if tab.agent_owner.is_none()
                    && tab.session.is_some()
                    && ui
                        .add_enabled(ready, egui::Button::new("Clone tab…"))
                        .clicked()
                {
                    action = Some(TabAction::Clone);
                    ui.close();
                }
                if ui.button("Rename tab").clicked() {
                    action = Some(TabAction::Rename);
                    ui.close();
                }
                let audio_error = tab.audio_error.clone().or_else(|| {
                    tab.audio
                        .as_ref()
                        .and_then(|audio| match &*audio.status.lock().unwrap() {
                            audio::Status::Failed(error) => Some(error.clone()),
                            _ => None,
                        })
                });
                if let Some(error) = audio_error {
                    ui.label(format!("Audio: {error}"));
                }
            });
        }
        if block_input {
            action = None;
        } else if close.clicked() {
            action = Some(TabAction::Close);
        }
        (action, dropped_tab, outer.intersect(ui.clip_rect()))
    }

    fn apply_tab_action(&mut self, ctx: &egui::Context, index: usize, action: TabAction) {
        if matches!(action, TabAction::Select | TabAction::Navigate(_)) && index < self.tabs.len() {
            if self.home || self.active != index {
                self.select_tab(index);
            }
            let tab = &self.tabs[index];
            let target = tab.session.is_some().then_some(tab.id);
            if self.address_tab != target {
                self.address_tab = target;
                self.browser_address.clear();
            }
            ctx.request_repaint();
        }
        let index = if matches!(action, TabAction::ToggleAlwaysOn) && index < self.tabs.len() {
            self.agent_owner_index(index)
        } else {
            index
        };
        let has_tab_slot = self.tabs.len() < MAX_TABS;
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        match action {
            TabAction::Select => {}
            TabAction::Close => self.ask_close(ctx, index),
            TabAction::Rename => {
                tab.blur();
                self.dialog = Some(Dialog::Rename(tab.id, tab.title.clone()));
            }
            TabAction::Clone => {
                if !has_tab_slot {
                    self.error = Some("Close a tab before cloning.".into());
                } else if tab.agent_owner.is_none() && tab.session.is_some() {
                    self.dialog = Some(Dialog::Clone(tab.id, String::new()));
                }
            }
            TabAction::Snap(snap) => {
                let id = tab.id;
                self.snap_tab(id, snap);
            }
            TabAction::Park => self.park_tab(ctx, index),
            TabAction::Mute => {
                tab.muted = !tab.muted;
                tab.audio_error = None;
            }
            TabAction::Volume(volume) => {
                tab.volume = Some(volume);
                if let Some(audio) = &tab.audio {
                    audio.set_volume(volume);
                }
                if let Some(id) = tab
                    .profile
                    .as_deref()
                    .or_else(|| tab.session.as_ref().map(|session| session.token.as_str()))
                    && let Err(error) =
                        settings::save_tab_volume(tab.connection.host.trim(), id, volume)
                {
                    self.error = Some(format!("Could not save tab volume: {error}"));
                }
            }
            TabAction::Navigate(command) => {
                let sent = tab
                    .navigation
                    .as_ref()
                    .is_some_and(|nav| nav.state().is_some() && nav.send(command.into()));
                if !sent
                    && command == "reload"
                    && let Some(session) = &tab.session
                {
                    session.send(rfb::key(0xffc2, true), false);
                    session.send(rfb::key(0xffc2, false), false);
                }
                tab.last_input = Some(Instant::now());
            }
            TabAction::ToggleVideoMode => {
                let enabled = !tab.video_mode;
                if tab
                    .session
                    .as_ref()
                    .is_some_and(|session| session.send(rfb::encodings(enabled), false))
                {
                    tab.video_mode = enabled;
                }
            }
            TabAction::AgentResolution(resolution) => {
                tab.agent_resolution = resolution;
                ctx.request_repaint();
            }
            TabAction::OpenAgentView(kind) => self.open_agent_view(ctx, index, kind),
            TabAction::ToggleAlwaysOn => {
                if let Some(session) = &tab.session {
                    let enabled = !tab.always_on;
                    let command = if enabled {
                        format!(
                            "always-on {} {} {}",
                            session.token,
                            saved::encode(&tab.title),
                            tab.snap.map_or("-", Snap::code),
                        )
                    } else {
                        format!("always-on {} off", session.token)
                    };
                    let (id, backend) = (tab.id, tab.backend);
                    self.request(ctx, Job::AlwaysOn(id, enabled), backend, &command);
                }
            }
            TabAction::Preview | TabAction::Info(_) | TabAction::AgentMemory => {
                let Some(session) = &tab.session else {
                    return;
                };
                let (job, command) = match action {
                    TabAction::Preview => (Job::OpenPreview(tab.id), "agent-ports"),
                    TabAction::AgentMemory => (Job::AgentMemory(tab.id), "agent-memory"),
                    TabAction::Info(info) => {
                        (Job::AgentInfo(info, tab.title.clone()), info.command())
                    }
                    _ => unreachable!(),
                };
                let backend = tab.backend;
                let request = format!("{command} {}", session.token);
                self.request(ctx, job, backend, &request);
            }
        }
    }

    fn browser_pane(
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        tab: &mut Tab,
        block_input: bool,
        interactive: bool,
    ) {
        tab.page_rect = None;
        if interactive
            && tab.texture.is_some()
            && tab.clipboard.is_none()
            && tab.clipboard_error.is_none()
            && let Some(session) = &tab.session
        {
            let repaint = ctx.clone();
            let view = tab.agent_owner.map(|_| {
                tab.file_editor.map_or_else(
                    || tab.agent_view.command().to_owned(),
                    |display| format!("edit{display}"),
                )
            });
            match clipboard::Clipboard::start_view(
                tab.connection.host.trim(),
                tab.connection.key.as_deref(),
                &session.token,
                view.as_deref(),
                move || repaint.request_repaint(),
            ) {
                Ok(clipboard) => tab.clipboard = Some(clipboard),
                Err(_) => tab.clipboard_error = Some("Could not open the clipboard connection."),
            }
        }
        if let Some(error) = tab.clipboard_error {
            ui.label(RichText::new(error).color(MUTED).small());
        }
        if let Some(port) = tab.agent_port {
            ui.label(
                RichText::new(format!(
                    "Agent · {} · TCP {port}",
                    tab.backend.name(&tab.connection.host)
                ))
                .small()
                .color(MUTED),
            );
        }
        if let Some(texture) = &tab.texture {
            let available = ui.available_size().max(Vec2::ZERO);
            if available.x < 1.0 || available.y < 1.0 {
                return;
            }
            let (pane, _) = ui.allocate_exact_size(available, Sense::hover());
            let dpi = ctx.pixels_per_point();
            let tile = rfb::Resolution::fit(
                ((pane.right() * dpi).round() - (pane.left() * dpi).round()).max(320.0),
                ((pane.bottom() * dpi).round() - (pane.top() * dpi).round()).max(200.0),
            );
            let desired = if tab.agent_port.is_some() {
                let host = ctx
                    .input(|input| input.viewport().monitor_size)
                    .map(|size| size * dpi);
                tab.agent_resolution.target(tile, host)
            } else if tab.video_mode {
                tile.video_size()
            } else {
                tile
            };
            let now = Instant::now();
            if tab.resize_target.is_none_or(|(size, _)| size != desired) {
                tab.resize_target = Some((desired, now));
                tab.resize_sent = None;
            }
            let can_resize = tab.session.as_ref().is_some_and(Session::supports_resize);
            if can_resize && [desired.0, desired.1] != tab.size && tab.resize_sent != Some(desired)
            {
                let delay = Duration::from_millis(150);
                let elapsed = now - tab.resize_target.unwrap().1;
                if elapsed < delay {
                    ctx.request_repaint_after(delay - elapsed);
                } else if tab
                    .session
                    .as_ref()
                    .is_some_and(|session| session.resize(desired))
                {
                    tab.resize_sent = Some(desired);
                } else {
                    ctx.request_repaint_after(delay);
                }
            }
            // A resize is asynchronous, and older parked desktops can't resize.
            let rect = design::fit_desktop(pane, tab.size);
            tab.page_rect = Some(rect);
            design::display(ui, texture.id(), rect);
            if !can_resize
                && [desired.0, desired.1] != tab.size
                && tab
                    .started
                    .is_some_and(|started| started.elapsed() >= Duration::from_secs(2))
            {
                let notice = if tab.profile.is_some() {
                    "Fixed-size desktop · Close this tab and open it from Saved sessions to resize."
                } else {
                    "Fixed-size desktop · Start a fresh tab to resize."
                };
                ui.painter()
                    .with_clip_rect(pane.intersect(ui.clip_rect()))
                    .text(
                        pane.center_top() + Vec2::new(0.0, 8.0),
                        egui::Align2::CENTER_TOP,
                        notice,
                        egui::FontId::proportional(12.0),
                        Color32::GOLD,
                    );
            }
            let browser = ui.interact(rect, ui.id().with("remote cursor"), Sense::hover());
            if !block_input
                && browser.contains_pointer()
                && let Some(pos) = ctx.pointer_hover_pos().filter(|pos| rect.contains(*pos))
            {
                ctx.set_cursor_icon(egui::CursorIcon::None);
                design::remote_cursor(ui, rect, pos);
            }
            let events = ui.input(|input| {
                input
                    .events
                    .iter()
                    .filter(|event| {
                        !matches!(event,
                Event::Key { key: Key::T | Key::W, modifiers, .. }
                if modifiers.ctrl && !modifiers.alt && !modifiers.shift)
                    })
                    .cloned()
                    .collect()
            });
            if !block_input
                && interactive
                && (!tab.input(rect, events) || !tab.poll_clipboard(ctx, ui.input(|i| i.focused)))
            {
                tab.close();
                tab.error = Some("The remote input channel stopped responding.".into());
            }
        } else if tab.session.is_some() {
            design::loading(
                ui,
                &tab.title,
                tab.started.map_or(0.0, |t| t.elapsed().as_secs_f32()),
            );
        } else if let Some(error) = &tab.error {
            design::frame(design::SURFACE, 14, 15).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if tab.idle_stopped {
                    ui.heading(error);
                    ui.label(if tab.profile.is_some() {
                        "Your session was saved. You can reopen it from Home."
                    } else {
                        "Open a new tab to continue browsing."
                    });
                } else {
                    ui.heading(if tab.ran {
                        "Session disconnected"
                    } else {
                        "Session could not start"
                    });
                    ui.label(RichText::new(error).color(Color32::LIGHT_RED));
                }
            });
        }
    }
}

fn keysym(key: Key, modified: bool) -> Option<u32> {
    Some(match key {
        Key::Backspace => 0xff08,
        Key::Tab => 0xff09,
        Key::Enter => 0xff0d,
        Key::Escape => 0xff1b,
        Key::Delete => 0xffff,
        Key::Insert => 0xff63,
        Key::Home => 0xff50,
        Key::End => 0xff57,
        Key::ArrowLeft => 0xff51,
        Key::ArrowUp => 0xff52,
        Key::ArrowRight => 0xff53,
        Key::ArrowDown => 0xff54,
        Key::PageUp => 0xff55,
        Key::PageDown => 0xff56,
        Key::F5 => 0xffc2,
        other if modified => {
            let name = other.name().as_bytes();
            if name.len() != 1 || !name[0].is_ascii_alphanumeric() {
                return None;
            }
            u32::from(name[0].to_ascii_lowercase())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_resolution_selects_fixed_modes_or_tracks_tile_and_monitor_pixels() {
        use rfb::Resolution;
        let tile = Resolution(960, 540);
        let host = Some(Vec2::new(2560.0, 1440.0));
        for (mode, expected) in [
            (AgentResolution::Uhd, Resolution(3840, 2160)),
            (AgentResolution::Qhd, Resolution(2560, 1440)),
            (AgentResolution::FullHd, Resolution(1920, 1080)),
            (AgentResolution::Dci2k, Resolution(2048, 1080)),
            (AgentResolution::OneK, Resolution(1024, 576)),
            (AgentResolution::MatchHost, Resolution(2560, 1440)),
            (AgentResolution::MatchTile, tile),
        ] {
            assert_eq!(mode.target(tile, host), expected);
        }
        assert_eq!(AgentResolution::MatchHost.target(tile, None), tile);
        assert_eq!(
            AgentResolution::MatchHost.target(tile, Some(Vec2::new(7680.0, 4320.0))),
            Resolution(3965, 2230)
        );
    }

    #[test]
    fn tiled_headers_follow_their_panes_and_close_the_right_tab() {
        let ctx = egui::Context::default();
        design::theme(&ctx);
        let mut app = Rectangle {
            tabs: [
                (1, "Left", Snap::Left),
                (2, "Upper", Snap::TopRight),
                (3, "Lower", Snap::BottomRight),
            ]
            .into_iter()
            .map(|(id, title, snap)| Tab {
                id,
                title: title.into(),
                snap: Some(snap),
                connection: Connection {
                    host: "Emerald@example.test".into(),
                    key: None,
                },
                ..Default::default()
            })
            .collect(),
            ..Default::default()
        };
        let mut frame = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0))),
                    events,
                    ..Default::default()
                },
                |ui| app.workspace(ui),
            );
            output.textures_delta.clear();
            output
        };
        let output = frame(Vec::new());
        let text: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some((text.galley.job.text.as_str(), text.pos)),
                _ => None,
            })
            .collect();
        let locate = |name| text.iter().find(|(label, _)| *label == name).unwrap().1;
        let left = locate("Emerald · Left");
        let upper = locate("Emerald · Upper");
        let lower = locate("Emerald · Lower");
        assert!(left.x + 400.0 < upper.x);
        assert_eq!(left.y, upper.y);
        assert_eq!(upper.x, lower.x);
        assert!(lower.y > upper.y + 150.0);
        let button = text
            .iter()
            .find(|(label, pos)| *label == "×" && pos.x > lower.x && (pos.y - lower.y).abs() < 6.0)
            .unwrap()
            .1;
        let pos = button + Vec2::new(5.0, 8.0);
        let event = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![Event::PointerMoved(pos), event(true)]);
        frame(vec![event(false)]);
        assert!(matches!(app.dialog, Some(Dialog::Close(3, _))));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires Primary and a Windows audio device"]
    fn plays_two_tabs_on_home_and_mutes_only_one() {
        let mut app = Rectangle {
            host: std::env::var("RECTANGLE_SMOKE_HOST").unwrap(),
            identity: std::env::var_os("RECTANGLE_SMOKE_IDENTITY").map(std::path::PathBuf::from),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        app.add_tab(&ctx);
        app.add_tab(&ctx);
        assert_eq!(app.tabs.len(), 2, "{:?}", app.error);
        app.home = true;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            app.sync_audio(&ctx);
            if app.tabs.iter().all(|tab| {
                tab.audio.as_ref().is_some_and(|audio| {
                    matches!(*audio.status.lock().unwrap(), audio::Status::Playing)
                })
            }) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "both audio streams did not start"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        let second_stream = app.tabs[1].audio.as_ref().unwrap().status.clone();
        app.tabs[0].muted = true;
        app.sync_audio(&ctx);
        assert!(app.tabs[0].audio.is_none());
        assert!(std::sync::Arc::ptr_eq(
            &second_stream,
            &app.tabs[1].audio.as_ref().unwrap().status,
        ));
        app.tabs[0].muted = false;
        app.sync_audio(&ctx);
        assert!(app.tabs[0].audio.is_some());
    }

    #[test]
    fn idle_expiry_leaves_a_stopped_tab_and_keeps_recently_used_tabs() {
        let now = Instant::now();
        let mut app = Rectangle {
            tabs: vec![
                Tab {
                    last_input: Some(now - IDLE_LIMIT),
                    ..Default::default()
                },
                Tab {
                    last_input: Some(now - Duration::from_secs(1)),
                    ..Default::default()
                },
                Tab {
                    agent_port: Some(8080),
                    always_on: true,
                    last_input: Some(now - IDLE_LIMIT),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        app.expire_idle_tabs(&egui::Context::default());
        assert_eq!(app.tabs.len(), 3);
        assert!(app.tabs[0].idle_stopped);
        assert_eq!(app.tabs[0].error.as_deref(), Some(IDLE_MESSAGE));
        assert!(!app.tabs[1].idle_stopped);
        assert!(!app.tabs[2].idle_stopped);
    }

    #[test]
    fn proton_badge_keeps_the_navigation_row_compact() {
        for (width, height) in [(640.0, 420.0), (1120.0, 790.0), (1920.0, 1080.0)] {
            let ctx = egui::Context::default();
            design::theme(&ctx);
            let mut app = Rectangle {
                home: true,
                library_host: Backend::Secondary,
                secondary: Connection {
                    host: "test-host".into(),
                    key: None,
                },
                secondary_entries: ["First saved session", "Second saved session"]
                    .into_iter()
                    .enumerate()
                    .map(|(i, title)| saved::Entry {
                        codex: false,
                        id: format!("{i:032x}"),
                        title: title.into(),
                        startup_page: String::new(),
                        agent_port: None,
                        file_manager: false,
                    })
                    .collect(),
                ..Default::default()
            };
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, height))),
                    ..Default::default()
                },
                |ui| app.workspace(ui),
            );
            output.textures_delta.clear();
            let text_position = |label| {
                output
                    .shapes
                    .iter()
                    .find_map(|item| match &item.shape {
                        egui::Shape::Text(shape) if shape.galley.job.text == label => {
                            Some(shape.pos)
                        }
                        _ => None,
                    })
                    .expect("label was not drawn")
            };
            assert!(
                text_position("Saved sessions").y < 165.0,
                "toolbar expanded at {width}×{height}: {:?}",
                text_position("Saved sessions")
            );
            let address = output
                .shapes
                .iter()
                .find_map(|item| match &item.shape {
                    egui::Shape::Text(shape) if shape.galley.job.text.starts_with("Search or") => {
                        Some(shape.pos)
                    }
                    _ => None,
                })
                .expect("address bar was not drawn");
            let new_tab = text_position("+ New Tab");
            let vpn = text_position("Proton VPN: Inactive");
            let memory = text_position("Primary RAM —");
            assert!(text_position("Home").x < memory.x && memory.x < text_position("Settings").x);
            assert!(text_position("Settings").x < address.x && address.x < vpn.x);
            assert!(new_tab.y > address.y && new_tab.x < address.x && vpn.x > width - 165.0);
            assert!(app.tabs.is_empty());
            let first = text_position("First saved session");
            assert!(text_position("Open").x < first.x);
            assert!(first.x < text_position("Edit startup page").x);
            assert!(text_position("Edit startup page").x < text_position("⋯").x);
            let second = text_position("Second saved session");
            assert!((1.0..=48.0).contains(&(second.y - first.y)));
        }
    }

    #[test]
    #[ignore = "requires an installed remote Rectangle host and an SSH key"]
    fn opening_and_closing_tabs_keeps_the_other_session() {
        let mut app = Rectangle {
            host: std::env::var("RECTANGLE_SMOKE_HOST").unwrap(),
            identity: std::env::var_os("RECTANGLE_SMOKE_IDENTITY").map(std::path::PathBuf::from),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        app.add_tab(&ctx);
        app.add_tab(&ctx);
        assert_eq!(app.tabs.len(), 2, "{:?}", app.error);
        assert_eq!(app.active, 1);
        for tab in &app.tabs {
            let started = Instant::now();
            loop {
                let view = tab.session.as_ref().unwrap().view.lock().unwrap();
                assert!(view.error.is_none(), "{:?}", view.error);
                if view.frame.is_some() {
                    break;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(20),
                    "no frame for tab {}",
                    tab.id
                );
                drop(view);
                std::thread::sleep(Duration::from_millis(30));
            }
        }
        let surviving_id = app.tabs[1].id;
        let surviving_session = app.tabs[1].session.as_ref().unwrap().view.clone();
        app.close_tab(0);
        assert_eq!(app.active, 0);
        assert_eq!(app.tabs[0].id, surviving_id);
        assert!(std::sync::Arc::ptr_eq(
            &surviving_session,
            &app.tabs[0].session.as_ref().unwrap().view
        ));
        assert!(surviving_session.lock().unwrap().error.is_none());
        app.close_tab(0);
        assert!(app.tabs.is_empty());
    }
}
