"""
Training script for VoiceFilterLite TSE model.

Usage:
    python training/train.py \
        --data-dir training/data/mixtures \
        --embeddings-dir training/data/embeddings \
        --epochs 50 \
        --batch-size 32 \
        --lr 1e-3 \
        --output training/checkpoints
"""

import argparse
import os
from pathlib import Path

import torch
import torch.nn as nn
import torch.nn.functional as F
from torch.utils.data import DataLoader, random_split

from dataset import TseDataset
from model import VoiceFilterLite, count_parameters


def si_sdr(estimated: torch.Tensor, target: torch.Tensor) -> torch.Tensor:
    """Scale-invariant signal-to-distortion ratio (in dB)."""
    # Flatten to [B, T] if needed.
    if estimated.dim() == 3:
        estimated = estimated.reshape(estimated.shape[0], -1)
        target = target.reshape(target.shape[0], -1)

    dot = torch.sum(estimated * target, dim=-1, keepdim=True)
    s_target_energy = torch.sum(target ** 2, dim=-1, keepdim=True) + 1e-8
    proj = dot * target / s_target_energy

    noise = estimated - proj
    si_sdr_val = 10 * torch.log10(
        torch.sum(proj ** 2, dim=-1) / (torch.sum(noise ** 2, dim=-1) + 1e-8) + 1e-8
    )
    return si_sdr_val.mean()


def train_epoch(
    model: VoiceFilterLite,
    loader: DataLoader,
    optimizer: torch.optim.Optimizer,
    device: torch.device,
) -> float:
    model.train()
    total_loss = 0.0
    num_batches = 0

    for mix_mag, target_mag, embedding in loader:
        mix_mag = mix_mag.to(device)       # [B, T, F]
        target_mag = target_mag.to(device)  # [B, T, F]
        embedding = embedding.to(device)    # [B, E]

        B = mix_mag.shape[0]
        state = model.initial_state(B).to(device)

        optimizer.zero_grad()
        mask, _ = model(mix_mag, embedding, state)

        # L1 loss on masked magnitude vs clean target magnitude.
        estimated_mag = mask * mix_mag
        loss = F.l1_loss(estimated_mag, target_mag)

        loss.backward()
        torch.nn.utils.clip_grad_norm_(model.parameters(), max_norm=5.0)
        optimizer.step()

        total_loss += loss.item()
        num_batches += 1

    return total_loss / max(num_batches, 1)


@torch.no_grad()
def validate(
    model: VoiceFilterLite,
    loader: DataLoader,
    device: torch.device,
) -> tuple[float, float]:
    model.eval()
    total_loss = 0.0
    total_sdr = 0.0
    num_batches = 0

    for mix_mag, target_mag, embedding in loader:
        mix_mag = mix_mag.to(device)
        target_mag = target_mag.to(device)
        embedding = embedding.to(device)

        B = mix_mag.shape[0]
        state = model.initial_state(B).to(device)

        mask, _ = model(mix_mag, embedding, state)
        estimated_mag = mask * mix_mag

        loss = F.l1_loss(estimated_mag, target_mag)
        sdr = si_sdr(estimated_mag, target_mag)

        total_loss += loss.item()
        total_sdr += sdr.item()
        num_batches += 1

    n = max(num_batches, 1)
    return total_loss / n, total_sdr / n


def main():
    parser = argparse.ArgumentParser(description="Train VoiceFilterLite TSE model")
    parser.add_argument("--data-dir", type=str, required=True)
    parser.add_argument("--embeddings-dir", type=str, required=True)
    parser.add_argument("--epochs", type=int, default=50)
    parser.add_argument("--batch-size", type=int, default=32)
    parser.add_argument("--lr", type=float, default=1e-3)
    parser.add_argument("--segment-length", type=float, default=3.0)
    parser.add_argument("--fft-size", type=int, default=1024)
    parser.add_argument("--hop-size", type=int, default=512)
    parser.add_argument("--output", type=str, default="training/checkpoints")
    parser.add_argument("--device", type=str, default="auto")
    args = parser.parse_args()

    # Device selection.
    if args.device == "auto":
        device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    else:
        device = torch.device(args.device)
    print(f"Using device: {device}")

    # Dataset.
    dataset = TseDataset(
        args.data_dir, args.embeddings_dir,
        fft_size=args.fft_size, hop_size=args.hop_size,
        segment_seconds=args.segment_length,
    )

    # Train/val split (95/5).
    val_size = max(1, int(0.05 * len(dataset)))
    train_size = len(dataset) - val_size
    train_ds, val_ds = random_split(dataset, [train_size, val_size])

    train_loader = DataLoader(
        train_ds, batch_size=args.batch_size, shuffle=True,
        num_workers=4, pin_memory=True,
    )
    val_loader = DataLoader(
        val_ds, batch_size=args.batch_size, shuffle=False,
        num_workers=2, pin_memory=True,
    )

    print(f"Train: {train_size}, Val: {val_size}")

    # Model.
    model = VoiceFilterLite(
        num_freq_bins=args.fft_size // 2 + 1,
    ).to(device)
    print(f"Model parameters: {count_parameters(model):,}")

    # Optimizer + scheduler.
    optimizer = torch.optim.Adam(model.parameters(), lr=args.lr)
    scheduler = torch.optim.lr_scheduler.CosineAnnealingLR(
        optimizer, T_max=args.epochs, eta_min=1e-5
    )

    # Training loop.
    output_dir = Path(args.output)
    output_dir.mkdir(parents=True, exist_ok=True)
    best_val_loss = float("inf")

    for epoch in range(1, args.epochs + 1):
        train_loss = train_epoch(model, train_loader, optimizer, device)
        val_loss, val_sdr = validate(model, val_loader, device)
        lr = optimizer.param_groups[0]["lr"]
        scheduler.step()

        print(
            f"Epoch {epoch:3d}/{args.epochs} | "
            f"train_loss={train_loss:.4f} | "
            f"val_loss={val_loss:.4f} | "
            f"val_sdr={val_sdr:.2f} dB | "
            f"lr={lr:.2e}"
        )

        # Save best model.
        if val_loss < best_val_loss:
            best_val_loss = val_loss
            torch.save(model.state_dict(), str(output_dir / "best.pt"))
            print(f"  -> Saved best model (val_loss={val_loss:.4f})")

        # Periodic checkpoint.
        if epoch % 10 == 0:
            torch.save(model.state_dict(), str(output_dir / f"epoch_{epoch:03d}.pt"))

    # Save final model.
    torch.save(model.state_dict(), str(output_dir / "final.pt"))
    print(f"Training complete. Best val_loss={best_val_loss:.4f}")
    print(f"Checkpoints saved to {output_dir}")


if __name__ == "__main__":
    main()
