"""
Dataset loader for TSE training. Reads pre-generated mixtures and their
corresponding clean targets + speaker embeddings.

The dataset expects the following directory structure (created by mix_audio.py):
    data/mixtures/
        mix_000000.wav      -- 2-speaker mixture
        target_000000.wav   -- clean target speaker audio
        meta_000000.txt     -- speaker IDs and SNR metadata

    data/embeddings/
        <speaker_id>.npy    -- pre-computed 256-dim WeSpeaker embedding

Usage:
    from dataset import TseDataset
    ds = TseDataset("data/mixtures", "data/embeddings",
                     fft_size=1024, hop_size=512, sample_rate=48000)
    mixture_mag, target_mag, embedding = ds[0]
"""

import os
from pathlib import Path

import numpy as np
import torch
from torch.utils.data import Dataset


class TseDataset(Dataset):
    """PyTorch dataset for TSE training."""

    def __init__(
        self,
        mixtures_dir: str,
        embeddings_dir: str,
        fft_size: int = 1024,
        hop_size: int = 512,
        sample_rate: int = 48000,
        segment_seconds: float = 3.0,
    ):
        self.mixtures_dir = Path(mixtures_dir)
        self.embeddings_dir = Path(embeddings_dir)
        self.fft_size = fft_size
        self.hop_size = hop_size
        self.sample_rate = sample_rate
        self.segment_samples = int(segment_seconds * sample_rate)

        # Hann window (periodic).
        self.window = torch.hann_window(fft_size, periodic=True)

        # Find all mixture files.
        self.mix_files = sorted(self.mixtures_dir.glob("mix_*.wav"))
        print(f"TseDataset: found {len(self.mix_files)} mixtures")

    def __len__(self) -> int:
        return len(self.mix_files)

    def __getitem__(self, idx: int) -> tuple[torch.Tensor, torch.Tensor, torch.Tensor]:
        import soundfile as sf

        mix_path = self.mix_files[idx]
        stem = mix_path.stem.replace("mix_", "")
        target_path = self.mixtures_dir / f"target_{stem}.wav"
        meta_path = self.mixtures_dir / f"meta_{stem}.txt"

        # Load audio.
        mixture, _ = sf.read(str(mix_path), dtype="float32")
        target, _ = sf.read(str(target_path), dtype="float32")

        # Trim/pad to segment length.
        mixture = self._trim_or_pad(mixture)
        target = self._trim_or_pad(target)

        # Load speaker embedding from metadata.
        meta = meta_path.read_text().strip().split("\t")
        speaker_id = meta[0]  # target speaker ID
        emb_path = self.embeddings_dir / f"{speaker_id}.npy"
        if emb_path.exists():
            embedding = np.load(str(emb_path)).astype(np.float32)
        else:
            # Fallback: zero embedding (will train poorly but won't crash).
            embedding = np.zeros(256, dtype=np.float32)

        # Compute STFT magnitude for both mixture and target.
        mixture_tensor = torch.from_numpy(mixture)
        target_tensor = torch.from_numpy(target)

        mix_stft = torch.stft(
            mixture_tensor, self.fft_size, self.hop_size,
            window=self.window, return_complex=True
        )
        target_stft = torch.stft(
            target_tensor, self.fft_size, self.hop_size,
            window=self.window, return_complex=True
        )

        mix_mag = mix_stft.abs().T       # [T, F]
        target_mag = target_stft.abs().T  # [T, F]

        embedding_tensor = torch.from_numpy(embedding)

        return mix_mag, target_mag, embedding_tensor

    def _trim_or_pad(self, audio: np.ndarray) -> np.ndarray:
        if len(audio) > self.segment_samples:
            # Random offset for data augmentation.
            max_start = len(audio) - self.segment_samples
            start = np.random.randint(0, max_start + 1)
            return audio[start : start + self.segment_samples]
        elif len(audio) < self.segment_samples:
            pad = np.zeros(self.segment_samples - len(audio), dtype=np.float32)
            return np.concatenate([audio, pad])
        return audio


def precompute_embeddings(
    wav_dir: str,
    output_dir: str,
    onnx_model_path: str,
    sample_rate: int = 16000,
):
    """
    Pre-compute WeSpeaker embeddings for all speakers in a directory.

    Expects wav_dir to have subdirectories per speaker:
        wav_dir/speaker_id/file.wav

    Saves one .npy file per speaker in output_dir.
    """
    import onnxruntime as ort

    wav_dir = Path(wav_dir)
    output_dir = Path(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    sess = ort.InferenceSession(onnx_model_path)
    input_name = sess.get_inputs()[0].name

    # Group files by speaker.
    speakers = {}
    for wav_path in sorted(wav_dir.rglob("*.wav")):
        speaker = wav_path.parent.name
        if speaker not in speakers:
            speakers[speaker] = []
        speakers[speaker].append(wav_path)

    print(f"Computing embeddings for {len(speakers)} speakers...")

    for speaker, files in speakers.items():
        embeddings = []
        for wav_path in files[:10]:  # max 10 files per speaker
            import soundfile as sf
            audio, sr = sf.read(str(wav_path), dtype="float32")
            if audio.ndim > 1:
                audio = audio.mean(axis=1)
            if sr != sample_rate:
                import torchaudio
                waveform = torch.from_numpy(audio).unsqueeze(0)
                audio = torchaudio.functional.resample(waveform, sr, sample_rate).squeeze(0).numpy()

            # Compute fbank features (matching WeSpeaker's expected input).
            import torchaudio
            waveform = torch.from_numpy(audio).unsqueeze(0) * 32768.0
            feats = torchaudio.compliance.kaldi.fbank(
                waveform, num_mel_bins=80, frame_length=25, frame_shift=10,
                dither=0.0, sample_frequency=sample_rate, window_type="hamming",
                use_energy=False,
            )
            feats = feats - feats.mean(dim=0)  # CMN
            feats = feats.unsqueeze(0).numpy()  # [1, T, 80]

            out = sess.run(None, {input_name: feats})[0]
            emb = out.squeeze()
            # L2 normalize.
            norm = np.linalg.norm(emb)
            if norm > 1e-10:
                emb = emb / norm
            embeddings.append(emb)

        if embeddings:
            centroid = np.mean(embeddings, axis=0).astype(np.float32)
            # Re-normalize centroid.
            norm = np.linalg.norm(centroid)
            if norm > 1e-10:
                centroid = centroid / norm
            np.save(str(output_dir / f"{speaker}.npy"), centroid)

    print(f"Saved embeddings to {output_dir}")


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument("--download", action="store_true", help="Download LibriSpeech clean-100")
    parser.add_argument("--compute-embeddings", action="store_true")
    parser.add_argument("--model", type=str, help="WeSpeaker ONNX model path")
    parser.add_argument("--output-dir", type=str, default="training/data")
    parser.add_argument("--wav-dir", type=str, help="WAV directory for embeddings")
    args = parser.parse_args()

    if args.download:
        import urllib.request
        import tarfile

        url = "https://www.openslr.org/resources/12/train-clean-100.tar.gz"
        output = Path(args.output_dir)
        output.mkdir(parents=True, exist_ok=True)
        tar_path = output / "train-clean-100.tar.gz"

        if not tar_path.exists():
            print(f"Downloading LibriSpeech clean-100 ({url})...")
            urllib.request.urlretrieve(url, str(tar_path))
            print("Download complete.")

        if not (output / "LibriSpeech").exists():
            print("Extracting...")
            with tarfile.open(str(tar_path), "r:gz") as tar:
                tar.extractall(str(output))
            print("Extraction complete.")

    if args.compute_embeddings:
        if not args.model:
            print("Error: --model required for --compute-embeddings")
            exit(1)
        wav_dir = args.wav_dir or f"{args.output_dir}/LibriSpeech/train-clean-100"
        precompute_embeddings(
            wav_dir, f"{args.output_dir}/embeddings", args.model
        )
