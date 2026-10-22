//! Smart-format model spike: runs tools/format_cases.json through the app's real dictation pipeline
//! (snippets, dictionary, pre-pass, few-shot LLM, drift guard) for each candidate model.
//!
//!     cargo run -p earshot --release --example format_spike [model.gguf ...]
//! With no arguments, every .gguf in models/llm except the summary model. Details: models/format_spike.json.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use earshot_lib::config::Config;
use earshot_lib::llm::Pool;
use earshot_lib::text::smart_format;
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Case {
    name: String,
    input: String,
    #[serde(default)]
    must: Vec<String>,
    #[serde(default)]
    must_not: Vec<String>,
    #[serde(default)]
    bullets: usize,
    #[serde(default)]
    terms: Vec<String>,
    app: Option<String>,
    #[serde(default)]
    new_ok: bool,
}

#[derive(Serialize)]
struct Outcome {
    name: String,
    output: String,
    ms: u128,
    llm: bool,
    fails: Vec<String>,
}

fn words(s: &str) -> Vec<String> {
    Regex::new(r"[\p{L}0-9']+").unwrap().find_iter(&s.to_lowercase()).map(|m| m.as_str().to_string()).collect()
}

fn check(c: &Case, out: &str) -> Vec<String> {
    let low = out.to_lowercase();
    let mut fails = Vec::new();
    for m in &c.must {
        if !m.split('|').any(|alt| out.contains(alt) || low.contains(&alt.to_lowercase())) {
            fails.push(format!("missing {m:?}"));
        }
    }
    for m in &c.must_not {
        let re = Regex::new(&format!(r"(^|[^\p{{L}}]){}([^\p{{L}}]|$)", regex::escape(&m.to_lowercase()))).unwrap();
        if re.is_match(&low) {
            fails.push(format!("has {m:?}"));
        }
    }
    let items = out.lines().filter(|l| Regex::new(r"^\s*([-*•]|\d+[.)])\s").unwrap().is_match(l)).count();
    if items < c.bullets {
        fails.push(format!("{items} list items"));
    }
    if !c.new_ok {
        let said: std::collections::HashSet<String> = words(&c.input).into_iter().chain(c.terms.iter().map(|t| t.to_lowercase())).collect();
        let new: Vec<String> = words(out).into_iter().filter(|w| !said.contains(w) && !w.chars().all(|ch| ch.is_ascii_digit())).collect();
        if new.len() > 2.max(words(out).len() / 8) {
            fails.push(format!("added {:?}", &new[..new.len().min(5)]));
        }
    }
    fails
}

fn main() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let models_dir = root.join("models");
    let cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(root.join("tools/format_cases.json"))?)?;
    let mut models: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if models.is_empty() {
        models = std::fs::read_dir(models_dir.join("llm"))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "gguf") && !p.to_string_lossy().contains("Transcript"))
            .collect();
        models.sort();
    }
    let mut report = serde_json::Map::new();
    for model in models {
        let name = model.file_name().unwrap().to_string_lossy().into_owned();
        let mut cfg = Config { models_dir: Some(models_dir.clone()), smart_format: true, ..Config::default() };
        cfg.llm.format_model = name.clone();
        let pool = Pool::new(cfg.llama_server(), 8, Duration::from_secs(600));
        let t = Instant::now();
        smart_format(&pool, &cfg, "Um, Monday, no wait, Tuesday.", None); // load + fill the prompt cache
        let load = t.elapsed();
        let mut results = Vec::new();
        for c in &cases {
            cfg.dictionary = c.terms.clone();
            let t = Instant::now();
            let (output, llm) = smart_format(&pool, &cfg, &c.input, c.app.as_deref());
            let ms = t.elapsed().as_millis();
            results.push(Outcome { name: c.name.clone(), fails: check(c, &output), output, ms, llm });
        }
        pool.shutdown();
        let mut llm_ms: Vec<u128> = results.iter().filter(|r| r.llm).map(|r| r.ms).collect();
        llm_ms.sort();
        let passed = results.iter().filter(|r| r.fails.is_empty()).count();
        println!(
            "{name:40} {passed:2}/{} pass   LLM on {:2}: median {:4} ms  max {:4} ms   first call {:.1}s",
            results.len(),
            llm_ms.len(),
            llm_ms.get(llm_ms.len() / 2).copied().unwrap_or(0),
            llm_ms.last().copied().unwrap_or(0),
            load.as_secs_f32()
        );
        for r in results.iter().filter(|r| !r.fails.is_empty()) {
            println!("    [{}] {:?} -> {:?}", r.name, r.fails, r.output.chars().take(120).collect::<String>());
        }
        report.insert(name, serde_json::to_value(&results)?);
    }
    std::fs::write(models_dir.join("format_spike.json"), serde_json::to_string_pretty(&report)?)?;
    Ok(())
}
