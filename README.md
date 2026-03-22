# FitTrackz

A Rust library for parsing `.fit` files from GPS running watches and applying signal smoothing algorithms to activity data. The parser is written entirely from scratch in Rust — no third-party FIT library — giving full access to every field including proprietary Coros running dynamics and Stryd developer fields. The project is also a Rust learning exercise, with a planned path to Python bindings via PyO3.

## Project structure

```
FitTrackz/
├── fit-core/           # Library crate — all the real logic
│   └── src/
│       ├── lib.rs          # Public API re-exports
│       ├── models.rs       # FitRecord, FitActivity, FitMetadata data models
│       ├── dev_fields.rs   # Binary FIT parser (standard + developer fields)
│       ├── parser.rs       # File I/O wrapper, dump/scan helpers
│       └── smoothing/
│           ├── mod.rs          # Smoother trait
│           ├── moving_avg.rs   # Simple moving average (SMA)
│           └── exponential.rs  # Exponential moving average (EMA)
├── fit-cli/            # Binary crate — CLI test harness
│   └── src/main.rs
├── docs/               # Technical write-ups
│   └── parsing-fit-metadata-from-coros-devices.md
└── fit-py/             # (planned) Python bindings via PyO3
```

See [fit-core/PARSING.md](fit-core/PARSING.md) for a technical deep-dive into the FIT binary format and how the parser works.

See [docs/parsing-fit-metadata-from-coros-devices.md](docs/parsing-fit-metadata-from-coros-devices.md) for a detailed account of debugging FIT metadata parsing on Coros devices — including why naïve forward-scan parsers fail and the backwards-pattern-search technique that solves it.

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

# Extract activity-level metadata as JSON (no full records parse)
cargo run --bin fit-cli -- path/to/activity.fit metadata
```

## Available channels

### Per-second record channels (`FitRecord`)

These are extracted from the continuous stream of Record messages (MesgNum 20) and exposed via `FitActivity`.

| Channel | Source | Unit | Requires |
|---------|--------|------|----------|
| `heart_rate` | FIT field 3 | bpm | Any GPS watch |
| `cadence` | FIT field 4 | strides/min | Any GPS watch |
| `speed` | FIT field 6 / 136 | m/s | Any GPS watch |
| `power` | FIT field 7 | W | Any GPS watch |
| `distance` | FIT field 5 | m | Any GPS watch |
| `altitude` | FIT field 2 / 78 | m | Any GPS watch |
| `latitude` | FIT field 0 | degrees | Any GPS watch |
| `longitude` | FIT field 1 | degrees | Any GPS watch |
| `vertical_oscillation` | FIT field 39 | mm | Any GPS watch with running dynamics |
| `stance_time` | FIT field 41 | ms | Any GPS watch with running dynamics |
| `stride_height` | Coros FIT field 83 | mm | Coros watch |
| `stride_length` | Coros FIT field 85 | mm | Coros watch |
| `form_power` | Stryd developer field | W | **Stryd pod** paired with Coros |
| `leg_spring_stiffness` | Stryd developer field | kN/m | **Stryd pod** paired with Coros |
| `air_power` | Stryd developer field | W | **Stryd pod** paired with Coros |
| `impact_loading_rate` | Stryd developer field | BW/s | **Stryd pod** paired with Coros |

The four Stryd fields are written into the file as FIT *developer fields* when a Stryd pod is connected to the watch. They will be `null` / absent on a Coros device running without a Stryd, and will not appear at all on other brands.

`stride_height` and `stride_length` are native Coros fields recorded by the watch itself — they are available on any Coros device without any additional accessory.

### Activity-level metadata (`FitMetadata`)

Extracted from `file_id` (MesgNum 0), `session` (MesgNum 18), and `device_info` (MesgNum 23) via the separate `metadata` subcommand. No per-second records are decoded, making this fast even on large files.

```json
{
  "manufacturer":           "coros",
  "product_name":           "COROS PACE Pro",
  "serial_number":          null,
  "time_created":           1141231102,
  "sport":                  "running",
  "sub_sport":              null,
  "start_time":             1141220634,
  "total_elapsed_s":        10151.56,
  "total_timer_s":          9823.74,
  "total_distance_m":       30385.53,
  "total_ascent_m":         218.0,
  "total_descent_m":        226.0,
  "total_calories":         2330,
  "avg_speed_ms":           3.093,
  "max_speed_ms":           3.846,
  "avg_heart_rate":         143,
  "max_heart_rate":         162,
  "avg_cadence":            91,
  "max_cadence":            96,
  "avg_power_w":            252,
  "max_power_w":            null,
  "training_stress_score":  236.0,
  "firmware_version":       null
}
```

Timestamp fields (`time_created`, `start_time`) are FIT epoch seconds. Add `631_065_600` for Unix time.

`null` fields are expected when the device does not record them — for example, the Coros Pace Pro does not write firmware version or max power to the session message. See [docs/parsing-fit-metadata-from-coros-devices.md](docs/parsing-fit-metadata-from-coros-devices.md) for the full field-number reference.

## CLI usage

### Record channels

```
fit-cli <file.fit> [channel] [smoother] [param] [min_speed_ms]
```

| Argument | Values | Default |
|----------|--------|---------|
| `channel` | any channel from the table above, or `dump`, `scan` | `heart_rate` |
| `smoother` | `sma`, `ema`, `none` | `none` |
| `param` | window size for `sma`; alpha (0–1) for `ema` | `10` |
| `min_speed_ms` | drop records below this speed in m/s | none |

Output is CSV on stdout:

```bash
cargo run --bin fit-cli -- activity.fit heart_rate sma 10 > hr.csv
cargo run --bin fit-cli -- activity.fit stride_length sma 10 1.0 > stride.csv
```

### Metadata

```bash
cargo run --bin fit-cli -- activity.fit metadata
```

Outputs a JSON object to stdout. Useful for populating a database without a full parse.

### Diagnostic commands

```bash
# Show all field numbers present in Record messages with value ranges.
# Use this to identify unknown proprietary fields on a new device.
cargo run --bin fit-cli -- activity.fit scan

# Show a representative mid-run record with all decoded channel values.
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
- [x] Activity metadata model (`FitMetadata`) with `metadata` CLI subcommand
- [x] Speed-threshold filtering (red-light stop removal)
- [x] Simple moving average
- [x] Exponential moving average
- [x] Coros PACE Pro native running dynamics (stride height, stride length)
- [x] Stryd developer fields (form power, leg spring stiffness, air power, impact loading rate)
- [x] Field scan diagnostic tool
- [ ] Savitzky-Golay filter
- [ ] Kalman filter
- [ ] Python bindings (`fit-py` via PyO3 + maturin)

## Licence

GNU General Public License v3.0 — see [LICENSE](LICENSE).
