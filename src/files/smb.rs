use crate::{Connection, saved};
use eframe::egui::{self, Color32};
use std::{
    collections::HashMap,
    io,
    net::Ipv4Addr,
    sync::mpsc::{Receiver, TryRecvError},
};

enum Service {
    Unconfigured,
    Ready {
        address: Ipv4Addr,
        folders: HashMap<String, Share>,
    },
}

struct Share {
    name: String,
    pending: bool,
}

enum Change {
    Enable { folder: String, name: String },
    Disable { folder: String },
}

#[derive(Default)]
pub(super) struct Shares {
    state: Option<Service>,
    attempted: bool,
    job: Option<(bool, Receiver<io::Result<String>>)>,
    dialog: Option<Change>,
    message: Option<(bool, String)>,
}

impl Shares {
    pub fn tick(&mut self, ctx: &egui::Context, connection: &Connection, token: &str) {
        if let Some((_, receiver)) = &self.job {
            let response = match receiver.try_recv() {
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    Err(io::Error::other("The SMB request disconnected."))
                }
                Ok(result) => result,
            };
            let changed = self.job.take().unwrap().0;
            match response.and_then(|output| parse(&output)) {
                Ok(state) => {
                    let pending = matches!(&state, Service::Ready { folders, .. } if folders.values().any(|share| share.pending));
                    self.state = Some(state);
                    self.message = changed.then(|| (true, if pending {
                        "Share queued. Save and close the Agent, then reopen it from Home to start sharing."
                    } else {
                        "SMB sharing updated."
                    }.into()));
                }
                Err(error) => {
                    let detail = error.to_string();
                    self.message = Some((
                        false,
                        if detail.contains("Invalid Rectangle display resolution.") {
                            "Update this server to manage SMB shares.".into()
                        } else {
                            detail
                        },
                    ));
                }
            }
        }
        if !self.attempted {
            self.attempted = true;
            self.request(ctx, connection, token, "list", false);
        }
    }

    fn request(
        &mut self,
        ctx: &egui::Context,
        connection: &Connection,
        token: &str,
        action: &str,
        changed: bool,
    ) {
        let repaint = ctx.clone();
        self.message = None;
        let request = format!("agent-smb {token} {action}");
        match saved::run(
            connection.host.trim(),
            connection.key.as_deref(),
            &request,
            move || repaint.request_repaint(),
        ) {
            Ok(result) => self.job = Some((changed, result)),
            Err(error) => self.message = Some((false, error.to_string())),
        }
    }

    pub fn context_menu(&mut self, ui: &mut egui::Ui, folder: &str, name: &str, agent: &str) {
        if self.job.is_some() {
            ui.add_enabled(false, egui::Button::new("Checking SMB…"));
            return;
        }
        if let Some(Service::Ready { address, folders }) = &self.state {
            let existing = folders.get(folder);
            let mut enabled = existing.is_some();
            if ui.checkbox(&mut enabled, "Share over SMB").changed() {
                self.dialog = Some(if enabled {
                    Change::Enable {
                        folder: folder.into(),
                        name: suggestion(agent, name),
                    }
                } else {
                    Change::Disable {
                        folder: folder.into(),
                    }
                });
                ui.close();
            }
            if let Some(existing) = existing.filter(|share| !share.pending) {
                let path = format!(r"\\{address}\{}", existing.name);
                if ui
                    .button("Copy Windows path")
                    .on_hover_text(&path)
                    .clicked()
                {
                    ui.ctx().copy_text(path);
                    ui.close();
                }
            } else if existing.is_some() {
                ui.label("Pending: save, close, and reopen the Agent.");
            }
        } else {
            let reason = if matches!(self.state, Some(Service::Unconfigured)) {
                "The server owner must enable SMB sharing once on this host."
            } else {
                "Could not read SMB settings. Update the server or refresh the folder."
            };
            ui.add_enabled(false, egui::Checkbox::new(&mut false, "Share over SMB"))
                .on_disabled_hover_text(reason);
        }
    }

    pub fn refresh(&mut self) {
        if self.job.is_none() {
            self.attempted = false;
        }
    }

    pub fn dialog(
        &mut self,
        ui: &mut egui::Ui,
        connection: &Connection,
        token: &str,
        enabled: bool,
    ) {
        if self.job.as_ref().is_some_and(|(changed, _)| *changed) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Updating SMB shares…");
            });
        }
        if let Some((ok, message)) = &self.message {
            ui.colored_label(
                if *ok {
                    crate::MUTED
                } else {
                    Color32::LIGHT_RED
                },
                message,
            );
        }
        let Some(mut change) = self.dialog.take() else {
            return;
        };
        let mut action = None;
        let mut keep = true;
        egui::Window::new("Share over SMB")
            .id(ui.id().with("agent smb dialog"))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .enabled(enabled)
            .show(ui.ctx(), |ui| {
                match &mut change {
                    Change::Enable { folder, name } => {
                        ui.label(folder.as_str());
                        ui.label("Windows share name");
                        ui.text_edit_singleline(name);
                        ui.label("Read only on your home network. A password is required.");
                        ui.label("First time only: save, close, and reopen the Agent. Rectangle needs space to copy the folder and keeps the original as a backup.");
                        if ui
                            .add_enabled(valid(name), egui::Button::new("Share folder"))
                            .clicked()
                        {
                            action = Some(format!(
                                "enable {} {}",
                                saved::encode(folder),
                                saved::encode(name)
                            ));
                        }
                    }
                    Change::Disable { folder } => {
                        ui.label("Stop sharing this folder? Its Windows drive will disconnect.");
                        if ui.button("Stop sharing").clicked() {
                            action = Some(format!("disable {}", saved::encode(folder)));
                        }
                    }
                }
                ui.label("Other SMB drives may briefly reconnect when shares change.");
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    keep = false;
                }
            });
        if let Some(action) = action {
            self.request(ui.ctx(), connection, token, &action, true);
        } else if keep {
            self.dialog = Some(change);
        }
    }
}

fn valid(name: &str) -> bool {
    name.len() <= 48
        && name
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && !["global", "homes", "printers", "ipc", "admin", "print"]
            .iter()
            .any(|reserved| name.eq_ignore_ascii_case(reserved))
}

fn suggestion(agent: &str, name: &str) -> String {
    fn clean(value: &str) -> String {
        value
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("-")
            .trim_start_matches('_')
            .chars()
            .take(23)
            .collect()
    }
    let agent = clean(agent);
    let folder = clean(name);
    format!(
        "{}-{}",
        if agent.is_empty() { "Agent" } else { &agent },
        if folder.is_empty() { "Folder" } else { &folder }
    )
}

fn parse(output: &str) -> io::Result<Service> {
    let invalid = || io::Error::other("Unexpected SMB reply from the server.");
    let mut lines = output.lines();
    let (version, header) = lines
        .next()
        .and_then(|line| line.split_once(' '))
        .ok_or_else(invalid)?;
    if !matches!(version, "RECTSMB1" | "RECTSMB2") {
        return Err(invalid());
    }
    if header == "disabled" {
        return Ok(Service::Unconfigured);
    }
    let address = header.parse().map_err(|_| invalid())?;
    let mut folders = HashMap::new();
    for line in lines {
        let (folder, share) = line.split_once(' ').ok_or_else(invalid)?;
        let (share, pending) = if version == "RECTSMB2" {
            match share.split_once(' ') {
                Some((name, "pending")) => (name, true),
                Some((name, "live")) => (name, false),
                _ => return Err(invalid()),
            }
        } else {
            (share, false)
        };
        let folder = saved::decode(folder, 2048).ok_or_else(invalid)?;
        if !valid(share) || !folder.starts_with("/agent/home/") || folders.len() >= 32 {
            return Err(invalid());
        }
        folders.insert(
            folder,
            Share {
                name: share.into(),
                pending,
            },
        );
    }
    Ok(Service::Ready { address, folders })
}

#[test]
fn share_names_fit_windows_paths() {
    assert_eq!(suggestion("Agent E", "Music"), "Agent-E-Music");
    assert!(valid(&suggestion("___", "音楽")));
    assert!(!valid("Global"));
    assert!(!valid("Music/../../tmp"));
}

#[test]
fn pending_share_is_distinct_from_a_live_windows_share() {
    let folder = "/agent/home/workspace/Music";
    let output = format!(
        "RECTSMB2 192.168.1.2\n{} Music pending\n",
        saved::encode(folder)
    );
    let Service::Ready { folders, .. } = parse(&output).unwrap() else {
        panic!("missing share")
    };
    assert!(folders[folder].pending);
    assert_eq!(folders[folder].name, "Music");
}
