"""
Create 2-speaker audio mixtures for TSE training.

Usage:
    # Mix two WAV files at a specific SNR:
    python mix_audio.py --source a.wav --interferer b.wav --snr 0 --output mix.wav

    # Generate bulk mixtures from a directory of clean speech:
    python mix_audio.py --bulk --source-dir data/clean/ \
        --output-dir data/mixtures/ --num 50000 --snr-range -5 5
"""

import argparse
import os
import random
from pathlib import Path

import numpy as np
import soundfile as sf


def load_audio(path: str, target_sr: int = 48000) -> np.ndarray:
    """Load a WAV file, resample to target_sr, convert to mono float32."""
    audio, sr = sf.read(path, dtype="float32", always_2d=True)
    audio = audio.mean(axis=1)  # mono

    if sr != target_sr:
        # Simple resampling via interpolation (for training data, not real-time).
        import torchaudio
        import torch

        waveform = torch.from_numpy(audio).unsqueeze(0)
        resampled = torchaudio.functional.resample(waveform, sr, target_sr)
        audio = resampled.squeeze(0).numpy()

    return audio


def mix_at_snr(
    source: np.ndarray, interferer: np.ndarray, snr_db: float
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """
    Mix source and interferer at the given SNR (in dB).

    Returns (mixture, source_aligned, interferer_aligned) all same length.
    """
    # Align lengths: truncate to the shorter one.
    min_len = min(len(source), len(interferer))
    source = source[:min_len].copy()
    interferer = interferer[:min_len].copy()

    # Compute power and scale interferer to achieve target SNR.
    source_power = np.mean(source ** 2) + 1e-10
    interferer_power = np.mean(interferer ** 2) + 1e-10
    target_interferer_power = source_power / (10 ** (snr_db / 10))
    scale = np.sqrt(target_interferer_power / interferer_power)
    interferer_scaled = interferer * scale

    mixture = source + interferer_scaled

    # Normalize to prevent clipping.
    peak = np.max(np.abs(mixture))
    if peak > 0.95:
        norm_factor = 0.95 / peak
        mixture *= norm_factor
        source *= norm_factor
        interferer_scaled *= norm_factor

    return mixture, source, interferer_scaled


def main():
    parser = argparse.ArgumentParser(description="Create 2-speaker audio mixtures")
    parser.add_argument("--source", type=str, help="Source (target) WAV file")
    parser.add_argument("--interferer", type=str, help="Interferer WAV file")
    parser.add_argument("--snr", type=float, default=0.0, help="SNR in dB")
    parser.add_argument("--output", type=str, help="Output mixture WAV file")
    parser.add_argument("--sample-rate", type=int, default=48000)

    # Bulk generation mode.
    parser.add_argument("--bulk", action="store_true", help="Generate bulk mixtures")
    parser.add_argument("--source-dir", type=str, help="Directory of clean WAVs")
    parser.add_argument("--output-dir", type=str, help="Output directory for mixtures")
    parser.add_argument("--num", type=int, default=50000, help="Number of mixtures")
    parser.add_argument(
        "--snr-range", nargs=2, type=float, default=[-5, 5],
        help="SNR range [min, max] in dB"
    )

    args = parser.parse_args()

    if args.bulk:
        generate_bulk(args)
    else:
        if not all([args.source, args.interferer, args.output]):
            parser.error("--source, --interferer, and --output are required")
        source = load_audio(args.source, args.sample_rate)
        interferer = load_audio(args.interferer, args.sample_rate)
        mixture, source_clean, _ = mix_at_snr(source, interferer, args.snr)
        sf.write(args.output, mixture, args.sample_rate)
        # Also save the aligned clean target for training reference.
        clean_path = args.output.replace(".wav", "_target.wav")
        sf.write(clean_path, source_clean, args.sample_rate)
        print(f"Wrote mixture: {args.output} ({len(mixture)} samples)")
        print(f"Wrote target:  {clean_path}")


def generate_bulk(args):
    """Generate bulk mixtures from a directory of clean speech files."""
    source_dir = Path(args.source_dir)
    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    # Collect all WAV files grouped by speaker.
    # Assumes directory structure: source_dir/speaker_id/file.wav
    speakers = {}
    for wav_path in sorted(source_dir.rglob("*.wav")):
        # Use parent directory name as speaker ID.
        speaker = wav_path.parent.name
        if speaker not in speakers:
            speakers[speaker] = []
        speakers[speaker].append(str(wav_path))

    if len(speakers) < 2:
        # Flat directory: treat each file as a different speaker.
        for wav_path in sorted(source_dir.rglob("*.wav")):
            speaker = wav_path.stem
            speakers[speaker] = [str(wav_path)]

    speaker_ids = list(speakers.keys())
    print(f"Found {len(speaker_ids)} speakers, {sum(len(v) for v in speakers.values())} files")

    snr_min, snr_max = args.snr_range

    for i in range(args.num):
        # Pick two different speakers.
        spk_a, spk_b = random.sample(speaker_ids, 2)
        file_a = random.choice(speakers[spk_a])
        file_b = random.choice(speakers[spk_b])

        snr = random.uniform(snr_min, snr_max)

        try:
            source = load_audio(file_a, args.sample_rate)
            interferer = load_audio(file_b, args.sample_rate)

            # Ensure minimum length (3 seconds).
            min_samples = 3 * args.sample_rate
            if len(source) < min_samples or len(interferer) < min_samples:
                continue

            mixture, source_clean, _ = mix_at_snr(source[:min_samples], interferer[:min_samples], snr)

            mix_path = output_dir / f"mix_{i:06d}.wav"
            target_path = output_dir / f"target_{i:06d}.wav"
            sf.write(str(mix_path), mixture, args.sample_rate)
            sf.write(str(target_path), source_clean, args.sample_rate)

            # Save metadata.
            meta_path = output_dir / f"meta_{i:06d}.txt"
            meta_path.write_text(f"{spk_a}\t{file_a}\t{spk_b}\t{file_b}\t{snr:.1f}\n")

        except Exception as e:
            print(f"Skipping mixture {i}: {e}")
            continue

        if (i + 1) % 1000 == 0:
            print(f"Generated {i + 1}/{args.num} mixtures")

    print(f"Done. Output in {output_dir}")


if __name__ == "__main__":
    main()
