//! Mic and system-audio (WASAPI loopback) capture into 16 kHz mono tracks.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;

use crate::stt::{SR, to_16k_mono};

/// Growing, thread-safe sample buffer. For loopback, gaps (WASAPI sends nothing while nothing plays)
/// are filled with silence against the wall clock so mic and system tracks stay aligned.
pub struct Track {
    samples: Mutex<Vec<f32>>,
    level: AtomicU32,
    pub t0: Instant,
}

impl Track {
    pub fn new(t0: Instant) -> Arc<Self> {
        Arc::new(Self { samples: Mutex::new(Vec::new()), level: AtomicU32::new(0), t0 })
    }

    pub fn push(&self, wave: &[f32], pad_to_clock: bool) {
        let mut s = self.samples.lock();
        if pad_to_clock {
            let expected = (self.t0.elapsed().as_secs_f64() * SR as f64) as usize;
            let gap = expected.saturating_sub(s.len() + wave.len());
            if gap > SR / 10 {
                let n = s.len() + gap;
                s.resize(n, 0.0);
            }
        }
        s.extend_from_slice(wave);
        if !wave.is_empty() {
            let rms = (wave.iter().map(|v| v * v).sum::<f32>() / wave.len() as f32).sqrt();
            self.level.store(rms.to_bits(), Ordering::Relaxed);
        }
    }

    pub fn len(&self) -> usize {
        self.samples.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Last block's RMS, for the waveform UI.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    pub fn read(&self, start: usize, end: Option<usize>) -> Vec<f32> {
        let s = self.samples.lock();
        let end = end.unwrap_or(s.len()).min(s.len());
        s[start.min(end)..end].to_vec()
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Source {
    Mic,
    /// Whatever the default output device is playing (the other side of a call).
    Loopback,
}

/// A running capture. cpal streams aren't Send everywhere, so each lives on its own thread.
pub struct Capture {
    pub track: Arc<Track>,
    stop: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    source: Source,
}

impl Capture {
    pub fn start(source: Source, t0: Instant) -> Result<Self> {
        let track = Track::new(t0);
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let t = track.clone();
        let thread = std::thread::spawn(move || {
            let stream = match open_stream(source, t) {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let _ = ready_tx.send(stream.play().map_err(|e| anyhow!("start stream: {e}")));
            let _ = stop_rx.recv();
            drop(stream);
        });
        ready_rx.recv().context("capture thread died")??;
        Ok(Self { track, stop: Some(stop_tx), thread: Some(thread), source })
    }

    pub fn stop(mut self) -> Arc<Track> {
        self.shutdown();
        self.track.clone()
    }

    fn shutdown(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if self.source == Source::Loopback {
            self.track.push(&[], true); // pad the tail so both tracks end together
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn open_stream(source: Source, track: Arc<Track>) -> Result<cpal::Stream> {
    let host = cpal::default_host();
    let (device, config) = match source {
        Source::Mic => {
            let d = host.default_input_device().context("no microphone")?;
            let c = d.default_input_config()?;
            (d, c)
        }
        Source::Loopback => {
            // cpal opens a WASAPI loopback client when an input stream is built on an output device
            let d = host.default_output_device().context("no output device")?;
            let c = d.default_output_config()?;
            (d, c)
        }
    };
    let (channels, rate) = (config.channels() as usize, config.sample_rate() as usize);
    let pad = source == Source::Loopback;
    let err = |e| eprintln!("audio stream error: {e}");
    let cfg = config.config();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            cfg.clone(),
            move |data: &[f32], _| track.push(&to_16k_mono(data, channels, rate), pad),
            err,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            cfg.clone(),
            move |data: &[i16], _| {
                let f: Vec<f32> = data.iter().map(|&s| s as f32 / 32768.0).collect();
                track.push(&to_16k_mono(&f, channels, rate), pad)
            },
            err,
            None,
        )?,
        f => anyhow::bail!("unsupported sample format {f:?}"),
    };
    Ok(stream)
}

pub fn write_wav(path: &std::path::Path, wave: &[f32]) -> Result<()> {
    let spec = hound::WavSpec { channels: 1, sample_rate: SR as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec)?;
    for &s in wave {
        w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16)?;
    }
    w.finalize()?;
    Ok(())
}

pub fn read_wav(path: &std::path::Path) -> Result<Vec<f32>> {
    let mut r = hound::WavReader::open(path)?;
    let spec = r.spec();
    let data: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => r.samples::<i16>().map(|s| s.unwrap_or(0) as f32 / 32768.0).collect(),
        hound::SampleFormat::Float => r.samples::<f32>().map(|s| s.unwrap_or(0.0)).collect(),
    };
    Ok(to_16k_mono(&data, spec.channels as usize, spec.sample_rate as usize))
}
