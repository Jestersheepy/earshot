"""One-time export: Underdog Bark (.bark) -> int8 ONNX encoder + numpy decoder/joint.

Runs in a throwaway venv with torch-cpu + transformers (see README). The app itself never imports torch.

    python tools/export_bark_onnx.py models/bark-src models/bark-onnx
"""
import json
import os
import shutil
import sys
from pathlib import Path

import numpy as np
import torch

src, out = Path(sys.argv[1]), Path(sys.argv[2])
out.mkdir(parents=True, exist_ok=True)
sys.path.insert(0, str(src))
import bark_loader  # noqa: E402  (official loader, shipped with the model)

cache = src.parent / "bark-fp32.pt"
model = torch.load(cache, weights_only=False) if cache.exists() else bark_loader.load_model(str(src / "model.bark"))
model.eval()
cfg = model.config


class Encoder(torch.nn.Module):
    """features (1, T, 128) -> projected encoder frames (1, T/8, 640)."""

    def __init__(self, m):
        super().__init__()
        self.encoder, self.proj = m.encoder, m.encoder_projector

    def forward(self, features):
        return self.proj(self.encoder(input_features=features).last_hidden_state)


# fp32 encoder is ~2.4 GB, so torch writes it with external data; keep that in a temp dir.
tmp = out / "tmp"
tmp.mkdir(exist_ok=True)
fp32_path, int8_path = tmp / "encoder.onnx", out / "encoder.int8.onnx"
dummy = torch.randn(1, 500, 128)
with torch.inference_mode():
    torch.onnx.export(
        Encoder(model), (dummy,), str(fp32_path), dynamo=False, opset_version=17,
        input_names=["features"], output_names=["encoded"],
        dynamic_axes={"features": {1: "frames"}, "encoded": {1: "steps"}},
    )

from onnxruntime.quantization import QuantType, quantize_dynamic  # noqa: E402

quantize_dynamic(str(fp32_path), str(int8_path), weight_type=QuantType.QInt8, op_types_to_quantize=["MatMul", "Gemm"])
shutil.rmtree(tmp)

# Decoder (embedding + 2-layer LSTM + projector) and joint head are ~17M params: plain numpy at runtime.
sd = {k: v.detach().numpy() for k, v in model.state_dict().items() if not k.startswith("encoder")}
import librosa  # noqa: E402

from transformers import AutoProcessor  # noqa: E402

proc = AutoProcessor.from_pretrained("nvidia/parakeet-tdt-0.6b-v3")
fe = proc.feature_extractor
mel = librosa.filters.mel(sr=16000, n_fft=fe.n_fft, n_mels=fe.feature_size, fmin=0.0, fmax=8000, norm="slaney")
np.savez(
    out / "decoder.npz",
    embedding=sd["decoder.embedding.weight"].astype(np.float16),
    **{k.replace("decoder.lstm.", "lstm_"): v.astype(np.float32) for k, v in sd.items() if "lstm" in k},
    dec_proj_w=sd["decoder.decoder_projector.weight"], dec_proj_b=sd["decoder.decoder_projector.bias"],
    joint_w=sd["joint.head.weight"].astype(np.float16), joint_b=sd["joint.head.bias"],
    mel_filters=mel.astype(np.float32),
)
pieces = proc.tokenizer.convert_ids_to_tokens(list(range(cfg.vocab_size)))
(out / "config.json").write_text(json.dumps({
    "vocab": pieces, "blank_id": cfg.blank_token_id, "durations": list(cfg.durations),
    "max_symbols_per_step": cfg.max_symbols_per_step, "n_fft": fe.n_fft, "hop": fe.hop_length,
    "win": fe.win_length, "preemphasis": fe.preemphasis, "pad_s": 0.25,
}))
(out / "NOTICE").write_text((src / "NOTICE").read_text())
for f in sorted(out.iterdir()):
    print(f"{f.name:24s} {os.path.getsize(f) / 1e6:8.1f} MB")
