use crate::session::ssh_command;
use std::{
    io::{self, Read},
    path::Path,
    process::{Child, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct HostMeter {
    next: Option<Instant>,
    pending: Option<std::sync::mpsc::Receiver<io::Result<String>>>,
    reading: Option<(u64, u64)>,
    unavailable: bool,
}

impl HostMeter {
    pub fn tick(&mut self, ctx: &eframe::egui::Context, host: &str, key: Option<&Path>) {
        use std::sync::mpsc::TryRecvError;
        if host.is_empty() {
            return;
        }
        if let Some(rx) = &self.pending {
            self.reading = match rx.try_recv() {
                Err(TryRecvError::Empty) => return,
                Ok(Ok(response)) => {
                    let mut fields = response.split_whitespace();
                    (|| {
                        if fields.next()? != "RECTHOSTMEM1" {
                            return None;
                        }
                        let used: u64 = fields.next()?.parse().ok()?;
                        let total: u64 = fields.next()?.parse().ok()?;
                        (total > 0 && used <= total && fields.next().is_none())
                            .then_some((used, total))
                    })()
                }
                _ => None,
            };
            self.unavailable = self.reading.is_none();
            self.pending = None;
        }
        let now = Instant::now();
        if self.next.is_none_or(|next| now >= next) {
            let repaint = ctx.clone();
            match crate::saved::run(host, key, "host-memory", move || repaint.request_repaint()) {
                Ok(task) => self.pending = Some(task),
                Err(_) => {
                    self.reading = None;
                    self.unavailable = true;
                }
            }
            self.next = Some(now + Duration::from_secs(5));
        }
        ctx.request_repaint_after(self.next.unwrap().saturating_duration_since(now));
    }

    pub fn show(&self, ui: &mut eframe::egui::Ui, configured: bool, compact: bool) -> bool {
        use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
        let width = if compact { 124.0 } else { 163.0 };
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 29.0), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                configured,
                "Open primary server btop",
            )
        });
        let (label, ratio) = match self.reading.filter(|_| configured) {
            Some((used, total)) => {
                let ratio = (used as f64 / total as f64) as f32;
                let gib = |n| n as f64 / (1_u64 << 30) as f64;
                (
                    format!("Primary {:.1}/{:.1} GiB", gib(used), gib(total)),
                    ratio,
                )
            }
            None => (
                if !configured || self.unavailable {
                    "Primary RAM —"
                } else {
                    "Primary RAM …"
                }
                .into(),
                0.0,
            ),
        };
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            8,
            if response.hovered() && configured {
                Color32::from_rgb(37, 45, 53)
            } else {
                Color32::from_rgb(26, 29, 35)
            },
        );
        let bar = Rect::from_min_max(
            Pos2::new(rect.left() + 8.0, rect.bottom() - 7.0),
            Pos2::new(rect.right() - 8.0, rect.bottom() - 4.0),
        );
        painter.rect_filled(bar, 2, crate::design::EDGE);
        if ratio > 0.0 {
            let color = if ratio >= 0.95 {
                Color32::LIGHT_RED
            } else if ratio >= 0.85 {
                Color32::GOLD
            } else {
                crate::MINT
            };
            painter.rect_filled(
                Rect::from_min_size(bar.min, Vec2::new(bar.width() * ratio, bar.height())),
                2,
                color,
            );
        }
        painter.text(
            Pos2::new(rect.center().x, rect.top() + 10.0),
            egui::Align2::CENTER_CENTER,
            label,
            egui::TextStyle::Small.resolve(ui.style()),
            crate::INK,
        );
        let response = response.on_hover_text(if configured {
            "Primary server RAM · Click to open btop over SSH"
        } else {
            "Configure the primary server in Settings to view host RAM."
        });
        configured && response.clicked()
    }
}

#[derive(Clone, Copy)]
pub struct Sample {
    pub used: u64,
    pub limit: u64,
    pub high: u64,
    pub max: u64,
    pub oom_kills: u64,
    pub stall: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Normal,
    NearLimit,
    Pressure,
    Reclaim,
    Throttling,
    Killed,
}

#[derive(Clone, Copy)]
pub struct Reading {
    pub sample: Sample,
    pub received: Instant,
    throttled: Option<Instant>,
    hit_limit: Option<Instant>,
}

impl Reading {
    fn new(sample: Sample, old: Option<Self>, received: Instant) -> Self {
        let event = |count, old_count, last| {
            if count > old_count {
                Some(received)
            } else {
                last
            }
        };
        let (throttled, hit_limit) = old.map_or((None, None), |old| {
            (
                event(sample.high, old.sample.high, old.throttled),
                event(sample.max, old.sample.max, old.hit_limit),
            )
        });
        Self {
            sample,
            received,
            throttled,
            hit_limit,
        }
    }

    pub fn ratio(self) -> f32 {
        (self.sample.used as f64 / self.sample.limit as f64) as f32
    }

    pub fn level(self, now: Instant) -> Level {
        let recent = |time: Option<Instant>| {
            time.is_some_and(|time| now.saturating_duration_since(time) < Duration::from_secs(4))
        };
        if self.sample.oom_kills > 0 {
            Level::Killed
        } else if recent(self.throttled) {
            Level::Throttling
        } else if recent(self.hit_limit) {
            Level::Reclaim
        } else if self.sample.stall.is_some_and(|stall| stall >= 0.10) {
            Level::Pressure
        } else if self.ratio() >= 0.85 {
            Level::NearLimit
        } else {
            Level::Normal
        }
    }
}

#[derive(Clone, Copy)]
pub enum Status {
    Connecting,
    Live(Reading),
    Unavailable,
}

pub struct Meter {
    child: Child,
    status: Arc<Mutex<Status>>,
}

impl Meter {
    pub fn start(
        host: &str,
        identity: Option<&Path>,
        token: &str,
        repaint: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        let remote = format!("exec ~/.local/bin/rectangle-session memory {token}");
        let mut child = ssh_command(host, identity, &remote)?
            .stderr(Stdio::null())
            .spawn()?;
        let mut source = child.stdout.take().unwrap();
        let status = Arc::new(Mutex::new(Status::Connecting));
        let shared = status.clone();
        thread::spawn(move || {
            let mut magic = [0; 8];
            if source.read_exact(&mut magic).is_ok() && &magic == b"RECTMEM1" {
                let mut previous = None;
                while let Ok(sample) = read_sample(&mut source) {
                    let reading = Reading::new(sample, previous, Instant::now());
                    *shared.lock().unwrap() = Status::Live(reading);
                    previous = Some(reading);
                    repaint();
                }
            }
            *shared.lock().unwrap() = Status::Unavailable;
            repaint();
        });
        Ok(Self { child, status })
    }

    pub fn status(&self) -> Status {
        *self.status.lock().unwrap()
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_sample(source: &mut impl Read) -> io::Result<Sample> {
    let mut packet = [0; 48];
    source.read_exact(&mut packet)?;
    let [used, limit, high, max, oom_kills, stall] =
        std::array::from_fn(|i| u64::from_be_bytes(packet[i * 8..i * 8 + 8].try_into().unwrap()));
    if limit == 0 || (stall > 10_000 && stall != u64::MAX) {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(Sample {
        used,
        limit,
        high,
        max,
        oom_kills,
        stall: (stall != u64::MAX).then(|| stall as f64 / 100.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_memory_events_from_high_usage() {
        let bytes: Vec<_> = [900_u64, 1000, 10, 10, 0, 0]
            .into_iter()
            .flat_map(u64::to_be_bytes)
            .collect();
        let sample = read_sample(&mut bytes.as_slice()).unwrap();
        let now = Instant::now();
        let initial = Reading::new(sample, None, now);
        assert_eq!(initial.level(now), Level::NearLimit);
        let reclaim = Reading::new(Sample { max: 11, ..sample }, Some(initial), now);
        assert_eq!(reclaim.level(now), Level::Reclaim);
        let high = Reading::new(Sample { high: 11, ..sample }, Some(initial), now);
        assert_eq!(high.level(now), Level::Throttling);
        assert_eq!(high.level(now + Duration::from_secs(5)), Level::NearLimit);
        let pressure = Reading::new(
            Sample {
                used: 500,
                stall: Some(1.2),
                ..sample
            },
            None,
            now,
        );
        assert_eq!(pressure.level(now), Level::Pressure);
        let killed = Reading::new(
            Sample {
                oom_kills: 1,
                ..sample
            },
            None,
            now,
        );
        assert_eq!(killed.level(now), Level::Killed);
    }

    #[test]
    #[ignore = "requires a live Rectangle host and SSH key"]
    fn reads_live_tab_memory() {
        use crate::{rfb::Resolution, session::Session};
        let host = std::env::var("RECTANGLE_SMOKE_HOST").unwrap();
        let identity = std::env::var_os("RECTANGLE_SMOKE_IDENTITY").map(std::path::PathBuf::from);
        let session =
            Session::start(&host, identity.as_deref(), Resolution::FALLBACK, || {}).unwrap();
        let deadline = Instant::now() + Duration::from_secs(25);
        while session.view.lock().unwrap().frame.is_none() {
            assert!(Instant::now() < deadline, "tab did not start");
            thread::sleep(Duration::from_millis(50));
        }
        let meter = Meter::start(&host, identity.as_deref(), &session.token, || {}).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut first = None;
        while Instant::now() < deadline {
            match meter.status() {
                Status::Connecting => {}
                Status::Unavailable => panic!("memory stream disconnected"),
                Status::Live(reading) => {
                    assert!(reading.sample.used > 0);
                    assert_eq!(reading.sample.limit, 3 << 30);
                    if reading.received != *first.get_or_insert(reading.received) {
                        eprintln!(
                            "Live tab: {} MiB of {} MiB",
                            reading.sample.used >> 20,
                            reading.sample.limit >> 20
                        );
                        return;
                    }
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("did not get two memory readings");
    }
}
