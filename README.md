# FitTrackz

A Rust library for parsing `.fit` files (Garmin/Coros format) and applying signal smoothing algorithms to activity data. Designed as a learning project with a clear path to Python bindings via PyO3.

## Project structure

```
FitTrackz/
├── fit-core/           # Library crate — all the real logic
│   └── src/
│       ├── models.rs       # FitRecord, FitActivity
│       ├── parser.rs       # parse_fit_file()
│       └── smoothing/
│           ├── mod.rs          # Smoother trait
│           ├── moving_avg.rs   # Simple moving average
│           └── exponential.rs  # Exponential moving average
├── fit-cli/            # Binary crate — CLI test harness
│   └── src/main.rs
└── fit-py/             # (planned) Python bindings via PyO3
```

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

# Smooth with an exponential moving average (alpha = 0.2)
cargo run --bin fit-cli -- path/to/activity.fit heart_rate ema 0.2
```

## CLI usage

```
fit-cli <file.fit> [channel] [smoother] [param]
```

| Argument   | Values                                               | Default      |
|------------|------------------------------------------------------|--------------|
| `channel`  | `heart_rate`, `speed`, `altitude`, `power`, `cadence`, `distance` | `heart_rate` |
| `smoother` | `sma` (moving average), `ema` (exponential), `none` | `none`       |
| `param`    | window size for `sma`, alpha (0–1) for `ema`        | `10`         |

Output is CSV on stdout (`timestamp`, `raw_<channel>`, `smoothed_<channel>`), so you can pipe it straight into Python:

```bash
cargo run --bin fit-cli -- activity.fit heart_rate sma 10 > hr.csv
```

```python
import csv
import matplotlib.pyplot as plt

rows = list(csv.DictReader(open("hr.csv")))
ts  = [int(r["timestamp"]) for r in rows]
raw = [float(r["raw_heart_rate"]) for r in rows]
sm  = [float(r["smoothed_heart_rate"]) for r in rows]

plt.plot(ts, raw, alpha=0.4, label="raw")
plt.plot(ts, sm, label="sma-10")
plt.legend()
plt.show()
```

## Smoothing algorithms

| Algorithm | Struct | Key parameter |
|---|---|---|
| Simple moving average | `MovingAverage` | `window` — number of points (e.g. 10) |
| Exponential moving average | `ExponentialMA` | `alpha` — blend factor 0–1 (e.g. 0.2) |
| Savitzky-Golay | *(planned)* | window + polynomial order |

## Roadmap

- [x] FIT file parsing (`fitparser` crate)
- [x] Core data model (`FitRecord`, `FitActivity`)
- [x] Simple moving average
- [x] Exponential moving average
- [ ] Savitzky-Golay filter
- [ ] Kalman filter
- [ ] Python bindings (`fit-py` via PyO3 + maturin)

## Licence

GNU General Public License v3.0 — see [LICENSE](LICENSE).
