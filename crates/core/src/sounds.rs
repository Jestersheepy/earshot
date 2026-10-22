//! Soft start/stop chimes, synthesised once and played asynchronously through the default output, so
//! there's no audio file to ship and no wait on the caller.

use std::sync::OnceLock;

use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
use windows::core::PCWSTR;

#[derive(Clone, Copy, Debug)]
pub enum Tone {
    /// Rising: listening.
    Start,
    /// Falling: stopped.
    Stop,
}

const SR: u32 = 24_000;
const E5: f32 = 659.25;
const A5: f32 = 880.0;

pub fn play(t: Tone) {
    static START: OnceLock<Vec<u8>> = OnceLock::new();
    static STOP: OnceLock<Vec<u8>> = OnceLock::new();
    let wav = match t {
        Tone::Start => START.get_or_init(|| wav(&chime(&[(E5, 0.0), (A5, 0.075)]))),
        Tone::Stop => STOP.get_or_init(|| wav(&chime(&[(A5, 0.0), (E5, 0.075)]))),
    };
    // The buffer is 'static, so the asynchronous play can read it after we return.
    unsafe {
        let _ = PlaySoundW(PCWSTR(wav.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
}

/// Notes (frequency, start s) as soft bell tones: quick attack, exponential decay, a little octave.
fn chime(notes: &[(f32, f32)]) -> Vec<f32> {
    let len = (SR as f32 * 0.32) as usize;
    let mut out = vec![0f32; len];
    for &(f, at) in notes {
        let start = (at * SR as f32) as usize;
        for (i, s) in out[start..].iter_mut().enumerate() {
            let t = i as f32 / SR as f32;
            let env = (t / 0.006).min(1.0) * (-t / 0.06).exp();
            let w = std::f32::consts::TAU * f * t;
            *s += env * (w.sin() + 0.18 * (2.0 * w).sin());
        }
    }
    out
}

/// 16-bit mono PCM WAV, quiet enough to sit under whatever else is playing.
fn wav(samples: &[f32]) -> Vec<u8> {
    const GAIN: f32 = 0.16;
    let data = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&SR.to_le_bytes());
    b.extend_from_slice(&(SR * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&(((s * GAIN).clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chimes_are_short_quiet_wavs() {
        let w = wav(&chime(&[(E5, 0.0), (A5, 0.075)]));
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(w.len(), 44 + (SR as f32 * 0.32) as usize * 2);
        let peak = w[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs()).max().unwrap();
        assert!(peak > 2000 && peak < 12000, "peak {peak}");
    }
}
