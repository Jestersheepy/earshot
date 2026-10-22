//! Bark speech-to-text on CPU: int8 ONNX encoder + TDT greedy decoder in plain Rust.
//! Port of the Python reference (earshot/stt/engine.py); `tests/golden.rs` keeps them in lockstep.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ndarray::Array3;
use ort::session::Session;
use ort::value::TensorRef;
use parking_lot::Mutex;
use realfft::RealFftPlanner;
use safetensors::{Dtype, SafeTensors};
use serde::Deserialize;

pub const SR: usize = 16_000;
const LOG_GUARD: f32 = 5.960_464_5e-8; // 2^-24
const HIDDEN: usize = 640;
const MAX_CHUNK_S: f32 = 30.0; // encoder attention is quadratic; split long audio at quiet points

#[derive(Deserialize)]
struct Config {
    vocab: Vec<String>,
    blank_id: usize,
    durations: Vec<usize>,
    max_symbols_per_step: usize,
    n_fft: usize,
    hop: usize,
    win: usize,
    preemphasis: f32,
    pad_s: f32,
}

/// Row-major matrix.
struct Mat {
    rows: usize,
    cols: usize,
    data: Vec<f32>,
}

impl Mat {
    fn row(&self, r: usize) -> &[f32] {
        &self.data[r * self.cols..(r + 1) * self.cols]
    }

    /// out = self @ x (+ out when accumulate)
    fn matvec(&self, x: &[f32], out: &mut [f32]) {
        for (r, o) in out.iter_mut().enumerate().take(self.rows) {
            *o += dot(self.row(r), x);
        }
    }
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    // 8 independent accumulators so LLVM emits packed FMAs.
    let mut acc = [0f32; 8];
    let (ca, cb) = (a.chunks_exact(8), b.chunks_exact(8));
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for i in 0..8 {
            acc[i] = x[i].mul_add(y[i], acc[i]);
        }
    }
    acc.iter().sum::<f32>() + ra.iter().zip(rb).map(|(x, y)| x * y).sum::<f32>()
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

struct Lstm {
    w_ih: Mat,
    w_hh: Mat,
    bias: Vec<f32>,
}

pub struct BarkModel {
    session: Session,
    cfg: Config,
    emb: Mat,
    lstm: Vec<Lstm>,
    dec_w: Mat,
    dec_b: Vec<f32>,
    joint_w: Mat,
    joint_b: Vec<f32>,
    mel: Mat,
    window: Vec<f32>,
}

fn load_mat(st: &SafeTensors, name: &str) -> Result<Mat> {
    let t = st.tensor(name).with_context(|| format!("missing tensor {name}"))?;
    let shape = t.shape();
    let (rows, cols) = (shape[0], if shape.len() > 1 { shape[1] } else { 1 });
    let bytes = t.data();
    let data = match t.dtype() {
        Dtype::F32 => bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect(),
        Dtype::F16 => bytes
            .chunks_exact(2)
            .map(|b| half::f16::from_le_bytes([b[0], b[1]]).to_f32())
            .collect(),
        d => anyhow::bail!("{name}: unsupported dtype {d:?}"),
    };
    Ok(Mat { rows, cols, data })
}

impl BarkModel {
    pub fn load(dir: &Path, threads: usize) -> Result<Self> {
        let env = ort_env(dir)?;
        let session = Session::builder(&env)?
            .with_intra_threads(threads)
            .map_err(ort::Error::<()>::from)?
            .commit_from_file(dir.join("encoder.int8.onnx"))
            .context("loading encoder.int8.onnx")?;
        let raw = std::fs::read(dir.join("decoder.safetensors")).context("reading decoder.safetensors")?;
        let st = SafeTensors::deserialize(&raw)?;
        let cfg: Config = serde_json::from_slice(&std::fs::read(dir.join("config.json"))?)?;
        let lstm = (0..2)
            .map(|i| {
                Ok(Lstm {
                    w_ih: load_mat(&st, &format!("lstm_weight_ih_l{i}"))?,
                    w_hh: load_mat(&st, &format!("lstm_weight_hh_l{i}"))?,
                    bias: load_mat(&st, &format!("lstm_bias_l{i}"))?.data,
                })
            })
            .collect::<Result<_>>()?;
        // torch.hann_window(win, periodic=False), centred inside n_fft like torch.stft
        let pad = (cfg.n_fft - cfg.win) / 2;
        let mut window = vec![0f32; cfg.n_fft];
        for i in 0..cfg.win {
            let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (cfg.win - 1) as f64).cos();
            window[pad + i] = w as f32;
        }
        Ok(Self {
            session,
            emb: load_mat(&st, "embedding")?,
            lstm,
            dec_w: load_mat(&st, "dec_proj_w")?,
            dec_b: load_mat(&st, "dec_proj_b")?.data,
            joint_w: load_mat(&st, "joint_w")?,
            joint_b: load_mat(&st, "joint_b")?.data,
            mel: load_mat(&st, "mel_filters")?,
            window,
            cfg,
        })
    }

    /// Log-mel features, per-feature normalised: (frames, 128) row-major.
    fn features(&self, wave: &[f32]) -> (usize, Vec<f32>) {
        let (n_fft, hop) = (self.cfg.n_fft, self.cfg.hop);
        let half = n_fft / 2;
        let mut x = vec![0f32; wave.len() + 2 * half];
        if !wave.is_empty() {
            x[half] = wave[0];
            for i in 1..wave.len() {
                x[half + i] = wave[i] - self.cfg.preemphasis * wave[i - 1];
            }
        }
        let n_frames = 1 + (x.len() - n_fft) / hop;
        let n_mels = self.mel.rows;
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(n_fft);
        let (mut buf, mut spec) = (fft.make_input_vec(), fft.make_output_vec());
        let mut power = vec![0f32; spec.len()];
        let mut feats = vec![0f32; n_frames * n_mels];
        for f in 0..n_frames {
            let frame = &x[f * hop..f * hop + n_fft];
            for i in 0..n_fft {
                buf[i] = frame[i] * self.window[i];
            }
            fft.process(&mut buf, &mut spec).expect("fft");
            for (p, c) in power.iter_mut().zip(&spec) {
                *p = c.norm_sqr();
            }
            for m in 0..n_mels {
                feats[f * n_mels + m] = (dot(self.mel.row(m), &power) + LOG_GUARD).ln();
            }
        }
        let valid = wave.len() / hop;
        for m in 0..n_mels {
            let col = (0..valid).map(|f| feats[f * n_mels + m] as f64);
            let mean = col.clone().sum::<f64>() / valid.max(1) as f64;
            let var = col.map(|v| (v - mean).powi(2)).sum::<f64>() / valid.saturating_sub(1).max(1) as f64;
            let std = var.sqrt() as f32 + 1e-5;
            for f in 0..n_frames {
                let v = &mut feats[f * n_mels + m];
                *v = if f < valid { (*v - mean as f32) / std } else { 0.0 };
            }
        }
        (n_frames, feats)
    }

    fn dec_step(&self, token: usize, state: &mut [(Vec<f32>, Vec<f32>)]) -> Vec<f32> {
        let mut x = self.emb.row(token).to_vec();
        for (l, (h, c)) in self.lstm.iter().zip(state.iter_mut()) {
            let mut g = l.bias.clone();
            l.w_ih.matvec(&x, &mut g);
            l.w_hh.matvec(h, &mut g);
            for k in 0..HIDDEN {
                let (i, f, gg, o) = (g[k], g[HIDDEN + k], g[2 * HIDDEN + k], g[3 * HIDDEN + k]);
                c[k] = sigmoid(f) * c[k] + sigmoid(i) * gg.tanh();
                h[k] = sigmoid(o) * c[k].tanh();
            }
            x = h.clone();
        }
        let mut out = self.dec_b.clone();
        self.dec_w.matvec(&x, &mut out);
        out
    }

    fn greedy_tdt(&self, enc: &[f32], steps: usize) -> Vec<usize> {
        let v = self.cfg.vocab.len();
        let mut state = vec![(vec![0f32; HIDDEN], vec![0f32; HIDDEN]); self.lstm.len()];
        let mut dec = self.dec_step(self.cfg.blank_id, &mut state);
        let (mut tokens, mut t, mut same_frame) = (Vec::new(), 0usize, 0usize);
        let mut hidden = vec![0f32; HIDDEN];
        let mut logits = vec![0f32; self.joint_w.rows];
        while t < steps {
            let e = &enc[t * HIDDEN..(t + 1) * HIDDEN];
            for k in 0..HIDDEN {
                hidden[k] = (e[k] + dec[k]).max(0.0);
            }
            logits.copy_from_slice(&self.joint_b);
            self.joint_w.matvec(&hidden, &mut logits);
            let tok = argmax(&logits[..v]);
            let mut dur = self.cfg.durations[argmax(&logits[v..])];
            if tok == self.cfg.blank_id {
                dur = dur.max(1);
            } else {
                tokens.push(tok);
                dec = self.dec_step(tok, &mut state);
                same_frame = if dur == 0 { same_frame + 1 } else { 0 };
                if same_frame >= self.cfg.max_symbols_per_step {
                    (dur, same_frame) = (1, 0);
                }
            }
            t += dur;
        }
        tokens
    }

    fn detok(&self, ids: &[usize]) -> String {
        let text: String = ids
            .iter()
            .map(|&i| self.cfg.vocab[i].as_str())
            .filter(|p| !(p.starts_with('<') && p.ends_with('>')))
            .collect();
        text.replace('\u{2581}', " ").trim().to_string()
    }

    fn transcribe_chunk(&mut self, wave: &[f32]) -> Result<String> {
        let pad = (self.cfg.pad_s * SR as f32) as usize;
        let mut padded = vec![0f32; pad];
        padded.extend_from_slice(wave);
        let (frames, feats) = self.features(&padded);
        let input = Array3::from_shape_vec((1, frames, self.mel.rows), feats)?;
        let outputs = self.session.run(ort::inputs!["features" => TensorRef::from_array_view(&input)?])?;
        let enc = outputs["encoded"].try_extract_array::<f32>()?;
        let steps = enc.shape()[1];
        let enc: Vec<f32> = enc.iter().copied().collect();
        drop(outputs);
        let ids = self.greedy_tdt(&enc, steps);
        Ok(self.detok(&ids))
    }

    /// 16 kHz mono in, text out. Long audio is cut at quiet points.
    pub fn transcribe(&mut self, wave: &[f32]) -> Result<String> {
        let mut parts = Vec::new();
        for chunk in split_on_silence(wave, MAX_CHUNK_S, MAX_CHUNK_S * 0.6) {
            let t = self.transcribe_chunk(chunk)?;
            if !t.is_empty() {
                parts.push(t);
            }
        }
        Ok(parts.join(" "))
    }
}

/// ONNX Runtime is loaded from onnxruntime.dll at runtime (the static prebuilt needs a newer MSVC STL).
/// Looked up in: $ORT_DYLIB_PATH, next to the exe, then the models folder.
pub(crate) fn ort_env(model_dir: &Path) -> Result<ort::environment::Environment> {
    static DLL: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let dll = DLL.get_or_init(|| {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
        std::env::var_os("ORT_DYLIB_PATH")
            .map(PathBuf::from)
            .into_iter()
            .chain(exe_dir.map(|d| d.join("onnxruntime.dll")))
            .chain(model_dir.parent().map(|d| d.join("onnxruntime.dll")))
            .find(|p| p.exists())
            .unwrap_or_else(|| PathBuf::from("onnxruntime.dll"))
    });
    Ok(ort::init_from(dll).map_err(|e| anyhow::anyhow!("loading {}: {e}", dll.display()))?.build()?)
}

fn argmax(x: &[f32]) -> usize {
    let mut best = 0;
    for (i, &v) in x.iter().enumerate() {
        if v > x[best] {
            best = i;
        }
    }
    best
}

/// Cut audio longer than max_s at the quietest 30 ms frame between min_s and max_s.
pub fn split_on_silence(wave: &[f32], max_s: f32, min_s: f32) -> Vec<&[f32]> {
    let (frame, max_n, min_n) = (SR * 3 / 100, (max_s * SR as f32) as usize, (min_s * SR as f32) as usize);
    let (mut chunks, mut start) = (Vec::new(), 0usize);
    while wave.len() - start > max_n {
        let (lo, hi) = (start + min_n, start + max_n);
        let n = (hi - lo) / frame;
        let quietest = (0..n)
            .min_by(|&a, &b| energy(&wave[lo + a * frame..lo + (a + 1) * frame]).total_cmp(&energy(&wave[lo + b * frame..lo + (b + 1) * frame])))
            .unwrap_or(0);
        let cut = lo + quietest * frame + frame / 2;
        chunks.push(&wave[start..cut]);
        start = cut;
    }
    chunks.push(&wave[start..]);
    chunks
}

fn energy(x: &[f32]) -> f32 {
    x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32
}

/// Linear resample + downmix to 16 kHz mono (fine for speech).
pub fn to_16k_mono(interleaved: &[f32], channels: usize, rate: usize) -> Vec<f32> {
    let mono: Vec<f32> = if channels <= 1 {
        interleaved.to_vec()
    } else {
        interleaved.chunks_exact(channels).map(|c| c.iter().sum::<f32>() / channels as f32).collect()
    };
    if rate == SR || mono.len() < 2 {
        return mono;
    }
    let n = mono.len() * SR / rate;
    let step = (mono.len() - 1) as f64 / (n.max(2) - 1) as f64;
    (0..n)
        .map(|i| {
            let p = i as f64 * step;
            let (j, frac) = (p as usize, (p - p.floor()) as f32);
            let next = mono[(j + 1).min(mono.len() - 1)];
            mono[j] + (next - mono[j]) * frac
        })
        .collect()
}

/// Lazy-loading, thread-safe wrapper: loads on first use, frees the model after `idle_unload` unused.
pub struct Engine {
    dir: PathBuf,
    threads: usize,
    idle_unload: Duration,
    inner: Arc<Mutex<(Option<BarkModel>, Instant)>>,
}

impl Engine {
    pub fn new(dir: PathBuf, threads: usize, idle_unload: Duration) -> Self {
        let inner = Arc::new(Mutex::new((None, Instant::now())));
        let weak = Arc::downgrade(&inner);
        std::thread::spawn(move || {
            while let Some(inner) = weak.upgrade() {
                {
                    let mut g: parking_lot::MutexGuard<'_, (Option<BarkModel>, Instant)> = inner.lock();
                    if g.0.is_some() && !idle_unload.is_zero() && g.1.elapsed() > idle_unload {
                        g.0 = None;
                    }
                }
                drop(inner);
                std::thread::sleep(Duration::from_secs(30));
            }
        });
        Self { dir, threads, idle_unload, inner }
    }

    pub fn preload(&self) -> Result<()> {
        let mut g = self.inner.lock();
        if g.0.is_none() {
            g.0 = Some(BarkModel::load(&self.dir, self.threads)?);
        }
        g.1 = Instant::now();
        Ok(())
    }

    pub fn transcribe(&self, wave: &[f32]) -> Result<String> {
        let mut g = self.inner.lock();
        if g.0.is_none() {
            g.0 = Some(BarkModel::load(&self.dir, self.threads)?);
        }
        let text = g.0.as_mut().unwrap().transcribe(wave);
        g.1 = Instant::now();
        text
    }

    pub fn is_loaded(&self) -> bool {
        self.inner.lock().0.is_some()
    }

    pub fn idle_unload(&self) -> Duration {
        self.idle_unload
    }
}
