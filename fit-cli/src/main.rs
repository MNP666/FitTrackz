// fit-cli: a thin command-line wrapper around fit-core.
//
// Usage:
//   cargo run --bin fit-cli -- path/to/activity.fit [channel] [smoother] [param]
//
// Examples:
//   cargo run --bin fit-cli -- my_run.fit                         # dump all records as CSV
//   cargo run --bin fit-cli -- my_run.fit heart_rate              # raw HR as CSV
//   cargo run --bin fit-cli -- my_run.fit heart_rate sma 10       # HR smoothed with SMA window=10
//   cargo run --bin fit-cli -- my_run.fit heart_rate ema 0.2      # HR smoothed with EMA alpha=0.2
//
// Pipe the output to a file and plot it in Python with matplotlib.

use std::{env, process};

use fit_core::{
    parse_fit_file,
    smoothing::{ExponentialMA, MovingAverage, Smoother},
};

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: fit-cli <file.fit> [channel] [sma|ema] [window|alpha]");
        eprintln!("Channels: heart_rate, speed, altitude, power, cadence, distance");
        process::exit(1);
    }

    let path = &args[1];
    let channel = args.get(2).map(String::as_str).unwrap_or("heart_rate");
    let smoother_name = args.get(3).map(String::as_str).unwrap_or("none");
    let param: f64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(10.0);

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

    // Extract the requested channel.
    let channel_data = activity.extract_channel(channel);

    if channel_data.is_empty() {
        eprintln!("No data found for channel '{channel}'");
        process::exit(1);
    }

    let (indices, raw_values): (Vec<usize>, Vec<f64>) = channel_data.into_iter().unzip();

    // Smooth if requested.
    let smoothed: Vec<f64> = match smoother_name {
        "sma" => MovingAverage::new(param as usize).smooth(&raw_values),
        "ema" => ExponentialMA::new(param).smooth(&raw_values),
        _     => raw_values.clone(),  // "none" or anything else → pass-through
    };

    // Print CSV to stdout.  Timestamps come from the records at those indices.
    println!("timestamp,raw_{channel},smoothed_{channel}");
    for (i, (&rec_idx, (&raw, &sm))) in indices
        .iter()
        .zip(raw_values.iter().zip(smoothed.iter()))
        .enumerate()
    {
        let ts = activity.records[rec_idx].timestamp;
        println!("{ts},{raw:.4},{sm:.4}");
        let _ = i; // suppress unused warning
    }
}
