"""
analysis/explore.py — quick look at a single run.

Run this script to get an overview of every channel in the file with the
settings from config.toml.  Useful as the first thing to run after collecting
a new activity.

Usage
-----
    cd FitTrackz
    python analysis/explore.py                        # uses DEFAULT_FIT
    python analysis/explore.py data/my_other_run.fit  # specific file
"""
#%%
import sys
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np


# Allow running from any directory by adding the analysis folder to the path.
sys.path.insert(0, str(Path(__file__).parent))
from utils import run_fit, channel_pairs, channel_units, DEFAULT_FIT
#%%
# ── Load data ─────────────────────────────────────────────────────────────────

fit_file = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_FIT
print(f"Loading {fit_file.name} …")


df = run_fit(fit_file) # uncomment for CLI
# df = run_fit(DEFAULT_FIT)
channels = channel_pairs(df)
units = channel_units()

print(f"  {len(df)} rows  |  {len(channels)} channels: {', '.join(channels)}")
print(f"  Duration: {df['elapsed_min'].iloc[-1]:.1f} min")
print(f"  Distance: {df['distance_m'].iloc[-1] / 1000:.2f} km")
#%%
# ── Plot ──────────────────────────────────────────────────────────────────────

n = len(channels)
fig, axes = plt.subplots(n, 1, figsize=(14, 3 * n), sharex=True)
if n == 1:
    axes = [axes]

x = df["distance_m"] / 1000   # km on the x-axis

for ax, ch in zip(axes, channels):
    raw_col = f"raw_{ch}"
    sm_col  = f"smoothed_{ch}"
    unit    = units.get(ch, "")

    ax.plot(x, df[raw_col], color='C3', alpha=0.65, linewidth=1.8, label="raw")
    ax.plot(x, df[sm_col],  color='C0', linewidth=1.5,             label="smoothed")
    ax.set_ylabel(f"{ch}\n({unit})" if unit else ch, fontsize=9)
    ax.legend(fontsize=8, loc="upper right")
    ax.grid(True, alpha=0.3)

axes[-1].set_xlabel("Distance (km)")
fig.suptitle(fit_file.stem, fontsize=12)
plt.tight_layout()
plt.show()
#%%
# ── Plot2 ──────────────────────────────────────────────────────────────────────

x_set = 'stride_length'
x_unit = unit    = units.get(x_set, "")
x_raw = df[f'raw_{x_set}']
x_smooth = df[f'smoothed_{x_set}']
comparisons = ['cadence', 'form_power', 'leg_spring_stiffness', 'stride_height'] # stick to 4
assert len(comparisons) == 4
fig, axes = plt.subplots(2,2, figsize=(10,10), sharex=True)
axes = axes.ravel()

for ax, ch in zip(axes, comparisons):
    raw_col = f"raw_{ch}"
    sm_col  = f"smoothed_{ch}"
    unit    = units.get(ch, "")


    ax.scatter(x_raw, df[raw_col], color='C3', alpha=0.45, edgecolor='k', label="raw")
    ax.scatter(x_smooth, df[sm_col],  color='C0', alpha=0.45, edgecolor='k', label="smoothed")
    ax.set_ylabel(f"{ch}\n({unit})" if unit else ch, fontsize=9)
    ax.legend(fontsize=8, loc="upper right")
    ax.grid(True, alpha=0.3)


axes[-1].set_xlabel(x_unit)
axes[-2].set_xlabel(x_unit)
fig.suptitle(fit_file.stem, fontsize=12)
plt.tight_layout()
plt.show()

# ── Plot3 ──────────────────────────────────────────────────────────────────────


comparisons = ['cadence', 'form_power', 'leg_spring_stiffness', 'stride_height', 'stride_length', 'impact_loading_rate'] # stick to 6
assert len(comparisons) == 6
fig, axes = plt.subplots(2,3, figsize=(10,10), sharex=False)
axes = axes.ravel()

for ax, ch in zip(axes, comparisons):
    raw_col = f"raw_{ch}"
    sm_col  = f"smoothed_{ch}"
    unit    = units.get(ch, "")

    min_val, max_val = df[raw_col].min(), df[raw_col].max()
    bins = np.linspace(min_val*0.95, max_val*1.05, 40) # noisy data assumed to be more spread

    ax.hist(df[raw_col], color='C3', alpha=0.45, edgecolor='k', label="raw", bins=bins)
    ax.hist(df[sm_col],  color='C0', alpha=0.45, edgecolor='k', label="smoothed", bins=bins)
    ax.set_xlabel(f"{ch}\n({unit})" if unit else ch, fontsize=9)
    ax.legend(fontsize=8, loc="upper right")
    ax.grid(True, alpha=0.3)


# axes[-1].set_xlabel(x_unit)
# axes[-2].set_xlabel(x_unit)
fig.suptitle(fit_file.stem, fontsize=12)
plt.tight_layout()
plt.show()