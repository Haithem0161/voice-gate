#!/usr/bin/env python3
# Phase 2 placeholder. Exports SpeechBrain ECAPA-TDNN to ONNX with a mandatory
# PyTorch vs ONNX equivalence check. Real implementation lands in Phase 2 per
# docs/voicegate/phase-02.md.
#
# Planned behavior (Phase 2):
#   1. from speechbrain.inference import SpeakerRecognition
#      model = SpeakerRecognition.from_hparams("speechbrain/spkrec-ecapa-voxceleb")
#   2. Extract model.mods.embedding_model
#   3. torch.onnx.export(..., opset_version=14,
#                        dynamic_axes={"feats": {1: "T"}})
#   4. MANDATORY equivalence check:
#        dummy = torch.randn(1, 16000)
#        pt_out = model.mods.embedding_model(dummy).detach().numpy()
#        ort_out = onnxruntime.InferenceSession("models/ecapa_tdnn.onnx")
#                   .run(None, {"feats": dummy.numpy()})[0]
#        assert np.max(np.abs(pt_out - ort_out)) < 1e-4
#   5. On failure, the script exits non-zero and the fallback decision
#      (D-002: use WeSpeaker pre-exported ONNX) is triggered in Phase 2.
#
# For Phase 1 this script exists only so `make models` has a target to call.

def main() -> int:
    print("scripts/export_ecapa.py: not yet implemented (Phase 2)")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
