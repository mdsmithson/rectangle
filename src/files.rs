use crate::{Connection, MUTED, saved, session};
use eframe::egui::{self, Color32, RichText};
use std::{
    fs::File,
    io,
    path::PathBuf,
    process::Stdio,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

const PAGE_SIZE: usize = 150;
const HOME: &str = "/agent/home";

struct Entry {
    kind: &'static str,
    bytes: u64,
    name: String,
}

pub struct Explorer {
    path: String,
    offset: usize,
    total: usize,
    entries: Vec<Entry>,
    listing: Option<Receiver<io::Result<String>>>,
    upload: Option<Receiver<Vec<String>>>,
    status: Option<(bool, String)>,
    listed: bool,
}

impl Default for Explorer {
    fn default() -> Self {
        Self {
            path: format!("{HOME}/workspace"),
            offset: 0,
            total: 0,
            entries: Vec::new(),
            listing: None,
            upload: None,
            status: None,
            listed: false,
        }
    }
}

impl Explorer {
    pub fn uploading(&self) -> bool {
        self.upload.is_some()
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        connection: &Connection,
        token: &str,
        interactive: bool,
    ) -> bool {
        let mut activity = false;
        if let Some(job) = &self.upload {
            match job.try_recv() {
                Ok(errors) => {
                    self.upload = None;
                    self.status = Some(if errors.is_empty() {
                        (true, "Upload complete".into())
                    } else {
                        (false, errors.join("\n"))
                    });
                    self.listed = false;
                }
                Err(TryRecvError::Disconnected) => {
                    self.upload = None;
                    self.status = Some((false, "Upload stopped before confirmation.".into()));
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(job) = &self.listing {
            match job.try_recv() {
                Err(TryRecvError::Empty) => {}
                response => {
                    self.listing = None;
                    match response
                        .map_err(|_| io::Error::other("Directory listing disconnected."))
                        .and_then(|result| result)
                        .and_then(|data| parse_listing(&data))
                    {
                        Ok((total, entries)) => {
                            self.total = total;
                            self.entries = entries;
                        }
                        Err(error) => self.status = Some((false, error.to_string())),
                    }
                }
            }
        }

        let enabled = self.listing.is_none() && self.upload.is_none();
        let mut next_dir = None;
        let mut refresh = false;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(enabled && self.path != "/", egui::Button::new("Up"))
                .clicked()
            {
                let (parent, _) = self.path.rsplit_once('/').unwrap_or(("/", ""));
                next_dir = Some(if parent.is_empty() { "/" } else { parent }.to_owned());
            }
            if ui
                .add_enabled(enabled, egui::Button::new("Refresh"))
                .clicked()
            {
                refresh = true;
            }
            ui.label(RichText::new(&self.path).monospace());
        });
        let writable = self.path == HOME || self.path.starts_with(&format!("{HOME}/"));
        if writable {
            ui.label(
                RichText::new("Drop files here from Windows to upload to this folder.")
                    .color(MUTED),
            );
        }
        if self.upload.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Uploading…");
            });
        } else if let Some((success, message)) = &self.status {
            ui.label(RichText::new(message).color(if *success {
                MUTED
            } else {
                Color32::LIGHT_RED
            }));
        }
        ui.separator();
        if self.listing.is_some() {
            ui.spinner();
        }
        egui::ScrollArea::vertical()
            .id_salt(&self.path)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for entry in &self.entries {
                    ui.horizontal(|ui| {
                        if entry.kind == "Folder" {
                            if ui
                                .add_enabled(enabled, egui::Button::new(&entry.name))
                                .clicked()
                            {
                                next_dir = Some(format!(
                                    "{}/{}",
                                    self.path.trim_end_matches('/'),
                                    entry.name
                                ));
                            }
                        } else {
                            ui.label(&entry.name);
                        }
                        ui.label(
                            RichText::new(if entry.kind == "File" {
                                format!("{} bytes", entry.bytes)
                            } else {
                                entry.kind.to_owned()
                            })
                            .small()
                            .color(MUTED),
                        );
                    });
                }
            });
        if self.total > PAGE_SIZE {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(enabled && self.offset > 0, egui::Button::new("Previous"))
                    .clicked()
                {
                    self.offset = self.offset.saturating_sub(PAGE_SIZE);
                    refresh = true;
                }
                if ui
                    .add_enabled(
                        enabled && self.offset + PAGE_SIZE < self.total,
                        egui::Button::new("Next"),
                    )
                    .clicked()
                {
                    self.offset += PAGE_SIZE;
                    refresh = true;
                }
            });
        }
        if interactive && enabled {
            let dropped = ui.input(|input| {
                input
                    .raw
                    .dropped_files
                    .iter()
                    .map(|file| file.path().to_owned())
                    .collect::<Vec<_>>()
            });
            if !dropped.is_empty() {
                activity = true;
                if writable {
                    self.status = None;
                    self.upload = Some(upload(
                        ui.ctx(),
                        connection.clone(),
                        token.to_owned(),
                        self.path.clone(),
                        dropped,
                    ));
                } else {
                    self.status = Some((
                        false,
                        "Open a folder under /agent/home to upload files.".into(),
                    ));
                }
            }
        }
        if let Some(next) = next_dir {
            activity = true;
            self.path = next;
            self.offset = 0;
            self.total = 0;
            self.entries.clear();
            self.listed = false;
        } else if refresh {
            self.status = None;
            self.listed = false;
        }
        // A failed listing is retried on Refresh or when moving to another folder.
        if self.listing.is_none() && self.upload.is_none() && !self.listed {
            self.listed = true;
            let ctx = ui.ctx().clone();
            let command = format!(
                "agent-files {token} {} {}",
                saved::encode(&self.path),
                self.offset
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
        activity || refresh
    }
}

fn parse_listing(data: &str) -> io::Result<(usize, Vec<Entry>)> {
    let invalid = || io::Error::other("Invalid File Manager directory listing.");
    let mut lines = data.lines();
    let total = lines
        .next()
        .and_then(|line| line.strip_prefix("RECTFILES1 "))
        .and_then(|total| total.parse().ok())
        .ok_or_else(invalid)?;
    let mut entries = Vec::new();
    for line in lines {
        let mut fields = line.splitn(3, ' ');
        let kind = match fields.next() {
            Some("d") => "Folder",
            Some("f") => "File",
            Some("o") => "Other",
            _ => return Err(invalid()),
        };
        let bytes = fields
            .next()
            .and_then(|size| size.parse().ok())
            .ok_or_else(invalid)?;
        let name = fields
            .next()
            .and_then(|name| saved::decode(name, 510))
            .ok_or_else(invalid)?;
        if matches!(name.as_str(), "" | "." | "..") || name.contains('/') {
            return Err(invalid());
        }
        entries.push(Entry { kind, bytes, name });
    }
    Ok((total, entries))
}

fn upload(
    ctx: &egui::Context,
    connection: Connection,
    token: String,
    destination: String,
    files: Vec<PathBuf>,
) -> Receiver<Vec<String>> {
    let (send, receive) = mpsc::channel();
    let ctx = ctx.clone();
    thread::spawn(move || {
        let errors = files
            .into_iter()
            .filter_map(|path| {
                upload_file(&connection, &token, &destination, &path)
                    .err()
                    .map(|error| format!("{}: {error}", path.display()))
            })
            .collect();
        let _ = send.send(errors);
        ctx.request_repaint();
    });
    receive
}

fn upload_file(
    connection: &Connection,
    token: &str,
    destination: &str,
    path: &std::path::Path,
) -> io::Result<()> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("The local file name is not valid UTF-8."))?;
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other("Select a regular file."));
    }
    let command = format!(
        "exec ~/.local/bin/rectangle-session agent-upload {token} {} {} {}",
        saved::encode(destination),
        saved::encode(name),
        metadata.len()
    );
    let mut child =
        session::ssh_command(connection.host.trim(), connection.key.as_deref(), &command)?
            .stdout(Stdio::piped())
            .spawn()?;
    let copied = io::copy(&mut file, &mut child.stdin.take().unwrap());
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    copied?;
    if output.stdout != b"OK\n" {
        return Err(io::Error::other(
            "The server did not confirm the uploaded file.",
        ));
    }
    Ok(())
}
