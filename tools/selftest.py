"""Offline end-to-end check of the meeting pipeline (no mic needed).

Builds time-aligned "Me"/"Others" tracks from numbered clips (NN_Me.wav / NN_Others.wav), leaks a quiet
copy of Others into Me (speakers without headphones), then runs VAD -> Bark -> echo drop -> LFM2 summary.

    python tools/selftest.py models/meet
"""
import sys
import time
import wave as wavmod
from pathlib import Path

import numpy as np

from earshot import config
from earshot.audio import Track
from earshot.meeting import Segmenter, clean, drop_echo, merge_turns, transcript_md
from earshot.stt.engine import SR, Engine
from earshot.summarize import available, summarize


def load(p):
    with wavmod.open(str(p)) as f:
        return np.frombuffer(f.readframes(f.getnframes()), np.int16).astype(np.float32) / 32768


clips = sorted(Path(sys.argv[1]).glob("*.wav"))
gap = np.zeros(int(0.9 * SR), np.float32)
me, others = [], []
for c in clips:
    w = load(c)
    me.append(w if "_Me" in c.stem else 0.15 * w)  # echo of the other side picked up by the mic
    others.append(w if "_Others" in c.stem else np.zeros_like(w))
    me.append(gap)
    others.append(gap)
tracks = {}
for name, parts in (("Me", me), ("Others", others)):
    t = Track()
    t.push(np.concatenate(parts))
    tracks[name] = t
print(f"meeting length {len(tracks['Me']) / SR:.0f}s")

cfg = config.load()
engine = Engine(cfg["stt_model_dir"])
t0 = time.time()
lines = []
for name, seg in (("Me", Segmenter(tracks["Me"], ref=tracks["Others"])), ("Others", Segmenter(tracks["Others"]))):
    for a, b in seg.poll(final=True):
        text = clean(engine.transcribe(seg.read(a, b)))
        if text:
            lines.append((a / SR, b / SR, name, text))
print(f"stt {time.time() - t0:.1f}s, {len(lines)} utterances")
lines = drop_echo(sorted(lines))
print(f"after echo drop: {len(lines)} utterances")
lines = merge_turns(lines)
print(f"turns: {len(lines)}\n")
print(transcript_md(lines))
if available(cfg["llm"]):
    t0 = time.time()
    print("\n" + summarize(cfg["llm"], lines, {"Title": "Launch sync", "Participants": "Me, Others"}))
    print(f"\nsummary {time.time() - t0:.1f}s")
