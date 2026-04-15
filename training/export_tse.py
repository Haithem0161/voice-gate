"""
Export VoiceFilterLite to ONNX with mandatory equivalence check.

Usage:
    python training/export_tse.py \
        --checkpoint training/checkpoints/best.pt \
        --output models/voicegate_tse.onnx

The script will:
1. Load the PyTorch model from the checkpoint
2. Export to ONNX with dynamic axes for variable-length input
3. Run an equivalence check: max(|pytorch - onnx|) < 1e-4
4. Print the model size and input/output names
"""

import argparse
from pathlib import Path

import numpy as np
import onnx
import onnxruntime as ort
import torch

from model import VoiceFilterLite, count_parameters


def export(checkpoint_path: str, output_path: str):
    # Load model.
    model = VoiceFilterLite()
    state_dict = torch.load(checkpoint_path, map_location="cpu", weights_only=True)
    model.load_state_dict(state_dict)
    model.eval()

    print(f"Loaded model: {count_parameters(model):,} parameters")

    # Dummy inputs for tracing.
    dummy_mag = torch.randn(1, 3, 513)   # [B=1, T=3, F=513]
    dummy_emb = torch.randn(1, 256)       # [B=1, E=256]
    dummy_state = model.initial_state(1)  # [2, 1, 128]

    # Export to ONNX.
    output = Path(output_path)
    output.parent.mkdir(parents=True, exist_ok=True)

    torch.onnx.export(
        model,
        (dummy_mag, dummy_emb, dummy_state),
        str(output),
        input_names=["magnitude", "embedding", "state"],
        output_names=["mask", "stateN"],
        dynamic_axes={
            "magnitude": {1: "time"},
            "mask": {1: "time"},
        },
        opset_version=14,
        do_constant_folding=True,
    )

    # Validate ONNX model.
    onnx_model = onnx.load(str(output))
    onnx.checker.check_model(onnx_model)
    print(f"ONNX model saved: {output} ({output.stat().st_size / 1024 / 1024:.2f} MB)")

    # Print input/output info.
    for inp in onnx_model.graph.input:
        print(f"  Input:  {inp.name} {[d.dim_value or d.dim_param for d in inp.type.tensor_type.shape.dim]}")
    for out in onnx_model.graph.output:
        print(f"  Output: {out.name} {[d.dim_value or d.dim_param for d in out.type.tensor_type.shape.dim]}")

    # Equivalence check.
    print("\nRunning PyTorch vs ONNX equivalence check...")

    with torch.no_grad():
        pt_mask, pt_state = model(dummy_mag, dummy_emb, dummy_state)

    sess = ort.InferenceSession(str(output))
    ort_out = sess.run(
        None,
        {
            "magnitude": dummy_mag.numpy(),
            "embedding": dummy_emb.numpy(),
            "state": dummy_state.numpy(),
        },
    )

    mask_diff = np.max(np.abs(pt_mask.numpy() - ort_out[0]))
    state_diff = np.max(np.abs(pt_state.numpy() - ort_out[1]))

    print(f"  mask  max|diff| = {mask_diff:.2e}")
    print(f"  state max|diff| = {state_diff:.2e}")

    assert mask_diff < 1e-4, f"MASK DIVERGENCE: {mask_diff} (threshold: 1e-4)"
    assert state_diff < 1e-4, f"STATE DIVERGENCE: {state_diff} (threshold: 1e-4)"
    print("  PASS: PyTorch and ONNX outputs match within tolerance.")

    # Test with variable-length input.
    print("\nTesting dynamic time axis...")
    long_mag = torch.randn(1, 10, 513)
    long_state = model.initial_state(1)
    with torch.no_grad():
        pt_long_mask, _ = model(long_mag, dummy_emb, long_state)
    ort_long = sess.run(
        None,
        {
            "magnitude": long_mag.numpy(),
            "embedding": dummy_emb.numpy(),
            "state": long_state.numpy(),
        },
    )
    long_diff = np.max(np.abs(pt_long_mask.numpy() - ort_long[0]))
    print(f"  T=10 max|diff| = {long_diff:.2e}")
    assert long_diff < 1e-4, f"LONG INPUT DIVERGENCE: {long_diff}"
    print("  PASS: Dynamic time axis works correctly.")

    print("\nExport complete.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Export VoiceFilterLite to ONNX")
    parser.add_argument("--checkpoint", type=str, required=True)
    parser.add_argument("--output", type=str, default="models/voicegate_tse.onnx")
    args = parser.parse_args()
    export(args.checkpoint, args.output)
