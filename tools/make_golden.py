"""Write models/golden.json: reference transcripts from the Python engine for the Rust port to match.

    python tools/make_golden.py
"""
import json
import wave
from pathlib import Path

import numpy as np

from earshot.config import MODELS_DIR
from earshot.stt.engine import BarkModel


def load(p):
    with wave.open(str(p)) as f:
        return np.frombuffer(f.readframes(f.getnframes()), np.int16).astype(np.float32) / 32768


model = BarkModel(MODELS_DIR / "bark-onnx")
wavs = [MODELS_DIR / "test1.wav", *sorted((MODELS_DIR / "meet").glob("*.wav"))]
cases = [{"wavs": [str(w.relative_to(MODELS_DIR))], "text": model.transcribe(load(w))} for w in wavs]
# >30 s: exercises split_on_silence
long = [MODELS_DIR / "test1.wav", *wavs[1:4]]
cases.append({"wavs": [str(w.relative_to(MODELS_DIR)) for w in long],
              "text": model.transcribe(np.concatenate([load(w) for w in long]))})
(MODELS_DIR / "golden.json").write_text(json.dumps(cases, indent=1))
print(f"{len(cases)} cases")
