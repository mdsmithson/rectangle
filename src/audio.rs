use crate::session::ssh_command;
use std::{
    io::{self, Read},
    path::Path,
    process::{Child, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    thread,
};

pub struct Audio {
    child: Child,
    pub status: Arc<Mutex<Status>>,
    volume: Arc<AtomicU8>,
}

#[derive(Clone)]
pub enum Status {
    Connecting,
    Playing,
    Failed(String),
}

impl Audio {
    pub fn start(
        host: &str,
        identity: Option<&Path>,
        token: &str,
        initial_volume: u8,
        repaint: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        let remote = format!("exec ~/.local/bin/rectangle-session audio {token}");
        let mut child = ssh_command(host, identity, &remote)?
            .stderr(Stdio::null())
            .spawn()?;
        let mut source = child.stdout.take().unwrap();
        let status = Arc::new(Mutex::new(Status::Connecting));
        let reported = status.clone();
        let volume = Arc::new(AtomicU8::new(initial_volume.min(100)));
        let playback_volume = Arc::clone(&volume);
        thread::spawn(move || {
            let result = (|| {
                let mut magic = [0; 8];
                source.read_exact(&mut magic)?;
                if &magic != b"RECTPCM1" {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Invalid audio stream",
                    ));
                }
                play(source, &playback_volume, || {
                    *reported.lock().unwrap() = Status::Playing;
                    repaint();
                })
            })();
            let message = result
                .err()
                .map_or_else(|| "Audio connection ended".into(), |e| e.to_string());
            *reported.lock().unwrap() = Status::Failed(message);
            repaint();
        });
        Ok(Self {
            child,
            status,
            volume,
        })
    }

    pub fn set_volume(&self, percent: u8) {
        self.volume.store(percent.min(100), Ordering::Relaxed);
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        // EOF on the SSH input asks the guest to stop its recording process.
        drop(self.child.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(not(windows))]
fn play(_: impl Read, _: &AtomicU8, _: impl FnOnce()) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Audio playback is implemented for Windows",
    ))
}

#[cfg(windows)]
fn play(mut source: impl Read, volume: &AtomicU8, ready: impl FnOnce()) -> io::Result<()> {
    use std::{
        cell::{Cell, UnsafeCell},
        mem::size_of,
        ptr,
        time::Duration,
    };
    use windows_sys::Win32::Media::Audio::*;

    const BYTES: usize = 3840; // 20 ms of 48 kHz stereo, 16 bits per sample.
    const HEADER: u32 = size_of::<WAVEHDR>() as u32;

    struct Slot {
        header: UnsafeCell<WAVEHDR>,
        data: UnsafeCell<[u8; BYTES]>,
        prepared: Cell<bool>,
    }
    struct Output {
        handle: HWAVEOUT,
        slots: Option<Box<[Slot; 4]>>,
    }
    fn check(code: u32) -> io::Result<()> {
        if code == 0 {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "Windows audio device error {code}"
            )))
        }
    }
    impl Drop for Output {
        fn drop(&mut self) {
            let slots = self.slots.take().unwrap();
            // The driver can retain pointers until reset/unprepare completes. If it
            // refuses to release them, keep the backing allocation valid.
            unsafe {
                let mut released = waveOutReset(self.handle) == 0;
                for slot in slots.iter().filter(|s| s.prepared.get()) {
                    released &= waveOutUnprepareHeader(self.handle, slot.header.get(), HEADER) == 0;
                }
                released &= waveOutClose(self.handle) == 0;
                if !released {
                    std::mem::forget(slots);
                }
            }
        }
    }
    let format = WAVEFORMATEX {
        wFormatTag: 1,
        nChannels: 2,
        nSamplesPerSec: 48000,
        nAvgBytesPerSec: 192000,
        nBlockAlign: 4,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    let mut handle = ptr::null_mut();
    // The WinMM driver copies the format on open. All queued buffers remain pinned
    // in `slots` and are only reused once the driver has released the old header.
    unsafe {
        check(waveOutOpen(
            &raw mut handle,
            WAVE_MAPPER,
            &raw const format,
            0,
            0,
            0,
        ))?;
    }
    let output = Output {
        handle,
        slots: Some(Box::new(std::array::from_fn(|_| Slot {
            header: UnsafeCell::new(WAVEHDR::default()),
            data: UnsafeCell::new([0; BYTES]),
            prepared: Cell::new(false),
        }))),
    };
    let mut ready = Some(ready);
    let mut index = 0;
    loop {
        let slot = &output.slots.as_ref().unwrap()[index];
        if slot.prepared.get() {
            loop {
                let code = unsafe { waveOutUnprepareHeader(handle, slot.header.get(), HEADER) };
                if code == WAVERR_STILLPLAYING {
                    thread::sleep(Duration::from_millis(2));
                } else {
                    check(code)?;
                    slot.prepared.set(false);
                    break;
                }
            }
        }
        // The slot is exclusively ours again after unprepare.
        unsafe {
            let samples = &mut *slot.data.get();
            source.read_exact(samples)?;
            scale_pcm(samples, volume.load(Ordering::Relaxed));
        }
        unsafe {
            slot.header.get().write(WAVEHDR {
                lpData: slot.data.get().cast(),
                dwBufferLength: BYTES as u32,
                ..WAVEHDR::default()
            });
            check(waveOutPrepareHeader(handle, slot.header.get(), HEADER))?;
        }
        slot.prepared.set(true);
        unsafe {
            check(waveOutWrite(handle, slot.header.get(), HEADER))?;
        }
        if let Some(ready) = ready.take() {
            ready();
        }
        index = (index + 1) % 4;
    }
}

#[cfg(windows)]
fn scale_pcm(samples: &mut [u8], percent: u8) {
    match percent {
        0 => samples.fill(0),
        100 => {}
        _ => {
            for sample in samples.chunks_exact_mut(2) {
                let value = i32::from(i16::from_le_bytes([sample[0], sample[1]]));
                let scaled = (value * i32::from(percent) / 100) as i16;
                sample.copy_from_slice(&scaled.to_le_bytes());
            }
        }
    }
}

#[cfg(all(test, windows))]
#[test]
fn scales_signed_stereo_pcm_at_the_selected_volume() {
    let stereo = [0, 128, 254, 127]; // -32768 in the left channel, 32766 in the right.
    let mut samples = stereo;
    scale_pcm(&mut samples, 100);
    assert_eq!(samples, stereo);
    scale_pcm(&mut samples, 50);
    assert_eq!(samples, [0, 192, 255, 63]);
    scale_pcm(&mut samples, 0);
    assert_eq!(samples, [0; 4]);
}

#[cfg(all(test, windows))]
#[test]
#[ignore = "requires the Windows default audio device"]
fn queues_pcm_on_the_windows_audio_device() {
    let samples = vec![0; 3840 * 6];
    let mut queued = false;
    let error = play(io::Cursor::new(samples), &AtomicU8::new(100), || {
        queued = true
    })
    .unwrap_err();
    assert!(queued, "Windows never accepted a PCM buffer: {error}");
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
}

#[cfg(all(test, windows))]
#[test]
#[ignore = "requires the primary server and a Windows audio device"]
fn plays_a_remote_tab_stream_through_windows() {
    use crate::{rfb::Resolution, session::Session};
    use std::time::{Duration, Instant};
    let host = std::env::var("RECTANGLE_SMOKE_HOST").unwrap();
    let identity = std::env::var_os("RECTANGLE_SMOKE_IDENTITY").map(std::path::PathBuf::from);
    let tab = Session::start(&host, identity.as_deref(), Resolution::FALLBACK, || {}).unwrap();
    let started = Instant::now();
    loop {
        let view = tab.view.lock().unwrap();
        assert!(view.error.is_none(), "{:?}", view.error);
        if view.frame.is_some() {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(25),
            "no remote display"
        );
        drop(view);
        thread::sleep(Duration::from_millis(30));
    }
    let audio = Audio::start(&host, identity.as_deref(), &tab.token, 100, || {}).unwrap();
    let started = Instant::now();
    loop {
        match audio.status.lock().unwrap().clone() {
            Status::Playing => break,
            Status::Connecting => (),
            Status::Failed(error) => panic!("{error}"),
        }
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "no remote PCM audio"
        );
        thread::sleep(Duration::from_millis(30));
    }
    drop(audio);
    assert!(tab.view.lock().unwrap().error.is_none());
}
