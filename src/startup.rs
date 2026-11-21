use crate::saved::{self, Entry, Parked};
use std::{
    io,
    path::Path,
    sync::mpsc::{Receiver, TryRecvError},
};

type Task = Receiver<io::Result<String>>;

pub struct Host {
    library: Option<Task>,
    parked: Option<Task>,
    result: Snapshot,
}

#[derive(Default)]
pub struct Snapshot {
    pub entries: Option<Vec<Entry>>,
    pub parked: Option<Vec<Parked>>,
    pub errors: Vec<String>,
}

impl Host {
    pub fn start(host: &str, key: Option<&Path>, ctx: &eframe::egui::Context) -> Self {
        let mut result = Snapshot::default();
        let mut request = |command| {
            let repaint = ctx.clone();
            match saved::run(host, key, command, move || repaint.request_repaint()) {
                Ok(task) => Some(task),
                Err(error) => {
                    result.errors.push(error.to_string());
                    None
                }
            }
        };
        // Begin both reads together. The parked response is reused when showing
        // Home, rather than issuing another SSH request after the library loads.
        let parked = request("parked-list");
        let library = request("list-v2");
        Self {
            library,
            parked,
            result,
        }
    }

    pub fn poll(&mut self) -> Option<Snapshot> {
        if let Some(result) = take(&mut self.library) {
            match result.and_then(|data| saved::parse(&data)) {
                Ok(entries) => self.result.entries = Some(entries),
                Err(error) => self.result.errors.push(error.to_string()),
            }
        }
        if let Some(result) = take(&mut self.parked) {
            match result.and_then(|data| saved::parse_parked(&data)) {
                Ok(parked) => self.result.parked = Some(parked),
                Err(error) => self.result.errors.push(error.to_string()),
            }
        }
        (self.library.is_none() && self.parked.is_none()).then(|| std::mem::take(&mut self.result))
    }

    #[cfg(test)]
    pub fn from_tasks(library: Task, parked: Task) -> Self {
        Self {
            library: Some(library),
            parked: Some(parked),
            result: Snapshot::default(),
        }
    }
}

fn take(task: &mut Option<Task>) -> Option<io::Result<String>> {
    let result = match task.as_ref()?.try_recv() {
        Err(TryRecvError::Empty) => return None,
        Err(TryRecvError::Disconnected) => Err(io::Error::other("Startup request disconnected")),
        Ok(result) => result,
    };
    *task = None;
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn startup_waits_for_both_reads_and_preserves_partial_success() {
        let (library_tx, library_rx) = mpsc::channel();
        let (parked_tx, parked_rx) = mpsc::channel();
        let mut host = Host::from_tasks(library_rx, parked_rx);
        library_tx.send(Ok("RECTSAVED2\n".into())).unwrap();
        assert!(host.poll().is_none());
        drop(parked_tx);
        let result = host.poll().unwrap();
        assert!(result.entries.unwrap().is_empty());
        assert!(result.parked.is_none());
        assert_eq!(result.errors.len(), 1);
    }

    #[test]
    #[ignore = "startup network measurement requires a live Rectangle host and SSH key"]
    fn startup_network_timing() {
        use std::time::{Duration, Instant};
        let host = std::env::var("RECTANGLE_SMOKE_HOST").unwrap();
        let key = std::env::var_os("RECTANGLE_SMOKE_IDENTITY").map(std::path::PathBuf::from);
        let ctx = eframe::egui::Context::default();
        for _ in 0..3 {
            let started = Instant::now();
            for command in ["parked-list", "list-v2", "parked-list"] {
                saved::run(&host, key.as_deref(), command, || {})
                    .unwrap()
                    .recv_timeout(Duration::from_secs(20))
                    .unwrap()
                    .unwrap();
            }
            let old = started.elapsed();
            let started = Instant::now();
            let mut pending = Host::start(&host, key.as_deref(), &ctx);
            loop {
                if let Some(snapshot) = pending.poll() {
                    assert!(snapshot.errors.is_empty(), "{:?}", snapshot.errors);
                    assert!(snapshot.entries.is_some() && snapshot.parked.is_some());
                    break;
                }
                assert!(started.elapsed() < Duration::from_secs(20));
                std::thread::sleep(Duration::from_millis(2));
            }
            println!(
                "sequential_ms={:.1} parallel_ms={:.1}",
                old.as_secs_f64() * 1000.0,
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
}
