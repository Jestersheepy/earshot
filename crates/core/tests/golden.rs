//! The Rust STT must reproduce the Python reference transcripts (models/golden.json, from
//! tools/make_golden.py). Skipped when the models aren't present.

use std::path::PathBuf;
use std::time::Instant;

use earshot_core::stt::{BarkModel, SR};

fn models() -> PathBuf {
    std::env::var("EARSHOT_MODELS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"))
}

fn load_wav(p: &PathBuf) -> Vec<f32> {
    let mut r = hound::WavReader::open(p).unwrap();
    assert_eq!(r.spec().sample_rate as usize, SR);
    r.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect()
}

#[test]
fn matches_python_reference() {
    let dir = models();
    let Ok(golden) = std::fs::read(dir.join("golden.json")) else {
        eprintln!("skipping: no models/golden.json");
        return;
    };
    let cases: Vec<serde_json::Value> = serde_json::from_slice(&golden).unwrap();
    let t = Instant::now();
    let mut model = BarkModel::load(&dir.join("bark-onnx"), 8).unwrap();
    eprintln!("load {:.2}s", t.elapsed().as_secs_f32());
    let (mut audio_s, mut stt_s, mut failures) = (0.0, 0.0, 0);
    for case in &cases {
        let wave: Vec<f32> =
            case["wavs"].as_array().unwrap().iter().flat_map(|w| load_wav(&dir.join(w.as_str().unwrap()))).collect();
        let t = Instant::now();
        let got = model.transcribe(&wave).unwrap();
        stt_s += t.elapsed().as_secs_f32();
        audio_s += wave.len() as f32 / SR as f32;
        let want = case["text"].as_str().unwrap();
        if got != want {
            // numpy vs Rust float rounding can flip a near-tie token; allow one word, never more
            let d = word_distance(want, &got);
            eprintln!("DIFF ({d} word) {}\n  want: {want}\n  got:  {got}", case["wavs"]);
            assert!(d <= 1, "transcript drifted from the Python reference");
            failures += 1;
        }
    }
    eprintln!("{} cases, {audio_s:.0}s audio in {stt_s:.2}s ({:.0}x realtime)", cases.len(), audio_s / stt_s);
    assert!(failures * 10 <= cases.len(), "more than 10% of cases differ");
}

fn word_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<_>, Vec<_>) = (a.split_whitespace().collect(), b.split_whitespace().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, y) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(x != y)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}
