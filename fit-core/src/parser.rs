// parser.rs — thin file-reading wrapper around the binary FIT parser.
//
// All the real parsing lives in dev_fields::parse_fit_activity_from_bytes.
// This module is responsible for I/O: reading the file into a byte buffer and
// forwarding it to the parser.  The fitparser crate is no longer needed.
//
// Learning notes:
//   - fs::read returns Vec<u8>, which owns the bytes → no lifetime worries
//   - The parser returns FitActivity directly, so no merge step needed
//   - ParseError only needs the std::io::Error variant now

use std::{fs, path::Path};

use crate::dev_fields::{parse_fit_activity_from_bytes, parse_fit_metadata_from_bytes};
use crate::models::{FitActivity, FitMetadata, FitRecord};

// ── Error type ──────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("could not read file: {0}")]
    Io(#[from] std::io::Error),
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Parse a `.fit` file at the given path into a `FitActivity`.
///
/// # Example
/// ```no_run
/// let activity = fit_core::parse_fit_file("my_run.fit").unwrap();
/// println!("{} records", activity.records.len());
/// ```
pub fn parse_fit_file<P: AsRef<Path>>(path: P) -> Result<FitActivity, ParseError> {
    let data = fs::read(path)?;
    Ok(parse_fit_activity_from_bytes(&data))
}

/// Parse activity-level metadata from a `.fit` file.
///
/// This is a lightweight parse — it reads only `file_id` (MesgNum 0),
/// `session` (MesgNum 18), and `device_info` (MesgNum 23) messages,
/// skipping all per-second Record messages entirely.
///
/// # Example
/// ```no_run
/// let meta = fit_core::parse_fit_metadata("my_run.fit").unwrap();
/// println!("{:?}", meta.manufacturer);  // Some("garmin")
/// ```
pub fn parse_fit_metadata<P: AsRef<Path>>(path: P) -> Result<FitMetadata, ParseError> {
    let data = fs::read(path)?;
    Ok(parse_fit_metadata_from_bytes(&data))
}

/// Return a human-readable dump of `max_records` representative Record
/// messages.  Used by `fit-cli dump` to inspect what channels are present.
/// Each inner Vec is one record: `(name, value, unit)`.
///
/// The function skips the first few records at the start of a run where GPS
/// has not locked yet and developer fields have not all been registered,
/// looking for records that carry the most data.
pub fn dump_raw_records<P: AsRef<Path>>(
    path: P,
    max_records: usize,
) -> Result<Vec<Vec<(String, String, String)>>, ParseError> {
    let data = fs::read(path)?;
    let activity = parse_fit_activity_from_bytes(&data);

    let total = activity.records.len();

    // Find the first record that looks like proper mid-run data: has speed,
    // heart rate, and at least one developer field.  Skip the first 30
    // records in case the GPS or dev-field registration is still warming up.
    let start = activity.records.iter()
        .skip(30.min(total / 10))     // skip ≤10 % of the activity
        .position(|r| r.speed.is_some() && r.heart_rate.is_some() && r.form_power.is_some())
        .map(|i| i + 30.min(total / 10))
        .unwrap_or(0);

    let mut out = Vec::new();
    for record in activity.records.iter().skip(start) {
        out.push(record_to_row(record));
        if out.len() >= max_records {
            break;
        }
    }
    Ok(out)
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Flatten a FitRecord into a list of (name, value_string, unit) tuples,
/// skipping None fields.
fn record_to_row(r: &FitRecord) -> Vec<(String, String, String)> {
    let mut row = Vec::new();

    macro_rules! field {
        ($name:expr, $val:expr, $unit:expr) => {
            if let Some(v) = $val {
                row.push(($name.to_string(), format!("{v:.4}"), $unit.to_string()));
            }
        };
    }

    field!("timestamp",            Some(r.timestamp as f64), "s (UNIX)");
    field!("latitude",             r.latitude,               "deg");
    field!("longitude",            r.longitude,              "deg");
    field!("altitude",             r.altitude,               "m");
    field!("speed",                r.speed,                  "m/s");
    field!("distance",             r.distance,               "m");
    field!("heart_rate",           r.heart_rate.map(|v| v as f64), "bpm");
    field!("cadence",              r.cadence.map(|v| v as f64),    "spm");
    field!("power",                r.power.map(|v| v as f64),      "W");
    field!("vertical_oscillation", r.vertical_oscillation,   "mm");
    field!("stance_time",          r.stance_time,            "ms");
    field!("stride_height",        r.stride_height,          "mm");
    field!("stride_length",        r.stride_length,          "mm");
    field!("form_power",           r.form_power,             "W");
    field!("leg_spring_stiffness", r.leg_spring_stiffness,   "KN/m");
    field!("air_power",            r.air_power,              "W");
    field!("impact_loading_rate",  r.impact_loading_rate,    "BW/s");

    row
}
