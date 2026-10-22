//! Models load again after being dropped, and side by side: ONNX Runtime allows one environment per process,
//! and the speech model is unloaded after idling. Skipped when the models aren't present.

use std::path::PathBuf;

use earshot_core::speakers::Embedder;
use earshot_core::stt::BarkModel;

#[test]
fn models_reload() {
    let models = std::env::var("EARSHOT_MODELS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"));
    if !models.join("bark-onnx").exists() || !models.join("speaker").exists() {
        eprintln!("skipping: no models");
        return;
    }
    let bark = BarkModel::load(&models.join("bark-onnx"), 2).unwrap();
    let speaker = Embedder::load(&models.join("speaker"), 2).unwrap();
    drop(bark);
    drop(speaker);
    BarkModel::load(&models.join("bark-onnx"), 2).unwrap();
}
