"""Markdown notes on disk: one .md (plus optional .wav) per voice note or meeting."""
import re
from datetime import datetime
from pathlib import Path

FILLERS = re.compile(r"\b(?:um+|uh+|erm|uhm|hmm+)\b[,.]?\s*", re.IGNORECASE)


def clean(text, drop_fillers=True):
    if drop_fillers:
        text = FILLERS.sub("", text)
    text = re.sub(r"\s{2,}", " ", text).strip()
    return text[:1].upper() + text[1:]


def new_note_path(notes_dir, folder, started):
    d = Path(notes_dir) / folder
    d.mkdir(parents=True, exist_ok=True)
    base = started.strftime("%Y-%m-%d %H%M")
    path, n = d / f"{base}.md", 2
    while path.exists():
        path, n = d / f"{base} ({n}).md", n + 1
    return path


def write_note(path, kind, started, duration_s, body, extra=None):
    meta = {"type": kind, "date": started.isoformat(timespec="seconds"), "duration": fmt_duration(duration_s)}
    meta.update(extra or {})
    front = "\n".join(f"{k}: {v}" for k, v in meta.items())
    path.write_text(f"---\n{front}\n---\n\n{body.strip()}\n", encoding="utf-8")
    return path


def fmt_duration(s):
    m, s = divmod(int(s), 60)
    h, m = divmod(m, 60)
    return f"{h}:{m:02d}:{s:02d}" if h else f"{m}:{s:02d}"


def now():
    return datetime.now()
