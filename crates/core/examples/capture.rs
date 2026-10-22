//! Records mic + system audio for N seconds and transcribes both: `cargo run --example capture -- 5`
use std::time::{Duration, Instant};

use earshot_core::audio::{Capture, Source};
use earshot_core::stt::{BarkModel, SR};

fn main() -> anyhow::Result<()> {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    let t0 = Instant::now();
    let mic = Capture::start(Source::Mic, t0)?;
    let lb = Capture::start(Source::Loopback, t0)?;
    std::thread::sleep(Duration::from_secs(secs));
    let (me, others) = (mic.stop(), lb.stop());
    let mut model = BarkModel::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/bark-onnx"), 8)?;
    for (name, t) in [("Me", me), ("Others", others)] {
        let w = t.read(0, None);
        let rms = (w.iter().map(|v| v * v).sum::<f32>() / w.len().max(1) as f32).sqrt();
        println!("{name}: {:.1}s rms {rms:.4} -> {}", w.len() as f32 / SR as f32, model.transcribe(&w)?);
    }
    Ok(())
}
