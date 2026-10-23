//! Small local LLMs through llama.cpp's llama-server, one process per model, started on demand and
//! stopped after `keep_warm` of no use. Nothing is loaded while idle.

use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use serde_json::{Value, json};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

struct Server {
    child: Child,
    port: u16,
    last_used: Instant,
}

#[derive(Clone)]
pub struct Pool {
    exe: PathBuf,
    threads: u32,
    servers: Arc<Mutex<HashMap<PathBuf, Server>>>,
}

impl Pool {
    pub fn new(exe: PathBuf, threads: u32, keep_warm: Duration) -> Self {
        let pool = Self { exe, threads, servers: Arc::default() };
        let weak = Arc::downgrade(&pool.servers);
        std::thread::spawn(move || {
            while let Some(servers) = weak.upgrade() {
                servers.lock().retain(|_, s| {
                    let keep = s.last_used.elapsed() < keep_warm;
                    if !keep {
                        let _ = s.child.kill();
                    }
                    keep
                });
                drop(servers);
                std::thread::sleep(Duration::from_secs(20));
            }
        });
        pool
    }

    pub fn available(&self, model: &Path) -> bool {
        self.exe.exists() && model.exists()
    }

    /// Load the model now (e.g. when dictation starts) so the first request doesn't wait.
    pub fn warm(&self, model: &Path, ctx: u32) {
        let (this, model) = (self.clone(), model.to_path_buf());
        std::thread::spawn(move || {
            let _ = this.port(&model, ctx);
        });
    }

    fn port(&self, model: &Path, ctx: u32) -> Result<u16> {
        let mut servers = self.servers.lock();
        if let Some(s) = servers.get_mut(model) {
            if s.child.try_wait()?.is_none() {
                s.last_used = Instant::now();
                return Ok(s.port);
            }
            servers.remove(model);
        }
        let port = free_port()?;
        let child = Command::new(&self.exe)
            .args(["-m", &model.to_string_lossy(), "-c", &ctx.to_string(), "-t", &self.threads.to_string()])
            .args(["--host", "127.0.0.1", "--port", &port.to_string(), "--no-webui", "--reasoning", "off"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .with_context(|| format!("starting {}", self.exe.display()))?;
        servers.insert(model.to_path_buf(), Server { child, port, last_used: Instant::now() });
        drop(servers);
        let deadline = Instant::now() + Duration::from_secs(120);
        let url = format!("http://127.0.0.1:{port}/health");
        while Instant::now() < deadline {
            if ureq::get(&url).call().is_ok_and(|r| r.status() == 200) {
                return Ok(port);
            }
            if let Some(s) = self.servers.lock().get_mut(model) {
                if s.child.try_wait()?.is_some() {
                    bail!("llama-server exited while loading {}", model.display());
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        bail!("llama-server did not become ready")
    }

    pub fn chat(&self, model: &Path, ctx: u32, system: &str, user: &str, max_tokens: u32, temperature: f32) -> Result<String> {
        let messages = vec![json!({"role": "system", "content": system}), json!({"role": "user", "content": user})];
        self.chat_messages(model, ctx, messages, max_tokens, temperature)
    }

    /// Full message list, e.g. few-shot user/assistant pairs before the real request.
    pub fn chat_messages(&self, model: &Path, ctx: u32, messages: Vec<Value>, max_tokens: u32, temperature: f32) -> Result<String> {
        let body = json!({
            "messages": messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
            "cache_prompt": true,
        });
        self.complete(model, ctx, body)
    }

    /// A reply held to a JSON schema by llama.cpp's grammar, so even a small model keeps to the shape.
    pub fn chat_json(&self, model: &Path, ctx: u32, system: &str, user: &str, schema: Value, max_tokens: u32) -> Result<Value> {
        let body = json!({
            "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
            "temperature": 0.0,
            "max_tokens": max_tokens,
            "cache_prompt": true,
            "json_schema": schema,
        });
        Ok(serde_json::from_str(&self.complete(model, ctx, body)?)?)
    }

    fn complete(&self, model: &Path, ctx: u32, body: Value) -> Result<String> {
        let port = self.port(model, ctx)?;
        let mut resp = ureq::post(&format!("http://127.0.0.1:{port}/v1/chat/completions"))
            .config()
            .timeout_global(Some(Duration::from_secs(1800)))
            .build()
            .send_json(&body)?;
        let v: Value = resp.body_mut().read_json()?;
        if let Some(s) = self.servers.lock().get_mut(model) {
            s.last_used = Instant::now();
        }
        let text = v["choices"][0]["message"]["content"].as_str().context("no completion")?;
        Ok(strip_think(text).trim().to_string())
    }

    pub fn shutdown(&self) {
        for (_, mut s) in self.servers.lock().drain() {
            let _ = s.child.kill();
        }
    }
}

/// Qwen-style models may emit <think>…</think> even when asked not to.
pub(crate) fn strip_think(s: &str) -> &str {
    match s.find("</think>") {
        Some(i) => &s[i + 8..],
        None => s,
    }
}

fn free_port() -> Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}
