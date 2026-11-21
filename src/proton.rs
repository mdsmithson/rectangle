use crate::saved;
use eframe::egui;
use std::{
    fs::File,
    io::{self, Read},
    net::{Ipv4Addr, SocketAddrV4},
    path::Path,
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant},
};

type Task = Receiver<io::Result<String>>;
const MAGIC: &str = "RECTPROTON1";

#[derive(Default)]
pub struct Proton {
    state: Option<&'static str>,
    next: Option<Instant>,
    status_job: Option<Task>,
    config_job: Option<(bool, Task)>,
    restart_job: Option<Task>,
    summary: Option<String>,
    message: Option<(bool, String)>,
    path: String,
}

impl Proton {
    pub fn tick(&mut self, ctx: &egui::Context, host: &str, key: Option<&Path>) {
        if host.is_empty() {
            return;
        }
        if let Some(result) = take(&mut self.restart_job) {
            self.message = Some(
                match result.and_then(|data| {
                    (data.trim() == "OK").then_some(()).ok_or_else(|| {
                        io::Error::other("The server did not confirm the VPN restart.")
                    })
                }) {
                    Ok(()) => (
                        true,
                        "Proton VPN restarted and its HTTPS health check passed.".into(),
                    ),
                    Err(error) => (false, error.to_string()),
                },
            );
            self.status_job = None;
            self.next = None;
        }
        if let Some(result) = take(&mut self.status_job) {
            self.state = Some(
                match result
                    .as_deref()
                    .ok()
                    .and_then(|s| s.strip_prefix("RECTPROTON1\n"))
                    .map(str::trim)
                {
                    Some("active") => "Active",
                    Some("inactive" | "offline" | "unconfigured") => "Inactive",
                    _ => "Unavailable",
                },
            );
        }
        if let Some((replace, task)) = self.config_job.take() {
            match task.try_recv() {
                Err(TryRecvError::Empty) => self.config_job = Some((replace, task)),
                result => {
                    let result = result
                        .unwrap_or_else(|_| Err(io::Error::other("SSH request disconnected")));
                    if replace {
                        let result = result.and_then(|data| {
                            (data.trim() == "OK").then_some(()).ok_or_else(|| {
                                io::Error::other("The primary server did not confirm the change")
                            })
                        });
                        match result {
                            Ok(()) => {
                                self.path.clear();
                                self.message = Some((true, "Proton configuration applied.".into()));
                                self.request(ctx, host, key, None);
                            }
                            Err(error) => self.message = Some((false, error.to_string())),
                        }
                        self.status_job = None;
                        self.next = None;
                    } else {
                        match result.and_then(|data| view(&data)) {
                            Ok(summary) => self.summary = Some(summary),
                            Err(error) => self.message = Some((false, error.to_string())),
                        }
                    }
                }
            }
        }
        if self.status_job.is_none() && self.next.is_none_or(|next| Instant::now() >= next) {
            let repaint = ctx.clone();
            self.status_job = saved::run(host, key, "proton-status", move || {
                repaint.request_repaint()
            })
            .ok();
            if self.status_job.is_none() {
                self.state = Some("Unavailable");
            }
            self.next = Some(Instant::now() + Duration::from_secs(8));
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    pub fn badge(&self, ui: &mut egui::Ui, configured: bool) {
        let state = if !configured {
            "Inactive"
        } else if self.restart_job.is_some() {
            "Restarting"
        } else if self
            .config_job
            .as_ref()
            .is_some_and(|(replace, _)| *replace)
        {
            "Updating"
        } else {
            self.state.unwrap_or("Checking")
        };
        let (color, hint) = match state {
            "Active" => (
                crate::MINT,
                "The gateway reached an HTTPS endpoint through the VPN.",
            ),
            "Inactive" => (
                egui::Color32::GOLD,
                "The Proton tunnel is not active for Rectangle.",
            ),
            "Unavailable" => (crate::MUTED, "Could not verify Proton status over SSH."),
            _ => (
                crate::MUTED,
                "Checking the Proton gateway on your SSH host.",
            ),
        };
        crate::design::frame(crate::design::SURFACE, 8, 5)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (dot, _) =
                        ui.allocate_exact_size(egui::Vec2::splat(9.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 3.0, color);
                    ui.label(
                        egui::RichText::new(format!("Proton VPN: {state}"))
                            .small()
                            .strong()
                            .color(color),
                    );
                });
            })
            .response
            .on_hover_text(hint);
    }

    fn request(
        &mut self,
        ctx: &egui::Context,
        host: &str,
        key: Option<&Path>,
        data: Option<Vec<u8>>,
    ) {
        let replace = data.is_some();
        let request = if replace {
            "proton-replace"
        } else {
            "proton-view"
        };
        let repaint = ctx.clone();
        match saved::run_with_input(host, key, request, data, move || repaint.request_repaint()) {
            Ok(task) => self.config_job = Some((replace, task)),
            Err(error) => self.message = Some((false, error.to_string())),
        }
    }

    pub fn settings(&mut self, ui: &mut egui::Ui, name: &str, host: &str, key: Option<&Path>) {
        ui.heading(format!("Proton VPN · {name}"));
        if host.is_empty() {
            ui.label("Save an SSH connection before managing Proton VPN.");
            return;
        }
        ui.label(format!("Configuration on {host}"));
        let busy = self.config_job.is_some() || self.restart_job.is_some();
        if ui
            .add_enabled(!busy, egui::Button::new("Restart VPN"))
            .on_hover_text("Restart the shared gateway using its installed Proton profile.")
            .clicked()
        {
            self.message = None;
            let repaint = ui.ctx().clone();
            match saved::run(host, key, "proton-restart", move || {
                repaint.request_repaint()
            }) {
                Ok(task) => self.restart_job = Some(task),
                Err(error) => self.message = Some((false, error.to_string())),
            }
        }
        ui.label(
            egui::RichText::new(
                "Restarting briefly interrupts VPN browsing for all tabs on this server.",
            )
            .small()
            .color(crate::MUTED),
        );
        if ui
            .add_enabled(!busy, egui::Button::new("View current config"))
            .clicked()
        {
            self.message = None;
            self.request(ui.ctx(), host, key, None);
        }
        if let Some(summary) = &self.summary {
            ui.label(summary);
        }
        ui.label("Path to a new Proton WireGuard .conf file");
        ui.add_enabled(
            !busy,
            egui::TextEdit::singleline(&mut self.path)
                .desired_width(380.0)
                .hint_text(r"C:\Users\…\Downloads\wg.conf"),
        );
        ui.label(
            egui::RichText::new(format!(
                "Applying a new config briefly interrupts browsing in tabs on {name}."
            ))
            .small()
            .color(crate::MUTED),
        );
        if ui
            .add_enabled(
                !busy && !self.path.trim().is_empty(),
                egui::Button::new("Apply new config"),
            )
            .clicked()
        {
            self.message = None;
            let result = (|| {
                let path = Path::new(self.path.trim().trim_matches('"'));
                if !path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("conf"))
                    || !path.is_file()
                {
                    return Err(io::Error::other(
                        "Enter the path to a Proton WireGuard .conf file",
                    ));
                }
                let mut bytes = Vec::new();
                File::open(path)?.take(16_385).read_to_end(&mut bytes)?;
                if bytes.is_empty() || bytes.len() > 16_384 {
                    return Err(io::Error::other(
                        "WireGuard config must be between 1 byte and 16 KiB",
                    ));
                }
                Ok(bytes)
            })();
            match result {
                Ok(bytes) => self.request(ui.ctx(), host, key, Some(bytes)),
                Err(error) => self.message = Some((false, error.to_string())),
            }
        }
        if busy {
            ui.spinner();
        }
        if let Some((ok, message)) = &self.message {
            ui.label(egui::RichText::new(message).color(if *ok {
                crate::MINT
            } else {
                egui::Color32::LIGHT_RED
            }));
        }
    }
}

fn take(task: &mut Option<Task>) -> Option<io::Result<String>> {
    let result = match task.as_ref()?.try_recv() {
        Ok(result) => result,
        Err(TryRecvError::Empty) => return None,
        Err(TryRecvError::Disconnected) => Err(io::Error::other("SSH request disconnected")),
    };
    *task = None;
    Some(result)
}

fn view(data: &str) -> io::Result<String> {
    let invalid = || io::Error::other("Unexpected VPN configuration response");
    let mut lines = data.lines();
    if lines.next() != Some(MAGIC) {
        return Err(invalid());
    }
    let first = lines.next().ok_or_else(invalid)?;
    if first == "unconfigured" && lines.next().is_none() {
        return Ok("No Proton configuration installed.".into());
    }
    let endpoint: SocketAddrV4 = first.parse().map_err(|_| invalid())?;
    let dns: Ipv4Addr = lines
        .next()
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    let address: Ipv4Addr = lines
        .next()
        .and_then(|s| s.strip_suffix("/32"))
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    if lines.next().is_some() {
        return Err(invalid());
    }
    Ok(format!(
        "Endpoint: {endpoint}\nTunnel address: {address}/32\nDNS: {dns}\nRoute: all IPv4 through WireGuard · IPv6 blocked\nPrivate and preshared keys are hidden."
    ))
}
