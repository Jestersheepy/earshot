//! Speaker clustering on a conversation between three synthetic voices (models/speakers_test, written by
//! Windows TTS; the voice is the last part of each file name). Skipped when the files aren't present.

use std::path::PathBuf;

use earshot_core::speakers::{Embedder, SAME_VOICE, cluster, dot};
use earshot_core::stt::SR;

#[test]
fn separates_three_voices() {
    let models = std::env::var("EARSHOT_MODELS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"));
    let Ok(dir) = std::fs::read_dir(models.join("speakers_test")) else {
        eprintln!("skipping: no models/speakers_test");
        return;
    };
    let mut wavs: Vec<PathBuf> = dir.filter_map(|e| e.ok().map(|e| e.path())).collect();
    wavs.sort();
    let mut emb = Embedder::load(&models.join("speaker"), 4).unwrap();
    let embs: Vec<Vec<f32>> = wavs
        .iter()
        .map(|p| {
            let mut r = hound::WavReader::open(p).unwrap();
            assert_eq!(r.spec().sample_rate as usize, SR);
            let pcm: Vec<f32> = r.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect();
            emb.embed(&pcm).unwrap()
        })
        .collect();
    let names: Vec<String> = wavs.iter().map(|p| p.file_stem().unwrap().to_string_lossy().rsplit('_').next().unwrap().to_string()).collect();
    for i in 0..embs.len() {
        for j in i + 1..embs.len() {
            let s = dot(&embs[i], &embs[j]);
            if names[i] == names[j] {
                assert!(s > 0.7, "{} vs {}: {s}", names[i], names[j]);
            } else {
                assert!(s < 0.45, "{} vs {}: {s}", names[i], names[j]);
            }
        }
    }
    let labels = cluster(&embs, SAME_VOICE);
    for i in 0..embs.len() {
        for j in 0..embs.len() {
            assert_eq!(labels[i] == labels[j], names[i] == names[j], "{:?} vs {:?}", labels, names);
        }
    }
}
