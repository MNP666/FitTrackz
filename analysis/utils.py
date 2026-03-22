"""
analysis/utils.py — subprocess wrapper around fit-cli.

The Rust binary is the single source of truth for parsing and smoothing.
Python's job is plotting and iteration, not reimplementing any of that logic.

Typical usage
-------------
from utils import run_fit, run_fit_metadata, DEFAULT_FIT

df   = run_fit(DEFAULT_FIT)                          # use config.toml defaults
df   = run_fit(DEFAULT_FIT, smoother="ema", param=0.2)
df   = run_fit(DEFAULT_FIT, channels=["heart_rate", "stride_length"])
meta = run_fit_metadata(DEFAULT_FIT)                 # dict of activity-level info
"""

import io
import json
import subprocess
from pathlib import Path
from typing import Optional

import pandas as pd

# ── Paths ─────────────────────────────────────────────────────────────────────

# Root of the Rust workspace — one level up from this file.
REPO_ROOT = Path(__file__).parent.parent

# Default FIT file used when no path is supplied.
# Change this to whichever file you are currently working with.
DEFAULT_FIT = REPO_ROOT / "data" / "raw" / "long_run.fit"


# ── Core wrapper ──────────────────────────────────────────────────────────────

def run_fit(
    fit_file: Path | str = DEFAULT_FIT,
    channels: Optional[list[str]] = None,
    smoother: Optional[str] = None,
    param: Optional[float] = None,
    min_speed: Optional[float] = None,
) -> pd.DataFrame:
    """
    Run fit-cli and return the result as a DataFrame.

    Any argument left as None is omitted from the command, which means the
    value from config.toml is used instead.  This lets you change only the
    smoother without re-specifying all channels.

    Parameters
    ----------
    fit_file    Path to the .fit file.
    channels    List of channel names, e.g. ["heart_rate", "stride_length"].
                None → use config.toml defaults.
    smoother    "sma", "ema", or "none".  None → use config.toml default.
    param       Window size (sma) or alpha (ema).  None → config.toml default.
    min_speed   Minimum speed in m/s.  Records below this are excluded and the
                smoother resets at each stop.  None → config.toml default.

    Returns
    -------
    pd.DataFrame with columns: timestamp, distance_m, raw_<ch>, smoothed_<ch>, …
    Missing values in the CSV are returned as NaN.
    """
    channel_arg = ",".join(channels) if channels else ""

    # Build the argument list, using empty string as a placeholder so that
    # positional arguments after channels are still in the right position.
    cmd = [
        "cargo", "run", "--release", "--bin", "fit-cli", "--",
        str(fit_file),
        channel_arg,                             # "" → config.toml channels
        smoother  if smoother  is not None else "",
        str(param) if param    is not None else "",
        str(min_speed) if min_speed is not None else "",
    ]

    # Strip trailing empty strings so we don't pass spurious positional args.
    while cmd and cmd[-1] == "":
        cmd.pop()

    result = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        check=True,
        cwd=REPO_ROOT,
    )

    if not result.stdout.strip():
        raise RuntimeError(
            f"fit-cli produced no output.\nstderr:\n{result.stderr}"
        )

    df = pd.read_csv(io.StringIO(result.stdout))

    # Convert UNIX timestamp to a proper datetime for nicer axis labels.
    df["time"] = pd.to_datetime(df["timestamp"], unit="s", utc=True)

    # Elapsed minutes from the first record — useful as a simple x-axis.
    df["elapsed_min"] = (df["timestamp"] - df["timestamp"].iloc[0]) / 60.0

    return df


# ── Metadata wrapper ──────────────────────────────────────────────────────────

def run_fit_metadata(fit_file: Path | str = DEFAULT_FIT) -> dict:
    """
    Run ``fit-cli <file> metadata`` and return the result as a plain dict.

    Keys mirror the ``FitMetadata`` Rust struct fields:
        manufacturer, product_name, serial_number, time_created,
        sport, sub_sport, start_time, total_elapsed_s, total_timer_s,
        total_distance_m, total_ascent_m, total_descent_m, total_calories,
        avg_speed_ms, max_speed_ms, avg_heart_rate, max_heart_rate,
        avg_cadence, avg_power_w, max_power_w, training_stress_score,
        firmware_version

    All values are either a Python scalar or ``None`` when the FIT file
    did not include that field.

    ``time_created`` and ``start_time`` are raw FIT epoch seconds
    (seconds since 1989-12-31 00:00:00 UTC).  Add 631_065_600 to convert
    to UNIX time, or use ``pd.to_datetime(value + 631_065_600, unit='s', utc=True)``.
    """
    cmd = [
        "cargo", "run", "--release", "--bin", "fit-cli", "--",
        str(fit_file),
        "metadata",
    ]
    result = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        check=True,
        cwd=REPO_ROOT,
    )
    if not result.stdout.strip():
        raise RuntimeError(
            f"fit-cli metadata produced no output.\nstderr:\n{result.stderr}"
        )
    return json.loads(result.stdout)


# ── Convenience helpers ───────────────────────────────────────────────────────

def channel_pairs(df: pd.DataFrame) -> list[str]:
    """
    Return the list of channel base names that have both a raw_ and a
    smoothed_ column in the DataFrame.

    Example: ["heart_rate", "stride_length"]
    """
    raw_cols = {c.removeprefix("raw_") for c in df.columns if c.startswith("raw_")}
    sm_cols  = {c.removeprefix("smoothed_") for c in df.columns if c.startswith("smoothed_")}
    return sorted(raw_cols & sm_cols)


def channel_units() -> dict[str, str]:
    """Human-readable units for each channel, for axis labels."""
    return {
        "heart_rate":           "bpm",
        "cadence":              "strides/min",
        "speed":                "m/s",
        "power":                "W",
        "distance":             "m",
        "altitude":             "m",
        "vertical_oscillation": "mm",
        "stance_time":          "ms",
        "stride_height":        "mm",
        "stride_length":        "mm",
        "form_power":           "W",
        "leg_spring_stiffness": "kN/m",
        "air_power":            "W",
        "impact_loading_rate":  "BW/s",
    }
