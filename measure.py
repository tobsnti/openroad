#!/usr/bin/env python3
"""Recount the notice-line pixels in the two full frames of this branch.

A pixel counts as notice text if it is within a Manhattan distance of 120 of
NOTICE_COLOR = srgb_u8(255, 103, 29) (chrome.rs:49, the block's ARGB FontColor).
The window stops at x 960 because the diagnostics overlay on the right is open
in only one of the two shots; it holds no notice text in either.
"""
import numpy as np
from PIL import Image

WINDOW = (300, 960, 575, 660)  # x0, x1, y0, y1
ORANGE = np.array([255, 103, 29])

for name in ("full-before-1280x720.png", "full-after-1280x720.png"):
    a = np.asarray(Image.open(name).convert("RGB")).astype(np.int16)
    m = np.abs(a - ORANGE).sum(axis=2) < 120
    x0, x1, y0, y1 = WINDOW
    w = m[y0:y1, x0:x1]
    ys, xs = np.where(w)
    print(f"{name}: window={w.sum()} full_width_band={m[y0:y1].sum()} "
          f"rows={(ys.min()+y0, ys.max()+y0) if ys.size else None} "
          f"cols={(xs.min()+x0, xs.max()+x0) if xs.size else None}")
