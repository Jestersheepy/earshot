//! Where meeting summaries are written: the local model through llama.cpp, or a cloud model the user brings
//! an API key for (Claude, OpenAI, or any OpenAI-compatible server such as OpenRouter, Groq, Ollama or LM Studio).

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde_json::{Value, json};

use crate::config::{Cloud, Config};
use crate::llm::Pool;

pub const PROVIDERS: [&str; 3] = ["anthropic", "openai", "custom"];
const CLAUDE_MODEL: &str = "claude-opus-5-5";
const OPENAI_URL: &str = "https://api.openai.com/v1";

pub enum Summarizer<'a> {
    Local { pool: &'a Pool, model: PathBuf, ctx: u32 },
    Claude { key: String, model: String },
    OpenAi { url: String, key: Option<String>, model: String, ctx: u32 },
}

impl<'a> Summarizer<'a> {
    /// The one Settings picked, or why it can't run.
    pub fn new(cfg: &Config, pool: &'a Pool) -> Result<Self> {
        if !cfg.cloud.enabled {
            let model = cfg.llm_model(&cfg.llm.summary_model);
            ensure!(
                pool.available(&model),
                "the summary model isn't downloaded (Settings › Models), or choose a cloud model in Settings › Meetings"
            );
            return Ok(Self::Local { pool, model, ctx: cfg.llm.ctx });
        }
        Self::cloud(&cfg.cloud, earshot_core::secret::get(&cfg.cloud.provider), cfg.llm.ctx)
    }

    pub fn cloud(c: &Cloud, key: Option<String>, local_ctx: u32) -> Result<Self> {
        let model = c.model.trim().to_string();
        match c.provider.as_str() {
            "anthropic" => Ok(Self::Claude {
                key: key.context("Add your Anthropic API key in Settings › Meetings")?,
                model: if model.is_empty() { CLAUDE_MODEL.into() } else { model },
            }),
            "openai" => {
                ensure!(!model.is_empty(), "Choose an OpenAI model in Settings › Meetings");
                let key = key.context("Add your OpenAI API key in Settings › Meetings")?;
                Ok(Self::OpenAi { url: OPENAI_URL.into(), key: Some(key), model, ctx: 100_000 })
            }
            _ => {
                let url = c.url.trim().trim_end_matches('/').to_string();
                ensure!(!url.is_empty() && !model.is_empty(), "Enter the server's URL and model in Settings › Meetings");
                // a server we know nothing about: assume a context as small as the local model's
                Ok(Self::OpenAi { url, key, model, ctx: local_ctx })
            }
        }
    }

    /// Tokens of transcript one prompt can hold; longer meetings are condensed in parts first.
    pub fn ctx(&self) -> u32 {
        match self {
            Self::Local { ctx, .. } | Self::OpenAi { ctx, .. } => *ctx,
            Self::Claude { .. } => 200_000,
        }
    }

    /// `max_tokens` and `temperature` tune the local model. Cloud models get their own defaults: current
    /// Claude models take no sampling settings, and OpenAI's reasoning models refuse them too.
    pub fn chat(&self, system: &str, user: &str, max_tokens: u32, temperature: f32) -> Result<String> {
        match self {
            Self::Local { pool, model, ctx } => pool.chat(model, *ctx, system, user, max_tokens, temperature),
            Self::Claude { key, model } => claude(key, model, system, user),
            Self::OpenAi { url, key, model, .. } => openai(url, key.as_deref(), model, system, user),
        }
    }
}

fn claude(key: &str, model: &str, system: &str, user: &str) -> Result<String> {
    let mut body = json!({
        "model": model,
        "max_tokens": 16000,
        "system": system,
        "messages": [{"role": "user", "content": user}],
    });
    let mut headers = vec![("x-api-key", key), ("anthropic-version", "2026-10-21")];
    // If the model declines, let the API retry on another Claude model rather than lose the summary.
    if ["claude-opus-5", "claude-fable-5", "claude-sonnet-5-5"].iter().any(|m| model.starts_with(m)) {
        body["fallbacks"] = json!("default");
        headers.push(("anthropic-beta", "server-side-fallback-2026-07-01"));
    }
    let v = post("https://api.anthropic.com/v1/messages", &headers, &body).map_err(|e| anyhow!("Claude: {e}"))?;
    if v["stop_reason"] == "refusal" {
        bail!("Claude declined to summarize this transcript");
    }
    let blocks = v["content"].as_array().context("Claude: empty reply")?;
    let text: String = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
    Ok(text.trim().to_string())
}

fn openai(url: &str, key: Option<&str>, model: &str, system: &str, user: &str) -> Result<String> {
    let body = json!({
        "model": model,
        "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
    });
    let auth = key.map(|k| format!("Bearer {k}"));
    let headers: Vec<(&str, &str)> = auth.as_deref().map(|a| ("authorization", a)).into_iter().collect();
    let v = post(&format!("{url}/chat/completions"), &headers, &body)?;
    let text = v["choices"][0]["message"]["content"].as_str().context("no completion")?;
    Ok(crate::llm::strip_think(text).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs the internet: `cargo test -p earshot --lib backend -- --ignored`
    #[test]
    #[ignore]
    fn bad_key_reports_the_services_message() {
        let c = Cloud { enabled: true, provider: "anthropic".into(), ..Default::default() };
        let e = Summarizer::cloud(&c, Some("sk-ant-invalid".into()), 4096).unwrap().chat("s", "u", 16, 0.0).unwrap_err();
        let e = e.to_string();
        assert!(e.starts_with("Claude: ") && e.contains("401"), "{e}");
    }
}

/// POST JSON, retrying twice when the service is busy. Errors carry the service's own message.
fn post(url: &str, headers: &[(&str, &str)], body: &Value) -> Result<Value> {
    let agent: ureq::Agent =
        ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(600))).build().into();
    let mut attempt = 0;
    loop {
        let mut req = agent.post(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let retry = |attempt: u32| std::thread::sleep(Duration::from_secs(5 * 3u64.pow(attempt)));
        let mut resp = match req.send_json(body) {
            Ok(r) => r,
            Err(e) if attempt < 2 => {
                eprintln!("{url}: {e}; retrying");
                retry(attempt);
                attempt += 1;
                continue;
            }
            Err(e) => bail!("couldn't reach the service ({e})"),
        };
        let status = resp.status().as_u16();
        let v: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
        if status == 200 {
            return Ok(v);
        }
        if (status == 429 || status >= 500) && attempt < 2 {
            eprintln!("{url}: HTTP {status}; retrying");
            retry(attempt);
            attempt += 1;
            continue;
        }
        let msg = v["error"]["message"].as_str().or(v["error"].as_str()).unwrap_or("request failed");
        bail!("{msg} (HTTP {status})");
    }
}
