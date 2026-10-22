"""Bark speech-to-text on CPU: ONNX encoder + numpy TDT greedy decoder. No torch. The reference the Rust
port (crates/core/src/stt.rs) is tested against; see make_golden.py.

    python tools/reference_stt.py recording.wav
"""
import json
import threading
import time
from pathlib import Path

import numpy as np

SR = 16000
LOG_GUARD = 2.0 ** -24
MAX_CHUNK_S = 30.0  # encoder attention is quadratic; split long audio at quiet points


class BarkModel:
    def __init__(self, model_dir, threads=None):
        import onnxruntime as ort

        d = Path(model_dir)
        opts = ort.SessionOptions()
        if threads:
            opts.intra_op_num_threads = threads
        self.session = ort.InferenceSession(str(d / "encoder.int8.onnx"), opts, providers=["CPUExecutionProvider"])
        w = np.load(d / "decoder.npz")
        self.emb = w["embedding"]
        self.lstm = [
            (w[f"lstm_weight_ih_l{i}"], w[f"lstm_weight_hh_l{i}"], w[f"lstm_bias_ih_l{i}"] + w[f"lstm_bias_hh_l{i}"])
            for i in range(2)
        ]
        self.dec_w, self.dec_b = w["dec_proj_w"], w["dec_proj_b"]
        self.joint_w, self.joint_b = w["joint_w"].astype(np.float32), w["joint_b"]
        self.mel = w["mel_filters"]
        cfg = json.loads((d / "config.json").read_text())
        self.cfg = cfg
        self.vocab = cfg["vocab"]
        self.blank = cfg["blank_id"]
        self.durations = cfg["durations"]
        win = np.hanning(cfg["win"])  # == torch.hann_window(periodic=False), centred in n_fft like torch.stft
        pad = (cfg["n_fft"] - cfg["win"]) // 2
        self.window = np.pad(win, (pad, cfg["n_fft"] - cfg["win"] - pad)).astype(np.float32)

    # --- frontend -----------------------------------------------------------------------------
    def features(self, wave):
        c = self.cfg
        n_fft, hop = c["n_fft"], c["hop"]
        x = np.concatenate([wave[:1], wave[1:] - c["preemphasis"] * wave[:-1]]).astype(np.float32)
        x = np.pad(x, (n_fft // 2, n_fft // 2))
        n_frames = 1 + (len(x) - n_fft) // hop
        frames = np.lib.stride_tricks.as_strided(x, (n_frames, n_fft), (x.strides[0] * hop, x.strides[0]))
        power = np.abs(np.fft.rfft(frames * self.window, axis=1)) ** 2
        feats = np.log(power @ self.mel.T + LOG_GUARD)  # (frames, 128)
        valid = len(wave) // hop
        v = feats[:valid]
        mean = v.mean(0)
        std = np.sqrt(((v - mean) ** 2).sum(0) / max(valid - 1, 1))
        feats = (feats - mean) / (std + 1e-5)
        feats[valid:] = 0.0
        return feats[None].astype(np.float32)

    # --- decoder ------------------------------------------------------------------------------
    def _dec_step(self, token, state):
        x = self.emb[token].astype(np.float32)
        new = []
        for (w_ih, w_hh, b), (h, c) in zip(self.lstm, state):
            g = w_ih @ x + w_hh @ h + b
            i, f, gg, o = np.split(g, 4)
            c = _sigmoid(f) * c + _sigmoid(i) * np.tanh(gg)
            h = _sigmoid(o) * np.tanh(c)
            new.append((h, c))
            x = h
        return self.dec_w @ x + self.dec_b, new

    def _greedy_tdt(self, enc):
        V = len(self.vocab)
        state = [(np.zeros(640, np.float32), np.zeros(640, np.float32)) for _ in self.lstm]
        dec, state = self._dec_step(self.blank, state)
        tokens, t, same_frame = [], 0, 0
        max_sym = self.cfg["max_symbols_per_step"]
        while t < len(enc):
            logits = self.joint_w @ np.maximum(enc[t] + dec, 0) + self.joint_b
            tok = int(np.argmax(logits[:V]))
            dur = self.durations[int(np.argmax(logits[V:]))]
            if tok == self.blank:
                dur = max(dur, 1)
            else:
                tokens.append(tok)
                dec, state = self._dec_step(tok, state)
                same_frame = same_frame + 1 if dur == 0 else 0
                if same_frame >= max_sym:
                    dur, same_frame = 1, 0
            t += dur
        return tokens

    def _detok(self, ids):
        pieces = (self.vocab[i] for i in ids)
        text = "".join(p for p in pieces if not (p.startswith("<") and p.endswith(">")))
        return text.replace("▁", " ").strip()

    def transcribe_chunk(self, wave):
        wave = np.concatenate([np.zeros(int(self.cfg["pad_s"] * SR), np.float32), wave.astype(np.float32)])
        enc = self.session.run(None, {"features": self.features(wave)})[0][0]
        return self._detok(self._greedy_tdt(enc))

    def transcribe(self, wave, sr=SR):
        wave = to_16k_mono(wave, sr)
        return " ".join(t for t in (self.transcribe_chunk(c) for c in split_on_silence(wave)) if t)


def _sigmoid(x):
    return 1.0 / (1.0 + np.exp(-x))


def to_16k_mono(wave, sr):
    wave = np.asarray(wave, np.float32)
    if wave.ndim == 2:
        wave = wave.mean(1)
    if sr != SR:  # linear resample is fine for speech; capture devices are usually 16/44.1/48 kHz
        n = int(len(wave) * SR / sr)
        wave = np.interp(np.linspace(0, len(wave) - 1, n), np.arange(len(wave)), wave).astype(np.float32)
    return wave


def split_on_silence(wave, max_s=MAX_CHUNK_S, min_s=MAX_CHUNK_S * 0.6):
    """Cut audio longer than max_s at the quietest 30 ms frame between min_s and max_s."""
    chunks, start, frame = [], 0, int(0.03 * SR)
    while len(wave) - start > max_s * SR:
        lo, hi = start + int(min_s * SR), start + int(max_s * SR)
        seg = wave[lo:hi][: (hi - lo) // frame * frame].reshape(-1, frame)
        cut = lo + int(np.argmin((seg ** 2).mean(1))) * frame + frame // 2
        chunks.append(wave[start:cut])
        start = cut
    chunks.append(wave[start:])
    return chunks


def rms(wave):
    return float(np.sqrt(np.mean(np.square(wave)))) if len(wave) else 0.0


class Engine:
    """Lazy-loading wrapper: loads the model on first use, frees it after `idle_unload_s` of no use."""

    def __init__(self, model_dir, idle_unload_s=900, threads=None):
        self.model_dir, self.idle_unload_s, self.threads = model_dir, idle_unload_s, threads
        self._model, self._lock, self._last_used = None, threading.Lock(), 0.0
        threading.Thread(target=self._reaper, daemon=True).start()

    def preload(self):
        with self._lock:
            self._get()

    def transcribe(self, wave, sr=SR):
        with self._lock:
            text = self._get().transcribe(wave, sr)
            self._last_used = time.monotonic()
            return text

    def _get(self):
        if self._model is None:
            self._model = BarkModel(self.model_dir, self.threads)
        self._last_used = time.monotonic()
        return self._model

    def _reaper(self):
        while True:
            time.sleep(30)
            with self._lock:
                if self._model and self.idle_unload_s and time.monotonic() - self._last_used > self.idle_unload_s:
                    self._model = None


if __name__ == "__main__":
    import sys
    import wave as wavmod

    MODELS_DIR = Path(__file__).resolve().parent.parent / "models"

    with wavmod.open(sys.argv[1]) as f:
        sr, ch = f.getframerate(), f.getnchannels()
        pcm = np.frombuffer(f.readframes(f.getnframes()), np.int16).astype(np.float32) / 32768
    pcm = pcm.reshape(-1, ch)
    t = time.time()
    m = BarkModel(MODELS_DIR / "bark-onnx")
    print(f"load {time.time() - t:.2f}s")
    t = time.time()
    print(m.transcribe(pcm, sr))
    print(f"stt {time.time() - t:.2f}s for {len(pcm) / sr:.1f}s audio")
