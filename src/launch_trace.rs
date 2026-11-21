use std::{
    fs::File,
    io::Write,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

struct Trace {
    started: Instant,
    file: Mutex<File>,
}

fn trace() -> Option<&'static Trace> {
    static TRACE: OnceLock<Option<Trace>> = OnceLock::new();
    TRACE
        .get_or_init(|| {
            let started = Instant::now();
            let path = std::env::var_os("RECTANGLE_STARTUP_TRACE")?;
            Some(Trace {
                started,
                file: Mutex::new(File::create(path).ok()?),
            })
        })
        .as_ref()
}

pub fn checkpoint(stage: &str) {
    if let Some(trace) = trace()
        && let Ok(mut file) = trace.file.lock()
    {
        let _ = writeln!(
            file,
            "{stage} {:.3}",
            trace.started.elapsed().as_secs_f64() * 1000.0
        );
    }
}

pub fn frame(ctx: &eframe::egui::Context) {
    if trace().is_none() {
        return;
    }
    static FIRST_UI: AtomicBool = AtomicBool::new(true);
    static FIRST_FRAME: AtomicBool = AtomicBool::new(true);
    if FIRST_UI.swap(false, Ordering::Relaxed) {
        checkpoint("first-ui");
        ctx.request_repaint();
    }
    // A second native frame starts after eframe has painted the first frame
    // and made its window visible. Do not confuse an extra layout pass with it.
    if ctx.cumulative_frame_nr() > 0 && FIRST_FRAME.swap(false, Ordering::Relaxed) {
        checkpoint("first-frame-complete");
    }
}
