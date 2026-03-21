// fit-cli: command-line wrapper around fit-core.
//
// Config is read from `config.toml` in the current directory.  Any value can
// be overridden on the command line.  This lets you change only the smoother
// without re-typing all the channel names.
//
// Usage:
//   fit-cli <file.fit> [channels] [smoother] [param] [min_speed_ms]
//
//   channels    Comma-separated list, e.g. "heart_rate,stride_length".
//               Omit to use the list from config.toml.
//   smoother    sma | ema | none   (default from config.toml)
//   param       Window for sma, alpha for ema  (default from config.toml)
//   min_speed_ms  Drop records below this m/s; smoother resets at each stop.
//
// Special channels:
//   dump        Show a representative mid-run record with all fields decoded.
//   scan        Show value statistics for every raw field number in the file.
//
// Output is CSV on stdout:
//   timestamp, distance_m, raw_ch1, smoothed_ch1, raw_ch2, smoothed_ch2, …
//
// Pipe to a file and plot in Python with matplotlib or pandas.

use std::{env, fs, process};

use serde::Deserialize;

use fit_core::{
    dump_raw_records, parse_fit_file, scan_record_fields,
    smoothing::{ExponentialMA, MovingAverage, Smoother},
};

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Config {
    #[serde(default)]
    defaults: Defaults,
}

#[derive(Deserialize)]
struct Defaults {
    #[serde(default = "default_channels")]
    channels: Vec<String>,

    #[serde(default = "default_smoother")]
    smoother: String,

    #[serde(default = "default_param")]
    param: f64,

    #[serde(default)]
    min_speed_ms: Option<f64>,
}

fn default_channels() -> Vec<String> {
    vec!["heart_rate".to_string()]
}
fn default_smoother() -> String { "none".to_string() }
fn default_param()   -> f64    { 10.0 }

impl Default for Defaults {
    fn default() -> Self {
        Self {
            channels:     default_channels(),
            smoother:     default_smoother(),
            param:        default_param(),
            min_speed_ms: None,
        }
    }
}

fn load_config() -> Defaults {
    let Ok(text) = fs::read_to_string("config.toml") else {
        return Defaults::default();
    };
    match toml::from_str::<Config>(&text) {
        Ok(cfg) => cfg.defaults,
        Err(e)  => {
            eprintln!("Warning: could not parse config.toml: {e}");
            Defaults::default()
        }
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: fit-cli <file.fit> [channels] [sma|ema|none] [param] [min_speed_ms]");
        eprintln!("  channels: comma-separated, e.g. heart_rate,stride_length");
        eprintln!("  Omit channels/smoother/param to use values from config.toml");
        process::exit(1);
    }

    let path    = &args[1];
    let channel = args.get(2).map(String::as_str).unwrap_or("");

    // ── Diagnostic modes ──────────────────────────────────────────────────────

    if channel == "scan" {
        let raw = match fs::read(path) {
            Ok(b)  => b,
            Err(e) => { eprintln!("Error reading file: {e}"); process::exit(1); }
        };
        let stats = scan_record_fields(&raw);
        println!("{:<6}  {:>7}  {:>12}  {:>12}  samples", "field", "records", "min", "max");
        println!("{}", "-".repeat(62));
        for s in &stats {
            let samples: Vec<String> = s.samples.iter().map(|v| format!("{v:.0}")).collect();
            println!("{:<6}  {:>7}  {:>12.2}  {:>12.2}  [{}]",
                s.field_num, s.count, s.min, s.max, samples.join(", "));
        }
        return;
    }

    if channel == "dump" {
        match dump_raw_records(path, 2) {
            Err(e) => { eprintln!("Error: {e}"); process::exit(1); }
            Ok(records) => {
                for (i, record) in records.iter().enumerate() {
                    println!("=== Record {} ({} fields) ===", i + 1, record.len());
                    for (name, value, units) in record {
                        println!("  {name:<35} {value}  [{units}]");
                    }
                }
            }
        }
        return;
    }

    // ── Load config, then apply CLI overrides ─────────────────────────────────

    let cfg = load_config();

    // Channels: CLI arg wins over config.toml.  CLI arg is comma-separated.
    let channels: Vec<String> = if channel.is_empty() {
        cfg.channels.clone()
    } else {
        channel.split(',').map(|s| s.trim().to_string()).collect()
    };

    let smoother_name = args.get(3).map(String::as_str)
        .unwrap_or(cfg.smoother.as_str())
        .to_string();

    let param: f64 = args.get(4)
        .and_then(|s| s.parse().ok())
        .unwrap_or(cfg.param);

    let min_speed: Option<f64> = args.get(5)
        .and_then(|s| s.parse().ok())
        .or(cfg.min_speed_ms);

    // ── Parse ─────────────────────────────────────────────────────────────────

    let activity = match parse_fit_file(path) {
        Ok(a)  => a,
        Err(e) => { eprintln!("Error: {e}"); process::exit(1); }
    };

    eprintln!("Loaded {} records  |  sport: {}",
        activity.records.len(),
        activity.sport.as_deref().unwrap_or("unknown"));

    if let Some(t) = min_speed {
        eprintln!("Speed filter: {t:.1} m/s — smoother resets at each stop");
    }

    let channel_refs: Vec<&str> = channels.iter().map(String::as_str).collect();

    // ── Extract all channels in one pass ──────────────────────────────────────
    //
    // extract_channels returns Vec<segment> where each segment is
    // Vec<(record_index, Vec<Option<f64>>)>.  The inner Vec has one slot per
    // channel in the same order as `channel_refs`.

    let segments = activity.extract_channels(&channel_refs, min_speed);

    eprintln!("{} running segment(s)  |  channels: {}",
        segments.len(), channels.join(", "));

    // ── Smoothing helper ──────────────────────────────────────────────────────
    //
    // Takes a column of Option<f64> (one entry per record in the segment).
    // Collects only the Some values, smooths them, then scatters them back to
    // the same positions — records where the channel was absent stay None.
    // A fresh smoother is created each call, so state never crosses segments.

    let smooth_col = |col: &[Option<f64>]| -> Vec<Option<f64>> {
        let (positions, values): (Vec<usize>, Vec<f64>) = col.iter()
            .enumerate()
            .filter_map(|(i, &v)| v.map(|x| (i, x)))
            .unzip();

        if values.is_empty() {
            return vec![None; col.len()];
        }

        let smoothed: Vec<f64> = match smoother_name.as_str() {
            "sma" => MovingAverage::new(param as usize).smooth(&values),
            "ema" => ExponentialMA::new(param).smooth(&values),
            _     => values.clone(),
        };

        let mut result = vec![None; col.len()];
        for (&pos, sm) in positions.iter().zip(smoothed) {
            result[pos] = Some(sm);
        }
        result
    };

    // ── CSV header ────────────────────────────────────────────────────────────

    let col_headers: String = channels.iter()
        .flat_map(|ch| [format!("raw_{ch}"), format!("smoothed_{ch}")])
        .collect::<Vec<_>>()
        .join(",");
    println!("timestamp,distance_m,{col_headers}");

    // ── Per-segment output ────────────────────────────────────────────────────

    let mut total_rows = 0usize;

    for segment in &segments {
        if segment.is_empty() { continue; }

        let n = segment.len();

        // Transpose from row-oriented to column-oriented.
        // col[c][r] = value of channel c in row r of this segment.
        let cols: Vec<Vec<Option<f64>>> = (0..channels.len())
            .map(|c| segment.iter().map(|(_, vals)| vals[c]).collect())
            .collect();

        // Smooth each column independently with a fresh smoother.
        let smoothed_cols: Vec<Vec<Option<f64>>> = cols.iter()
            .map(|col| smooth_col(col))
            .collect();

        // Write one CSV row per record in this segment.
        for row in 0..n {
            let rec_idx = segment[row].0;
            let record  = &activity.records[rec_idx];
            let ts      = record.timestamp;
            let dist    = record.distance.unwrap_or(0.0);

            let cells: String = (0..channels.len())
                .flat_map(|c| {
                    let raw = fmt_opt(cols[c][row]);
                    let sm  = fmt_opt(smoothed_cols[c][row]);
                    [raw, sm]
                })
                .collect::<Vec<_>>()
                .join(",");

            println!("{ts},{dist:.1},{cells}");
            total_rows += 1;
        }
    }

    eprintln!("{total_rows} rows written");
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn fmt_opt(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{x:.4}"),
        None    => String::new(),   // blank cell in CSV — easy to handle in pandas/matplotlib
    }
}
