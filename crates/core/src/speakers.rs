//! Who spoke when: CAM++ speaker embeddings (3D-Speaker, sherpa-onnx export) over a Kaldi-style fbank, and
//! average-linkage clustering on cosine similarity. tools/reference_speakers.py is the reference.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use ndarray::Array3;
use ort::session::Session;
use ort::value::TensorRef;
use realfft::{RealFftPlanner, RealToComplex};

use crate::stt::{SR, ort_env};

pub const MODEL: &str = "3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx";
/// Same voice above this cosine similarity. On test voices the same speaker scores >= 0.82 and different
/// speakers <= 0.41.
pub const SAME_VOICE: f32 = 0.5;
/// Shorter audio gives unreliable embeddings; such lines take their speaker from the nearest voice.
pub const MIN_S: f32 = 0.8;

const FRAME: usize = 400;
const SHIFT: usize = 160;
const NFFT: usize = 512;
const BINS: usize = 80;

pub struct Embedder {
    session: Session,
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    /// Per mel bin: first FFT index and the triangle's weights from there.
    banks: Vec<(usize, Vec<f32>)>,
}

impl Embedder {
    /// `dir` holds the model (models/speaker).
    pub fn load(dir: &Path, threads: usize) -> Result<Self> {
        let env = ort_env(dir)?;
        let session = Session::builder(&env)?
            .with_intra_threads(threads)
            .map_err(ort::Error::<()>::from)?
            .commit_from_file(dir.join(MODEL))
            .with_context(|| format!("loading {MODEL}"))?;
        let window = (0..FRAME)
            .map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (FRAME - 1) as f64).cos()).powf(0.85) as f32)
            .collect();
        Ok(Self { session, fft: RealFftPlanner::<f32>::new().plan_fft_forward(NFFT), window, banks: mel_banks() })
    }

    /// (frames, 80) log mel energies, mean-normalised per bin. `pcm` is 16 kHz mono in [-1, 1].
    pub fn fbank(&self, pcm: &[f32]) -> (usize, Vec<f32>) {
        if pcm.len() < FRAME {
            return (0, Vec::new());
        }
        let n = 1 + (pcm.len() - FRAME) / SHIFT;
        let mut feats = vec![0f32; n * BINS];
        let mut buf = vec![0f32; NFFT];
        let mut spec = self.fft.make_output_vec();
        let mut frame = [0f64; FRAME];
        for f in 0..n {
            for (d, s) in frame.iter_mut().zip(&pcm[f * SHIFT..f * SHIFT + FRAME]) {
                *d = *s as f64;
            }
            let mean = frame.iter().sum::<f64>() / FRAME as f64;
            frame.iter_mut().for_each(|x| *x -= mean);
            for i in (1..FRAME).rev() {
                frame[i] -= 0.97 * frame[i - 1];
            }
            frame[0] -= 0.97 * frame[0];
            buf.fill(0.0);
            for i in 0..FRAME {
                buf[i] = (frame[i] * self.window[i] as f64) as f32;
            }
            self.fft.process(&mut buf, &mut spec).expect("fft sizes match");
            let row = &mut feats[f * BINS..(f + 1) * BINS];
            for (b, (start, w)) in self.banks.iter().enumerate() {
                let e: f32 = w.iter().zip(&spec[*start..]).map(|(w, c)| w * c.norm_sqr()).sum();
                row[b] = e.max(f32::EPSILON).ln();
            }
        }
        for b in 0..BINS {
            let mean = (0..n).map(|f| feats[f * BINS + b]).sum::<f32>() / n as f32;
            (0..n).for_each(|f| feats[f * BINS + b] -= mean);
        }
        (n, feats)
    }

    /// Unit-length voice embedding.
    pub fn embed(&mut self, pcm: &[f32]) -> Result<Vec<f32>> {
        let (n, feats) = self.fbank(pcm);
        anyhow::ensure!(n > 0, "audio too short for a speaker embedding");
        let input = Array3::from_shape_vec((1, n, BINS), feats)?;
        let out = self.session.run(ort::inputs!["x" => TensorRef::from_array_view(&input)?])?;
        let e: Vec<f32> = out["embedding"].try_extract_array::<f32>()?.iter().copied().collect();
        Ok(normalized(e))
    }
}

fn mel_banks() -> Vec<(usize, Vec<f32>)> {
    let mel = |f: f64| 1127.0 * (1.0 + f / 700.0).ln();
    let (lo, hi) = (mel(20.0), mel(SR as f64 / 2.0));
    let centers: Vec<f64> = (0..BINS + 2).map(|i| lo + (hi - lo) * i as f64 / (BINS + 1) as f64).collect();
    let fft_mel: Vec<f64> = (0..NFFT / 2).map(|k| mel(k as f64 * SR as f64 / NFFT as f64)).collect();
    (0..BINS)
        .map(|b| {
            let (l, c, r) = (centers[b], centers[b + 1], centers[b + 2]);
            let w: Vec<f32> = fft_mel.iter().map(|&m| ((m - l) / (c - l)).min((r - m) / (r - c)).max(0.0) as f32).collect();
            let start = w.iter().position(|&x| x > 0.0).unwrap_or(0);
            let end = w.iter().rposition(|&x| x > 0.0).map_or(start, |e| e + 1);
            (start, w[start..end].to_vec())
        })
        .collect()
}

pub fn normalized(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= n);
    v
}

pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Average-linkage agglomerative clustering of unit vectors: keep merging the most similar pair of clusters
/// while their mean pairwise cosine similarity is above `threshold`. Labels are numbered by first appearance.
pub fn cluster(embs: &[Vec<f32>], threshold: f32) -> Vec<usize> {
    let n = embs.len();
    // The mean pairwise dot product of two clusters equals dot(sum_a, sum_b) / (|a| |b|).
    let mut sums: Vec<Vec<f32>> = embs.to_vec();
    let mut members: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    let mut alive: Vec<bool> = vec![true; n];
    let mut sim = vec![f32::NEG_INFINITY; n * n];
    for a in 0..n {
        for b in a + 1..n {
            sim[a * n + b] = dot(&sums[a], &sums[b]);
        }
    }
    loop {
        let mut best = (f32::NEG_INFINITY, 0, 0);
        for a in (0..n).filter(|&a| alive[a]) {
            for b in (a + 1..n).filter(|&b| alive[b]) {
                if sim[a * n + b] > best.0 {
                    best = (sim[a * n + b], a, b);
                }
            }
        }
        let (s, a, b) = best;
        if s < threshold {
            break;
        }
        alive[b] = false;
        let moved = std::mem::take(&mut members[b]);
        members[a].extend(moved);
        let add = std::mem::take(&mut sums[b]);
        sums[a].iter_mut().zip(add).for_each(|(x, y)| *x += y);
        for o in (0..n).filter(|&o| alive[o] && o != a) {
            let v = dot(&sums[a], &sums[o]) / (members[a].len() * members[o].len()) as f32;
            sim[a.min(o) * n + a.max(o)] = v;
        }
    }
    let mut groups: Vec<&Vec<usize>> = members.iter().filter(|m| !m.is_empty()).collect();
    groups.sort_by_key(|g| *g.iter().min().unwrap());
    let mut labels = vec![0; n];
    for (k, g) in groups.iter().enumerate() {
        for &i in g.iter() {
            labels[i] = k;
        }
    }
    labels
}

/// Unit-length mean of the given embeddings.
pub fn centroid<'a>(embs: impl IntoIterator<Item = &'a Vec<f32>>) -> Vec<f32> {
    let mut sum: Vec<f32> = Vec::new();
    for e in embs {
        if sum.is_empty() {
            sum = vec![0.0; e.len()];
        }
        sum.iter_mut().zip(e).for_each(|(s, x)| *s += x);
    }
    normalized(sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clusters_obvious_groups() {
        let e = |x: f32, y: f32| normalized(vec![x, y, 0.0]);
        let embs = vec![e(1.0, 0.0), e(0.0, 1.0), e(0.95, 0.1), e(0.1, 0.9), e(1.0, 0.05)];
        assert_eq!(cluster(&embs, 0.5), vec![0, 1, 0, 1, 0]);
        assert_eq!(cluster(&embs, 0.0), vec![0, 0, 0, 0, 0]);
        assert_eq!(cluster(&[], 0.5), Vec::<usize>::new());
    }
}
