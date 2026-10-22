//! Summary-mode spike: runs every meeting in tools/summary_cases through the three summary modes for each
//! candidate model, checks action-item recall and grounded citations, and writes models/summary_spike.md.
//!
//!     cargo run -p earshot --release --example summary_spike [model.gguf ...]
//! With no arguments: LFM2-2.6B-Transcript, Qwen3-4B-Instruct-2507 and LFM2-8B-A1B from models/llm.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use earshot_lib::config::Config;
use earshot_lib::library;
use earshot_lib::llm::Pool;
use earshot_lib::summary::{Mode, summarize};
use regex::Regex;

const DEFAULT_MODELS: [&str; 3] =
    ["LFM2-2.6B-Transcript-Q4_K_M.gguf", "Qwen3-4B-Instruct-2507-Q4_K_M.gguf", "LFM2-8B-A1B-Q4_K_M.gguf"];

fn section<'a>(md: &'a str, heading: &str) -> &'a str {
    let Some(a) = md.find(heading) else { return "" };
    let rest = &md[a + heading.len()..];
    &rest[..rest.find("\n###").unwrap_or(rest.len())]
}

fn main() -> anyhow::Result<()> {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let cfg = Config::load();
    let llm_dir = repo.join("models").join("llm");
    let args: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let models: Vec<PathBuf> = if args.is_empty() { DEFAULT_MODELS.iter().map(|m| llm_dir.join(m)).collect() } else { args };
    let mut cases: Vec<PathBuf> = std::fs::read_dir(repo.join("tools").join("summary_cases"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    cases.sort();
    let server = repo.join("models").join("llama.cpp").join("llama-server.exe");
    let pool = Pool::new(server, cfg.llm.threads, Duration::from_secs(60));
    let stamp = Regex::new(r"\[(\d\d:\d\d)\]")?;

    let mut out = String::from("# Summary spike\n\n");
    let mut table = String::from("| Model | Case | Mode | Seconds | Action recall | Grounded |\n|---|---|---|---|---|---|\n");
    for model in &models {
        let name = model.file_stem().unwrap().to_string_lossy().to_string();
        if !model.exists() {
            eprintln!("skip {name}: not found");
            continue;
        }
        for case in &cases {
            let text = std::fs::read_to_string(case)?;
            let expect: Vec<String> = text
                .lines()
                .find_map(|l| l.strip_prefix("expect:"))
                .map(|e| e.split('|').map(|s| s.trim().to_lowercase()).collect())
                .unwrap_or_default();
            let lines = library::transcript(case)?;
            let times: Vec<String> = lines.iter().map(|l| format!("{:02}:{:02}", l.start as u32 / 60, l.start as u32 % 60)).collect();
            let case_name = case.file_stem().unwrap().to_string_lossy().to_string();
            let head = format!("Title: {case_name}");
            for mode in Mode::ALL {
                let t = Instant::now();
                let md = match summarize(&pool, model, cfg.llm.ctx, &lines, &head, mode) {
                    Ok(md) => md,
                    Err(e) => format!("ERROR: {e:#}"),
                };
                let secs = t.elapsed().as_secs_f32();
                let actions = section(&md, "### Action items").to_lowercase();
                let hit = expect.iter().filter(|k| actions.contains(k.as_str())).count();
                let missed: Vec<&str> = expect.iter().filter(|k| !actions.contains(k.as_str())).map(String::as_str).collect();
                let grounded = if mode == Mode::Nuanced {
                    let between = section(&md, "### Between the lines");
                    let bullets: Vec<&str> = between.lines().filter(|l| l.trim_start().starts_with('-')).collect();
                    let ok = bullets.iter().filter(|b| stamp.captures_iter(b).any(|c| times.contains(&c[1].to_string()))).count();
                    format!("{ok}/{}", bullets.len())
                } else {
                    "–".into()
                };
                let recall = if missed.is_empty() { format!("{hit}/{}", expect.len()) } else { format!("{hit}/{} (missed: {})", expect.len(), missed.join(", ")) };
                println!("{name:40} {case_name:15} {:9} {secs:6.1}s recall {recall} grounded {grounded}", mode.name());
                writeln!(table, "| {name} | {case_name} | {} | {secs:.1} | {recall} | {grounded} |", mode.name())?;
                writeln!(out, "## {name} · {case_name} · {} ({secs:.1} s)\n\n{md}\n", mode.name())?;
            }
        }
        pool.shutdown();
    }
    let report = format!("{table}\n{out}");
    let path: &Path = &repo.join("models").join("summary_spike.md");
    std::fs::write(path, report)?;
    println!("\nwrote {}", path.display());
    Ok(())
}
