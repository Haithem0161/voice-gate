"""
VoiceFilterLite: causal GRU mask predictor for target speaker extraction.

Takes a magnitude spectrogram + speaker embedding and predicts a
time-frequency mask that isolates the target speaker's voice.

Architecture:
  1. Project embedding: Linear(256, 64) -> expand to [B, T, 64]
  2. Concat with magnitude: [B, T, 513 + 64] = [B, T, 577]
  3. Causal GRU: 2 layers, hidden=128 -> [B, T, 128]
  4. Mask head: Linear(128, 513) + Sigmoid -> [B, T, 513]

Parameters: ~900K (~3.5MB FP32 ONNX)

ONNX inputs:
  - "magnitude": f32[1, T, 513]
  - "embedding": f32[1, 256]
  - "state": f32[2, 1, 128]

ONNX outputs:
  - "mask": f32[1, T, 513]
  - "stateN": f32[2, 1, 128]
"""

import torch
import torch.nn as nn


class VoiceFilterLite(nn.Module):
    def __init__(
        self,
        num_freq_bins: int = 513,
        embedding_dim: int = 256,
        embedding_proj_dim: int = 64,
        gru_hidden: int = 128,
        gru_layers: int = 2,
    ):
        super().__init__()
        self.num_freq_bins = num_freq_bins
        self.embedding_dim = embedding_dim
        self.gru_hidden = gru_hidden
        self.gru_layers = gru_layers

        self.emb_proj = nn.Linear(embedding_dim, embedding_proj_dim)
        self.gru = nn.GRU(
            input_size=num_freq_bins + embedding_proj_dim,
            hidden_size=gru_hidden,
            num_layers=gru_layers,
            batch_first=True,
            bidirectional=False,  # causal for real-time
        )
        self.mask_head = nn.Sequential(
            nn.Linear(gru_hidden, num_freq_bins),
            nn.Sigmoid(),
        )

    def forward(
        self,
        magnitude: torch.Tensor,
        embedding: torch.Tensor,
        state: torch.Tensor,
    ) -> tuple[torch.Tensor, torch.Tensor]:
        """
        Args:
            magnitude: [B, T, F] input magnitude spectrogram
            embedding: [B, E] speaker embedding (L2-normalized)
            state: [num_layers, B, H] GRU hidden state

        Returns:
            mask: [B, T, F] predicted mask in [0, 1]
            stateN: [num_layers, B, H] updated GRU state
        """
        B, T, F = magnitude.shape

        # Project embedding and repeat across time.
        emb = self.emb_proj(embedding)          # [B, D']
        emb = emb.unsqueeze(1).expand(-1, T, -1)  # [B, T, D']

        # Concatenate magnitude features with speaker embedding.
        x = torch.cat([magnitude, emb], dim=-1)  # [B, T, F + D']

        # Causal GRU.
        x, stateN = self.gru(x, state)  # [B, T, H], [L, B, H]

        # Predict mask.
        mask = self.mask_head(x)  # [B, T, F]

        return mask, stateN

    def initial_state(self, batch_size: int = 1) -> torch.Tensor:
        """Return zero-initialized GRU state."""
        return torch.zeros(self.gru_layers, batch_size, self.gru_hidden)


def count_parameters(model: nn.Module) -> int:
    return sum(p.numel() for p in model.parameters() if p.requires_grad)


if __name__ == "__main__":
    model = VoiceFilterLite()
    print(f"Parameters: {count_parameters(model):,}")
    print(f"Estimated ONNX size: {count_parameters(model) * 4 / 1024 / 1024:.1f} MB (FP32)")

    # Quick forward pass test.
    mag = torch.randn(1, 3, 513)  # 3 STFT frames
    emb = torch.randn(1, 256)
    state = model.initial_state(1)

    mask, stateN = model(mag, emb, state)
    print(f"Input: magnitude {mag.shape}, embedding {emb.shape}, state {state.shape}")
    print(f"Output: mask {mask.shape}, stateN {stateN.shape}")
    print(f"Mask range: [{mask.min().item():.4f}, {mask.max().item():.4f}]")
