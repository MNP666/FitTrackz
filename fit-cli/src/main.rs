// fit-cli: a thin command-line wrapper around fit-core.
//
// Usage:
//   cargo run --bin fit-cli -- <file.fit> [channel] [smoother] [param] [min_speed]
//
// Examples:
//   cargo run --bin fit-cli -- run.fit                              # HR as CSV, no filter
//   cargo run --bin fit-cli -- run.fit heart_rate                  # raw HR
//   cargo run --bin fit-cli -- run.fit heart_rate sma 10           # HR with SMA window=10
//   cargo run --bin fit-cli -- run.fit heart_rate sma 10 1.0       # skip red-light stops (<1 m/s)
//   cargo run --bin fit-cli -- run.fit form_power ema 0.2 1.0      # form power, EMA, filtered
//   cargo run --bin fit-cli -- run.fit leg_spring_stiffness sma 5  # leg stiffness, SMA
//
// Available channels:
//   heart_rate, speed, altitude, power, cadence, distance,
//   vertical_oscillation, stance_time,
//   stride_height, stride_length,
//   form_power, leg_spring_stiffness, air_power, impact_loading_rate
//
// Pipe output to a file and plot it in Python with matplotlib.

use std::{env, process};

use fit_core::{
    dump_raw_records, parse_fit_file, scan_record_fields,
    smoothing::{ExponentialMA, MovingAverage, Smoother},
};

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!(
            "Usage: fit-cli <file.fit> [channel] [sma|ema|none] [window|alpha] [min_speed_ms]"
        );
        eprintln!("  channel defaults to 'heart_rate'");
        eprintln!("  min_speed_ms: drop records below this speed (m/s). Use 1.0 to skip red lights.");
        process::exit(1);
    }

    let path    = &args[1];
    let channel = args.get(2).map(String::as_str).unwrap_or("heart_rate");

    // Special mode: scan every raw field number in Record messages and print
    // value statistics.  Use this to hunt for unknown proprietary channels —
    // look for field numbers whose value range matches what you expect:
    //   stride_length ≈  900–1200  (mm × 10 in the raw encoding → 9000–12000)
    //   stride_height ≈   50–80   (mm × 10 → 500–800)
    //
    //   cargo run --bin fit-cli -- run.fit scan
    if channel == "scan" {
        let raw = match std::fs::read(path) {
            Ok(b)  => b,
            Err(e) => { eprintln!("Error reading file: {e}"); process::exit(1); }
        };
        let stats = scan_record_fields(&raw);
        println!("{:<6}  {:>7}  {:>12}  {:>12}  samples", "field", "records", "min", "max");
        println!("{}", "-".repeat(62));
        for s in &stats {
            let sample_str: Vec<String> = s.samples.iter().map(|v| format!("{v:.0}")).collect();
            println!(
                "{:<6}  {:>7}  {:>12.2}  {:>12.2}  [{}]",
                s.field_num, s.count, s.min, s.max,
                sample_str.join(", ")
            );
        }
        return;
    }

    // Special mode: dump all fields from the first few records in human-readable
    // form.  Run this when a channel returns no data to see what is actually
    // present.
    //   cargo run --bin fit-cli -- run.fit dump
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

    let smoother_name = args.get(3).map(String::as_str).unwrap_or("none");
    let param: f64    = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(10.0);
    let min_speed: Option<f64> = args.get(5).and_then(|s| s.parse().ok());

    // Parse the FIT file.
    let activity = match parse_fit_file(path) {
        Ok(a)  => a,
        Err(e) => { eprintln!("Error: {e}"); process::exit(1); }
    };

    eprintln!(
        "Loaded {} records  |  sport: {}",
        activity.records.len(),
        activity.sport.as_deref().unwrap_or("unknown")
    );

    // Extract the requested channel, with optional speed filter.
    let channel_data = match min_speed {
        Some(threshold) => {
            eprintln!("Speed filter: dropping records below {threshold:.1} m/s");
            activity.extract_channel_filtered(channel, threshold)
        }
        None => activity.extract_channel(channel),
    };

    if channel_data.is_empty() {
        eprintln!("No data found for channel '{channel}'");
        eprintln!("(If stride_height or stride_length is empty, see the comment in parser.rs)");
        process::exit(1);
    }

    eprintln!("{} data points after filtering", channel_data.len());

    let (indices, raw_values): (Vec<usize>, Vec<f64>) = channel_data.into_iter().unzip();

    // Smooth if requested.
    let smoothed: Vec<f64> = match smoother_name {
        "sma" => MovingAverage::new(param as usize).smooth(&raw_values),
        "ema" => ExponentialMA::new(param).smooth(&raw_values),
        _     => raw_values.clone(),  // "none" or anything else → pass-through
    };

    // Print CSV to stdout.  Timestamps come from the records at those indices.
    println!("timestamp,raw_{channel},smoothed_{channel}");
    for (&rec_idx, (&raw, &sm)) in indices
        .iter()
        .zip(raw_values.iter().zip(smoothed.iter()))
    {
        let ts = activity.records[rec_idx].timestamp;
        println!("{ts},{raw:.4},{sm:.4}");
    }
}
