//! CoreAudio output for the server's 48 kHz, signed 16-bit stereo stream.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    io::{self, Read},
    sync::{
        atomic::{AtomicU8, Ordering},
        mpsc,
    },
};

pub fn play(source: impl Read, volume: &AtomicU8, ready: impl FnOnce()) -> io::Result<()> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or_else(|| io::Error::other("No audio output device available"))?;
    let config = device.default_output_config().map_err(io::Error::other)?;
    match config.sample_format() {
        cpal::SampleFormat::F32 => output::<f32>(device, config.into(), source, volume, ready),
        cpal::SampleFormat::I16 => output::<i16>(device, config.into(), source, volume, ready),
        cpal::SampleFormat::U16 => output::<u16>(device, config.into(), source, volume, ready),
        format => Err(io::Error::other(format!(
            "Unsupported audio output format: {format}"
        ))),
    }
}

fn output<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: cpal::Device,
    config: cpal::StreamConfig,
    mut source: impl Read,
    volume: &AtomicU8,
    ready: impl FnOnce(),
) -> io::Result<()> {
    // Bound the queue to 100 ms. Network reads never block the audio callback.
    let (tx, rx) = mpsc::sync_channel::<[f32; 2]>(4800);
    let (errors, failed) = mpsc::channel();
    let channels = usize::from(config.channels);
    let step = 48000.0 / f64::from(config.sample_rate);
    let mut phase = 1.0;
    let mut frame = [0.0; 2];
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                for samples in data.chunks_mut(channels) {
                    while phase >= 1.0 {
                        frame = rx.try_recv().unwrap_or([0.0; 2]);
                        phase -= 1.0;
                    }
                    for (channel, sample) in samples.iter_mut().enumerate() {
                        let value = if channels == 1 {
                            (frame[0] + frame[1]) * 0.5
                        } else if channel < 2 {
                            frame[channel]
                        } else {
                            0.0
                        };
                        *sample = T::from_sample(value);
                    }
                    phase += step;
                }
            },
            move |error| {
                let _ = errors.send(error);
            },
            None,
        )
        .map_err(io::Error::other)?;
    stream.play().map_err(io::Error::other)?;
    ready();
    loop {
        let mut block = [0; 3840];
        source.read_exact(&mut block)?;
        if let Ok(error) = failed.try_recv() {
            return Err(io::Error::other(error));
        }
        let gain = f32::from(volume.load(Ordering::Relaxed)) / (100.0 * 32768.0);
        for bytes in block.chunks_exact(4) {
            let frame = [
                i16::from_le_bytes([bytes[0], bytes[1]]) as f32 * gain,
                i16::from_le_bytes([bytes[2], bytes[3]]) as f32 * gain,
            ];
            loop {
                match tx.try_send(frame) {
                    Ok(()) => break,
                    Err(mpsc::TrySendError::Disconnected(_)) => {
                        return Err(io::Error::other("Audio output disconnected"));
                    }
                    Err(mpsc::TrySendError::Full(_)) => {
                        if let Ok(error) = failed.try_recv() {
                            return Err(io::Error::other(error));
                        }
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                }
            }
        }
    }
}
