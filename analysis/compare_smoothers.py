"""
analysis/compare_smoothers.py — compare smoothing settings side by side.

Iterates over a grid of algorithms and parameters for a chosen channel,
calling fit-cli once per combination.  No CSV files are written — everything
stays in memory.

Usage
-----
    cd FitTrackz
    python analysis/compare_smoothers.py                          # defaults
    python analysis/compare_smoothers.py stride_length            # one channel
    python analysis/compare_smoothers.py form_power data/run.fit  # channel + file
"""

import sys
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

sys.path.insert(0, str(Path(__file__).parent))
from utils import run_fit, channel_units, DEFAULT_FIT

# ── Config ────────────────────────────────────────────────────────────────────

channel  = sys.argv[1] if len(sys.argv) > 1 else "stride_length"
fit_file = Path(sys.argv[2]) if len(sys.argv) > 2 else DEFAULT_FIT

# Grid of (smoother, param, label) to compare.
# Edit this list freely — no files will be created.
VARIANTS = [
    ("none", 1,    "raw"),
    ("sma",  5,    "sma-5"),
    ("sma",  15,   "sma-15"),
    ("sma",  30,   "sma-30"),
    ("ema",  0.05, "ema-0.05"),
    ("ema",  0.2,  "ema-0.20"),
]

# ── Fetch data ────────────────────────────────────────────────────────────────

print(f"Channel: {channel}  |  File: {fit_file.name}")
print(f"Running {len(VARIANTS)} variant(s) …\n")

results = []
for smoother, param, label in VARIANTS:
    print(f"  {label} …", end="", flush=True)
    df = run_fit(fit_file, channels=[channel], smoother=smoother, param=param)
    results.append((label, df))
    print(f" {len(df)} rows")

# ── Plot ──────────────────────────────────────────────────────────────────────

unit = channel_units().get(channel, "")
fig, ax = plt.subplots(figsize=(14, 5))

x_col = "distance_m"

for i, (label, df) in enumerate(results):
    sm_col = f"smoothed_{channel}"
    if sm_col not in df.columns:
        print(f"Warning: {sm_col} not found for variant '{label}'")
        continue

    # Raw only once (first variant is "none" / the unsmoothed signal)
    if label == "raw":
        ax.plot(df[x_col] / 1000, df[sm_col],
                color="k", linewidth=0.8, zorder=0, label="raw")
    else:
        offset = np.mean(df[sm_col])*0.05
        ax.plot(df[x_col] / 1000, df[sm_col]+offset*i,
                linewidth=1.4, label=label, zorder=i + 1)

ax.set_xlabel("Distance (km)")
ax.set_ylabel(f"{channel} ({unit})" if unit else channel)
ax.set_title(f"Smoother comparison — {channel} — {fit_file.stem}")
ax.legend()
ax.grid(True, alpha=0.3)
plt.tight_layout()
plt.show()
