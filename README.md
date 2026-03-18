# FitTrackz

A Rust library for parsing `.fit` files from GPS running watches and applying signal smoothing algorithms to activity data. The parser is written entirely from scratch in Rust — no third-party FIT library — giving full access to every field including proprietary Coros running dynamics. The project is also a Rust learning exercise, with a planned path to Python bindings via PyO3.

## Project structure

```
FitTrackz/
├── fit-core/           # Library crate — all the real logic
│   └── src/
│       ├── lib.rs          # Public API re-exports
│       ├── models.rs       # FitRecord, FitActivity data model
│       ├── dev_fields.rs   # Binary FIT parser (standard + developer fields)
│       ├── parser.rs       # File I/O wrapper, dump/scan helpers
│       └── smoothing/
│           ├── mod.rs          # Smoother trait
│           ├── moving_avg.rs   # Simple moving average (SMA)
│           └── exponential.rs  # Exponential moving average (EMA)
├── fit-cli/            # Binary crate — CLI test harness
│   └── src/main.rs
└── fit-py/             # (planned) Python bindings via PyO3
```

See [fit-core/PARSING.md](fit-core/PARSING.md) for a technical deep-dive into the FIT binary format and how the parser works.

## Prerequisites

- [Rust](https://rustup.rs) (stable, 1.70+)

## Quick start

```bash
# Check everything compiles
cargo check

# Run the unit tests
cargo test

# Parse a .fit file and dump heart rate as CSV
cargo run --bin fit-cli -- path/to/activity.fit heart_rate

# Smooth heart rate with a 10-point moving average
cargo run --bin fit-cli -- path/to/activity.fit heart_rate sma 10

# Filter out red-light stops (< 1 m/s), then smooth
cargo run --bin fit-cli -- path/to/activity.fit form_power sma 10 1.0
```

## Available channels

| Channel | Source | Unit |
|---------|--------|------|
| `heart_rate` | Standard FIT field 3 | bpm |
| `cadence` | Standard FIT field 4 | strides/min (×2 = steps/min) |
| `speed` | Standard FIT field 6 or 136 | m/s |
| `power` | Standard FIT field 7 | W |
| `distance` | Standard FIT field 5 | m |
| `altitude` | Standard FIT field 2 or 78 | m |
| `vertical_oscillation` | Standard FIT field 39 | mm |
| `stance_time` | Standard FIT field 41 | ms |
| `stride_height` | Coros FIT field 83 | mm |
| `stride_length` | Coros FIT field 85 | mm |
| `form_power` | Coros developer field | W |
| `leg_spring_stiffness` | Coros developer field | kN/m |
| `air_power` | Coros developer field | W |
| `impact_loading_rate` | Coros developer field | BW/s |

## CLI usage

```
fit-cli <file.fit> [channel] [smoother] [param] [min_speed_ms]
```

| Argument | Values | Default |
|----------|--------|---------|
| `channel` | any channel from the table above, or `dump`, `scan` | `heart_rate` |
| `smoother` | `sma`, `ema`, `none` | `none` |
| `param` | window size for `sma`; alpha (0–1) for `ema` | `10` |
| `min_speed_ms` | drop records below this speed in m/s (use `1.0` to skip red-light stops) | none |

Output is CSV on stdout so you can pipe it directly into Python:

```bash
cargo run --bin fit-cli -- activity.fit heart_rate sma 10 > hr.csv
cargo run --bin fit-cli -- activity.fit stride_length sma 10 1.0 > stride.csv
```

### Diagnostic commands

```bash
# Show all fields present in the file with value statistics.
# Use this to identify unknown proprietary field numbers.
cargo run --bin fit-cli -- activity.fit scan

# Show a representative mid-run record with all channels decoded.
cargo run --bin fit-cli -- activity.fit dump
```

## Smoothing algorithms

| Algorithm | Flag | Key parameter |
|-----------|------|---------------|
| Simple moving average | `sma` | `window` — number of points (e.g. `10`) |
| Exponential moving average | `ema` | `alpha` — blend factor 0–1 (e.g. `0.2`) |
| Savitzky-Golay | *(planned)* | window + polynomial order |

### Speed filtering

Passing a `min_speed_ms` threshold drops all records where the runner was stationary or nearly so (e.g. waiting at a red light). Filtering happens before smoothing so the pause does not distort the smoothed signal. A value of `1.0` (m/s = 3.6 km/h) works well for outdoor runs.

## Plotting with Python

```python
import csv
import matplotlib.pyplot as plt

rows = list(csv.DictReader(open("stride.csv")))
ts  = [int(r["timestamp"]) for r in rows]
raw = [float(r["raw_stride_length"]) for r in rows]
sm  = [float(r["smoothed_stride_length"]) for r in rows]

plt.plot(ts, raw, alpha=0.3, label="raw")
plt.plot(ts, sm, label="sma-10")
plt.ylabel("Stride length (mm)")
plt.legend()
plt.show()
```

## Roadmap

- [x] Binary FIT parser (definition messages, data messages, developer fields)
- [x] Compressed timestamp support
- [x] Core data model (`FitRecord`, `FitActivity`)
- [x] Speed-threshold filtering (red-light stop removal)
- [x] Simple moving average
- [x] Exponential moving average
- [x] Coros PACE Pro running dynamics (stride height, stride length, form power, leg spring stiffness, air power, impact loading rate)
- [x] Field scan diagnostic tool
- [ ] Savitzky-Golay filter
- [ ] Kalman filter
- [ ] Python bindings (`fit-py` via PyO3 + maturin)

## Licence

GNU General Public License v3.0 — see [LICENSE](LICENSE).
