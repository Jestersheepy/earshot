//! First run: download the models into %LOCALAPPDATA%\Earshot\models, reporting progress to the panel's
//! setup page. Downloads resume where they stopped, and every file is checked against its SHA-256.
//! The speech model comes first, so dictation works while the rest is still downloading.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use parking_lot::Mutex;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

use crate::config::Config;
use crate::controller::AppEvent;

pub struct Model {
    /// Under the models folder.
    pub path: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    /// What it's for, as the setup page names it.
    pub part: &'static str,
}

/// In download order. The speech model is our ONNX export of Underdog Bark (CC-BY-4.0, see its NOTICE);
/// the others come from where their authors publish them, pinned to a commit.
pub const MODELS: &[Model] = &[
    Model {
        path: "bark-onnx/encoder.int8.onnx",
        url: "https://github.com/Jestersheepy/earshot/releases/download/models-v1/encoder.int8.onnx",
        size: 839_344_525,
        sha256: "7690b8de8560457af47a7c7ce7c928628b43a9dc567ebb071e20d27b0216e726",
        part: "Speech model",
    },
    Model {
        path: "bark-onnx/decoder.safetensors",
        url: "https://github.com/Jestersheepy/earshot/releases/download/models-v1/decoder.safetensors",
        size: 49_021_688,
        sha256: "1698c1c0ceb61fb96f230716733d409fc9fb7b6a110939cb962167c370a7fbdd",
        part: "Speech model",
    },
    Model {
        path: "bark-onnx/config.json",
        url: "https://github.com/Jestersheepy/earshot/releases/download/models-v1/config.json",
        size: 115_913,
        sha256: "bc131bcf0e790bc553e540b3137762e234ef21a1bd86c632415f8f1d5c282112",
        part: "Speech model",
    },
    Model {
        path: "bark-onnx/NOTICE",
        url: "https://github.com/Jestersheepy/earshot/releases/download/models-v1/NOTICE",
        size: 626,
        sha256: "caa57aadbe53f9c7bf2e2e9bdd54bf64e5e8f775bc128ac8645350d8aeb1517a",
        part: "Speech model",
    },
    Model {
        path: "speaker/3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx",
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx",
        size: 29_596_978,
        sha256: "357a834f702b80161e5b981182c038e18553c1f2ca752ed6cec2052365d4129b",
        part: "Speaker model",
    },
    Model {
        path: "llm/LFM2-1.2B-Q4_K_M.gguf",
        url: "https://huggingface.co/LiquidAI/LFM2-1.2B-GGUF/resolve/5399e76c648f4eb8c053feb1ab747277dea5bf8b/LFM2-1.2B-Q4_K_M.gguf",
        size: 730_893_248,
        sha256: "55175400e3f509a9616227afeffd58d87e80b9f628a5d3d54ada884d85221fed",
        part: "Formatting model",
    },
    Model {
        path: "llm/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/a06e946bb6b655725eafa393f4a9745d460374c9/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        size: 2_497_281_120,
        sha256: "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597",
        part: SUMMARIES,
    },
];

const SUMMARIES: &str = "Summary model";

#[derive(Serialize, Clone, Default)]
pub struct Progress {
    /// "idle" | "downloading" | "verifying" | "done" | "failed"
    pub state: &'static str,
    pub done: u64,
    pub total: u64,
    pub part: String,
    /// Dictation can be used (the speech model is in place).
    pub speech_ready: bool,
    pub error: Option<String>,
}

static RUNNING: AtomicBool = AtomicBool::new(false);
static LAST: Mutex<Option<Progress>> = Mutex::new(None);

fn present(dir: &Path, m: &Model) -> bool {
    fs::metadata(dir.join(m.path)).is_ok_and(|f| f.len() == m.size)
}

/// Models still to download; the summary model only if `summaries` (someone using a cloud model may skip it).
pub fn missing(cfg: &Config, summaries: bool) -> Vec<&'static Model> {
    let dir = cfg.models_dir();
    MODELS.iter().filter(|m| (summaries || m.part != SUMMARIES) && !present(&dir, m)).collect()
}

pub fn speech_ready(cfg: &Config) -> bool {
    let dir = cfg.models_dir();
    MODELS.iter().filter(|m| m.path.starts_with("bark-onnx/")).all(|m| present(&dir, m))
}

pub fn summary_bytes(cfg: &Config) -> u64 {
    let dir = cfg.models_dir();
    MODELS.iter().filter(|m| m.part == SUMMARIES && !present(&dir, m)).map(|m| m.size).sum()
}

#[derive(Serialize)]
pub struct Status {
    /// Bytes to download for everything except the summary model.
    pub needed: u64,
    /// Bytes for the summary model, if it's missing.
    pub summaries: u64,
    pub speech_ready: bool,
    pub progress: Option<Progress>,
}

pub fn status(cfg: &Config) -> Status {
    let dir = cfg.models_dir();
    let needed = MODELS.iter().filter(|m| m.part != SUMMARIES && !present(&dir, m)).map(|m| m.size).sum();
    Status { needed, summaries: summary_bytes(cfg), speech_ready: speech_ready(cfg), progress: LAST.lock().clone() }
}

/// Download what's missing on a background thread (once), with "setup-progress" events for the page.
/// Tells the controller "models-ready" when it's done.
pub fn start(app: AppHandle, cfg: &Config, summaries: bool, tx: Sender<AppEvent>) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let dir = cfg.models_dir();
    let todo = missing(cfg, summaries);
    let mut p = Progress {
        state: "downloading",
        total: todo.iter().map(|m| m.size).sum(),
        speech_ready: speech_ready(cfg),
        ..Default::default()
    };
    std::thread::spawn(move || {
        let emit = |p: &Progress| {
            *LAST.lock() = Some(p.clone());
            let _ = app.emit("setup-progress", p);
        };
        let mut base = 0;
        let mut result = Ok(());
        for m in &todo {
            p.part = m.part.to_string();
            p.state = "downloading";
            let mut last = Instant::now() - Duration::from_secs(1);
            let r = fetch(m, &dir, |done, verifying| {
                p.done = base + done;
                if verifying {
                    p.state = "verifying";
                }
                if verifying || last.elapsed() >= Duration::from_millis(120) {
                    last = Instant::now();
                    emit(&p);
                }
            });
            if let Err(e) = r {
                eprintln!("downloading {}: {e:#}", m.path);
                result = Err(e);
                break;
            }
            base += m.size;
            if m.path.starts_with("bark-onnx/") && MODELS.iter().filter(|m| m.path.starts_with("bark-onnx/")).all(|m| present(&dir, m)) {
                p.speech_ready = true;
                let _ = tx.send(AppEvent::Action("speech-ready".into()));
            }
        }
        match result {
            Ok(()) => {
                p.state = "done";
                p.done = p.total;
                let _ = tx.send(AppEvent::Action("models-ready".into()));
            }
            Err(e) => {
                p.state = "failed";
                p.error = Some(format!("{e:#}"));
            }
        }
        emit(&p);
        RUNNING.store(false, Ordering::SeqCst);
    });
}

/// One file into place: resume a partial download, check the hash, then move it in.
/// `on(bytes of this file so far, verifying)`.
fn fetch(m: &Model, dir: &Path, mut on: impl FnMut(u64, bool)) -> Result<()> {
    let dest = dir.join(m.path);
    fs::create_dir_all(dest.parent().context("bad path")?)?;
    let part = PathBuf::from(format!("{}.part", dest.display()));
    let mut done = fs::metadata(&part).map_or(0, |f| f.len());
    if done > m.size {
        fs::remove_file(&part)?;
        done = 0;
    }
    if done < m.size {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .build()
            .into();
        let mut req = agent.get(m.url);
        if done > 0 {
            req = req.header("range", &format!("bytes={done}-"));
        }
        let resp = req.call().with_context(|| format!("couldn't reach {}", host(m.url)))?;
        let mut file = match resp.status().as_u16() {
            206 => OpenOptions::new().append(true).open(&part)?,
            200 => {
                done = 0;
                File::create(&part)?
            }
            s => bail!("{} answered HTTP {s}", host(m.url)),
        };
        let mut body = resp.into_body().into_reader();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = body.read(&mut buf).context("download interrupted")?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            done += n as u64;
            on(done, false);
        }
        file.flush()?;
        ensure!(done == m.size, "download interrupted ({done} of {} bytes)", m.size);
    }
    on(done, true);
    if sha256(&part)? != m.sha256 {
        let _ = fs::remove_file(&part);
        bail!("{} was damaged in transit; try again", m.path);
    }
    fs::rename(&part, &dest)?;
    Ok(())
}

fn sha256(path: &Path) -> Result<String> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn host(url: &str) -> &str {
    url.split('/').nth(2).unwrap_or(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_sane() {
        for m in MODELS {
            assert_eq!(m.sha256.len(), 64, "{}", m.path);
            assert!(m.url.starts_with("https://") && m.url.ends_with(m.path.rsplit('/').next().unwrap()), "{}", m.url);
        }
        // the speech model downloads first so dictation works soonest
        assert!(MODELS[0].path.starts_with("bark-onnx/"));
    }

    /// Needs the internet: `cargo test -p earshot --lib setup -- --ignored`
    #[test]
    #[ignore]
    fn downloads_resumes_and_verifies() {
        let m = MODELS.iter().find(|m| m.path.starts_with("speaker/")).unwrap();
        let dir = std::env::temp_dir().join("earshot-setup-test");
        let _ = fs::remove_dir_all(&dir);
        fetch(m, &dir, |_, _| {}).unwrap();
        let dest = dir.join(m.path);
        assert_eq!(sha256(&dest).unwrap(), m.sha256);
        // a download cut off part way picks up from there
        let bytes = fs::read(&dest).unwrap();
        fs::remove_file(&dest).unwrap();
        fs::write(format!("{}.part", dest.display()), &bytes[..5_000_000]).unwrap();
        let mut first = None;
        fetch(m, &dir, |done, _| {
            first.get_or_insert(done);
        })
        .unwrap();
        assert!(first.unwrap() > 5_000_000, "restarted from {first:?}");
        assert_eq!(sha256(&dest).unwrap(), m.sha256);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hashes_a_file() {
        let p = std::env::temp_dir().join("earshot-sha-test");
        fs::write(&p, b"abc").unwrap();
        assert_eq!(sha256(&p).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let _ = fs::remove_file(p);
    }
}
