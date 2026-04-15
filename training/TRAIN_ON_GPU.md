# TSE Model Training Guide (GPU Machine)

Run these steps on a machine with an NVIDIA GPU (RTX 3080+ recommended).
Total time: ~4-6 hours. Cost on cloud GPU: ~$2-3.

## Prerequisites

- Python 3.10+
- NVIDIA GPU with CUDA support
- ~15 GB disk space (LibriSpeech + mixtures + checkpoints)

## Step 0: Clone and enter the repo

```bash
git clone <your-repo-url> voice-gate
cd voice-gate
```

## Step 1: Set up Python environment

```bash
python3 -m venv training/.venv
source training/.venv/bin/activate
pip install -r training/requirements.txt
```

Verify GPU is visible:
```bash
python3 -c "import torch; print(f'CUDA available: {torch.cuda.is_available()}'); print(f'GPU: {torch.cuda.get_device_name(0) if torch.cuda.is_available() else \"none\"}')"
```

## Step 2: Download LibriSpeech clean-100

```bash
python3 training/dataset.py --download --output-dir training/data
```

This downloads ~6 GB and extracts to `training/data/LibriSpeech/train-clean-100/`.
251 speakers, ~100 hours of clean speech.

## Step 3: Download the WeSpeaker model (needed for embedding pre-computation)

```bash
# If you don't already have it:
./scripts/download_models.sh
```

The WeSpeaker ResNet34 ONNX should be at `models/wespeaker_resnet34_lm.onnx` (~25 MB).

## Step 4: Pre-compute speaker embeddings

```bash
python3 training/dataset.py --compute-embeddings \
    --model models/wespeaker_resnet34_lm.onnx \
    --wav-dir training/data/LibriSpeech/train-clean-100 \
    --output-dir training/data
```

This produces one `.npy` file per speaker in `training/data/embeddings/`.
Each file is a 256-dim L2-normalized centroid embedding, computed from up
to 10 utterances using the same WeSpeaker model that VoiceGate uses at
runtime. This ensures train/inference embedding consistency.

Takes ~10-15 minutes on CPU (ONNX inference is fast).

## Step 5: Generate 2-speaker mixtures

```bash
python3 training/mix_audio.py --bulk \
    --source-dir training/data/LibriSpeech/train-clean-100 \
    --output-dir training/data/mixtures \
    --num 50000 \
    --snr-range -5 5 \
    --sample-rate 48000
```

This creates 50,000 mixture WAV files at 48 kHz with random SNR between
-5 and +5 dB. Each mixture pairs two different speakers. The "target"
speaker is the one whose embedding will be provided during training.

Output files per mixture:
- `mix_NNNNNN.wav` -- the 2-speaker mixture
- `target_NNNNNN.wav` -- clean target speaker audio (training supervision)
- `meta_NNNNNN.txt` -- speaker IDs and SNR metadata

Takes ~30-60 minutes. Disk usage: ~8 GB.

## Step 6: Train the model

```bash
python3 training/train.py \
    --data-dir training/data/mixtures \
    --embeddings-dir training/data/embeddings \
    --epochs 50 \
    --batch-size 32 \
    --lr 1e-3 \
    --segment-length 3.0 \
    --fft-size 1024 \
    --hop-size 512 \
    --output training/checkpoints
```

Training parameters:
- Model: VoiceFilterLite (~900K params)
- Loss: L1 on masked magnitude spectrogram vs clean target magnitude
- Optimizer: Adam with cosine annealing (1e-3 -> 1e-5)
- Validation: 5% holdout, SDR metric
- Gradient clipping: max_norm=5.0
- Best model saved to `training/checkpoints/best.pt`

Expected output per epoch:
```
Epoch   1/50 | train_loss=0.XXXX | val_loss=0.XXXX | val_sdr=X.XX dB | lr=1.00e-03
Epoch   2/50 | train_loss=0.XXXX | val_loss=0.XXXX | val_sdr=X.XX dB | lr=9.98e-04
...
```

Target metrics:
- val_loss should decrease steadily for ~30 epochs then plateau
- val_sdr should reach > 5 dB (good), > 8 dB (very good), > 10 dB (excellent)
- If val_sdr stays below 3 dB after 20 epochs, something is wrong

Training time:
- RTX 3080: ~4-6 hours
- RTX 4090: ~2-3 hours
- A100: ~1-2 hours
- CPU only: ~24-48 hours (not recommended for 50k mixtures)

## Step 7: Export to ONNX

```bash
python3 training/export_tse.py \
    --checkpoint training/checkpoints/best.pt \
    --output models/voicegate_tse.onnx
```

This will:
1. Load the trained PyTorch model
2. Export to ONNX with dynamic time axis
3. Run a mandatory equivalence check (max|pytorch - onnx| < 1e-4)
4. Test variable-length input (T=3 and T=10)
5. Print model size and I/O tensor info

Expected output:
```
Loaded model: 9XX,XXX parameters
ONNX model saved: models/voicegate_tse.onnx (X.XX MB)
  Input:  magnitude [1, 'time', 513]
  Input:  embedding [1, 256]
  Input:  state [2, 1, 128]
  Output: mask [1, 'time', 513]
  Output: stateN [2, 1, 128]

Running PyTorch vs ONNX equivalence check...
  mask  max|diff| = X.XXe-07
  state max|diff| = X.XXe-07
  PASS: PyTorch and ONNX outputs match within tolerance.

Testing dynamic time axis...
  T=10 max|diff| = X.XXe-07
  PASS: Dynamic time axis works correctly.

Export complete.
```

If the equivalence check FAILS, do NOT use the model. Check the export
script for issues.

## Step 8: Generate test fixtures

```bash
# Create a mixed-speech test fixture for Rust integration tests
python3 training/mix_audio.py \
    --source tests/fixtures/speaker_a.wav \
    --interferer tests/fixtures/speaker_b.wav \
    --snr 0 \
    --sample-rate 48000 \
    --output tests/fixtures/mixed_ab_overlap.wav
```

## Step 9: Copy the model back

The only file you need from the GPU machine:

```bash
# From the GPU machine:
scp models/voicegate_tse.onnx your-dev-machine:Projects/voice-gate/models/

# Also grab the test fixture if generated:
scp tests/fixtures/mixed_ab_overlap.wav your-dev-machine:Projects/voice-gate/tests/fixtures/
```

## Step 10: Enable TSE on the dev machine

Edit `~/.config/voicegate/config.toml`:

```toml
[tse]
enabled = true
model_path = "models/voicegate_tse.onnx"
blend = 1.0
```

Or toggle TSE ON in the GUI (the button is in the waveform section header).

Then restart VoiceGate:
```bash
cargo run --release -- run
```

## Quick sanity check (CPU, ~5 min)

If you want to verify the training pipeline works before committing to
the full 50k-mixture run, do a minimal test:

```bash
# Generate just 100 mixtures
python3 training/mix_audio.py --bulk \
    --source-dir training/data/LibriSpeech/train-clean-100 \
    --output-dir training/data/mixtures_test \
    --num 100 --snr-range -5 5 --sample-rate 48000

# Train for 2 epochs
python3 training/train.py \
    --data-dir training/data/mixtures_test \
    --embeddings-dir training/data/embeddings \
    --epochs 2 --batch-size 4 \
    --output training/checkpoints_test

# Export (model will be garbage but pipeline is validated)
python3 training/export_tse.py \
    --checkpoint training/checkpoints_test/best.pt \
    --output models/voicegate_tse_test.onnx
```

If this completes without errors, the full training run will work.

## Troubleshooting

**"CUDA out of memory"**: Reduce `--batch-size` to 16 or 8.

**"No module named 'soundfile'"**: `pip install soundfile` (needs `libsndfile1-dev` on Ubuntu).

**"LibriSpeech download fails"**: Download manually from https://www.openslr.org/12/ and extract to `training/data/LibriSpeech/`.

**"No speakers found"**: Check that the directory structure is `train-clean-100/<speaker_id>/<chapter_id>/<file>.flac`. The dataset.py expects FLAC files in subdirectories.

**val_sdr stuck below 0 dB**: The model is not learning. Check:
- Are embeddings correctly pre-computed? (`ls training/data/embeddings/*.npy | wc -l` should be ~250)
- Are mixtures valid audio? (`play training/data/mixtures/mix_000000.wav`)
- Is the STFT config matching? (fft_size=1024, hop_size=512, matching the Rust side)

## Model Architecture Reference

```
VoiceFilterLite (~900K parameters, ~3.5 MB ONNX FP32)
+-----------------------------------------------+
| Input: magnitude [B, T, 513]                  |
| Input: embedding [B, 256]                     |
| Input: state [2, B, 128]                      |
+-----------------------------------------------+
| Linear(256, 64)   -> [B, 1, 64]              |
| Expand            -> [B, T, 64]              |
| Cat(mag, emb)     -> [B, T, 577]             |
| GRU(577, 128, L=2, causal) -> [B, T, 128]    |
| Linear(128, 513)  -> [B, T, 513]             |
| Sigmoid           -> [B, T, 513]  (mask)     |
+-----------------------------------------------+
| Output: mask [B, T, 513]                     |
| Output: stateN [2, B, 128]                   |
+-----------------------------------------------+

STFT parameters (must match Rust side exactly):
  FFT size:  1024
  Hop size:  512
  Window:    Periodic Hann
  Freq bins: 513 (= FFT_SIZE/2 + 1)
  Sample rate: 48000 Hz
```
