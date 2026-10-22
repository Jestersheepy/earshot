"""Meeting summary with a small local LLM. llama-server is started only for the summary, then killed.

    python -m earshot.summarize "path/to/meeting.md"
"""
import json
import subprocess
import time
import urllib.request
from pathlib import Path

SYSTEM = ("You are an expert meeting analyst. Analyze the transcript carefully and provide clear, accurate "
          "information based on the content.")
ASK = ("Provide a brief executive summary (2-3 sentences) of the key outcomes and decisions from this transcript. "
       "Then list the key decisions that were made. Then list the specific action items that were assigned during "
       "this meeting, including who is responsible for each item when mentioned. Use the markdown headings "
       "'### Summary', '### Decisions' and '### Action items'.")
ASK_PART = "Provide a detailed summary of this part of the transcript, covering all topics, decisions and action items."
CHARS_PER_TOKEN = 4


def available(llm_cfg):
    return Path(llm_cfg["server"]).exists() and Path(llm_cfg["model"]).exists()


class LlamaServer:
    def __init__(self, cfg):
        self.cfg, self.url = cfg, f"http://127.0.0.1:{cfg['port']}"

    def __enter__(self):
        c = self.cfg
        self.proc = subprocess.Popen(
            [c["server"], "-m", c["model"], "-c", str(c["ctx"]), "-t", str(c["threads"]),
             "--host", "127.0.0.1", "--port", str(c["port"])],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, creationflags=subprocess.CREATE_NO_WINDOW)
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            if self.proc.poll() is not None:
                raise RuntimeError("llama-server exited during startup")
            try:
                if urllib.request.urlopen(self.url + "/health", timeout=1).status == 200:
                    return self
            except OSError:
                time.sleep(0.5)
        raise TimeoutError("llama-server did not become ready")

    def chat(self, user, max_tokens=1024):
        body = json.dumps({"messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": user}],
                           "temperature": 0.3, "max_tokens": max_tokens}).encode()
        req = urllib.request.Request(self.url + "/v1/chat/completions", body, {"Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=1800) as r:
            return json.load(r)["choices"][0]["message"]["content"].strip()

    def __exit__(self, *exc):
        self.proc.terminate()
        try:
            self.proc.wait(10)
        except subprocess.TimeoutExpired:
            self.proc.kill()


def summarize(llm_cfg, lines, header):
    """lines: [(start_s, speaker, text)]. header: dict of Title/Date/Duration/Participants."""
    transcript = [f"**{s}**: {x}" for _, s, x in lines]
    head = "\n".join(f"{k}: {v}" for k, v in header.items())
    budget = int(llm_cfg["ctx"] * 0.6) * CHARS_PER_TOKEN  # leave room for the prompt and the answer
    with LlamaServer(llm_cfg) as llm:
        parts = _split(transcript, budget)
        if len(parts) == 1:
            return llm.chat(f"{ASK}\n\n{head}\n----------\n" + "\n".join(parts[0]))
        partials = [llm.chat(f"{ASK_PART}\n\n{head}\n----------\n" + "\n".join(p)) for p in parts]
        merged = "\n\n".join(f"Part {i + 1}:\n{p}" for i, p in enumerate(partials))
        return llm.chat(f"{ASK}\n\nThe transcript was long, so here are summaries of its parts in order.\n\n"
                        f"{head}\n----------\n{merged}")


def _split(lines, budget):
    parts, cur, size = [], [], 0
    for line in lines:
        if cur and size + len(line) > budget:
            parts.append(cur)
            cur, size = [], 0
        cur.append(line)
        size += len(line) + 1
    return parts + [cur] if cur else parts


if __name__ == "__main__":
    import re
    import sys

    from earshot import config

    text = Path(sys.argv[1]).read_text(encoding="utf-8")
    rows = [(0, s, x) for s, x in re.findall(r"\*\*\[[\d:]+\] (\w+):\*\* (.+)", text)]
    t = time.time()
    print(summarize(config.load()["llm"], rows, {"Title": "Meeting", "Participants": "Me, Others"}))
    print(f"\n({time.time() - t:.1f}s)")
