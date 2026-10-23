//! Meaning-based similarity with the real model (models/embed). Skipped when the files aren't present.

use std::path::PathBuf;

use earshot_core::embed::{QUERY_PREFIX, TextEmbedder};
use earshot_core::speakers::dot;

#[test]
fn closer_in_meaning_scores_higher() {
    let models = std::env::var("EARSHOT_MODELS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"));
    let dir = models.join("embed");
    if !dir.join("model_quantized.onnx").exists() {
        eprintln!("skipping: no models/embed");
        return;
    }
    let mut emb = TextEmbedder::load(&dir, 4).unwrap();
    let q = format!("{QUERY_PREFIX}the cost feels too high");
    let v = emb.embed(&[&q, "pricing worries", "the weather is sunny"]).unwrap();
    assert_eq!(v[0].len(), 384);
    let (near, far) = (dot(&v[0], &v[1]), dot(&v[0], &v[2]));
    eprintln!("near {near:.3} far {far:.3}");
    assert!(near > far + 0.05);
    // batching with padding gives the same vector as embedding alone
    let solo = emb.embed(&["pricing worries"]).unwrap();
    assert!(dot(&solo[0], &v[1]) > 0.99);
}
