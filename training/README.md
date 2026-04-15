# VoiceGate TSE Training Pipeline

Train a VoiceFilterLite model for target speaker extraction.

## Prerequisites

```bash
pip install -r training/requirements.txt
```

You also need the WeSpeaker ResNet34 ONNX model for embedding pre-computation:
```bash
./scripts/download_models.sh
```

## Steps

### 1. Download training data

```bash
python training/dataset.py --download --output-dir training/data
```

Downloads LibriSpeech clean-100 (~6 GB).

### 2. Pre-compute speaker embeddings

```bash
python training/dataset.py --compute-embeddings \
    --model models/wespeaker_resnet34_lm.onnx \
    --output-dir training/data
```

### 3. Generate mixtures

```bash
python training/mix_audio.py --bulk \
    --source-dir training/data/LibriSpeech/train-clean-100 \
    --output-dir training/data/mixtures \
    --num 50000 --snr-range -5 5 --sample-rate 48000
```

### 4. Train

```bash
python training/train.py \
    --data-dir training/data/mixtures \
    --embeddings-dir training/data/embeddings \
    --epochs 50 --batch-size 32 --lr 1e-3 \
    --output training/checkpoints
```

Training takes ~4-8 hours on a single GPU (RTX 3080+).

### 5. Export to ONNX

```bash
python training/export_tse.py \
    --checkpoint training/checkpoints/best.pt \
    --output models/voicegate_tse.onnx
```

The export script runs a mandatory PyTorch vs ONNX equivalence check.

### 6. Test fixtures

```bash
python training/mix_audio.py \
    --source tests/fixtures/speaker_a.wav \
    --interferer tests/fixtures/speaker_b.wav \
    --snr 0 --sample-rate 48000 \
    --output tests/fixtures/mixed_ab_overlap.wav
```

## Model Architecture

VoiceFilterLite: ~900K parameters, ~3.5 MB ONNX (FP32).

- Input: magnitude spectrogram [1, T, 513] + speaker embedding [1, 256] + GRU state [2, 1, 128]
- Output: time-frequency mask [1, T, 513] (sigmoid, values in [0, 1]) + updated GRU state
- Architecture: Linear(256->64) embedding projection, 2-layer causal GRU (hidden=128), Linear(128->513) + Sigmoid mask head
