// parser.rs — wraps the `fitparser` crate and converts its generic types
// into our own FitActivity / FitRecord types.
//
// Key learning moments here:
//   - std::fs::File + passing it to a library by mutable reference
//   - Iterating over heterogeneous message types with match
//   - Converting fitparser's Value enum to Rust primitives
//   - The FIT semicircle coordinate system
//   - How Coros repurposes standard FIT field numbers for proprietary metrics
//
// Developer fields (Form Power, Leg Spring Stiffness, etc.) are NOT exposed
// by fitparser 0.6 through its standard API.  We build them separately via
// our own binary parser in dev_fields.rs and merge at decode time.

use std::{fs, fs::File, path::Path};

use fitparser::{FitDataField, Value};

use crate::dev_fields::{build_dev_field_store, DevFieldStore};
use crate::models::{FitActivity, FitRecord};

// ── Error type ──────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("could not open file: {0}")]
    Io(#[from] std::io::Error),

    #[error("fitparser error: {0}")]
    Fit(#[from] fitparser::Error),
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Return the raw field names and values from the first `max_records` Record
/// messages that contain many fields.  Used by `fit-cli dump` to discover
/// exactly what names fitparser assigns.
///
/// Each inner Vec is one record: `(field_name, display_value, units)`.
pub fn dump_raw_records<P: AsRef<Path>>(
    path: P,
    max_records: usize,
) -> Result<Vec<Vec<(String, String, String)>>, ParseError> {
    // Standard fitparser fields
    let mut file = File::open(path.as_ref())?;
    let fit_data = fitparser::from_reader(&mut file)?;

    // Developer fields via our own binary parser
    let raw = fs::read(path.as_ref())?;
    let dev_store = build_dev_field_store(&raw);

    let mut out = Vec::new();

    for data_record in fit_data {
        if data_record.kind() != fitparser::profile::MesgNum::Record {
            continue;
        }
        let fields = data_record.fields();
        if fields.len() < 10 {
            continue;
        }

        // Standard fields from fitparser
        let mut row: Vec<(String, String, String)> = fields
            .iter()
            .map(|f| (f.name().to_string(), format!("{:?}", f.value()), f.units().to_string()))
            .collect();

        // Developer fields from our binary parser, labelled clearly
        if let Some(ts) = find_u32(fields, "timestamp") {
            if let Some(dev) = dev_store.get(&ts) {
                for (name, value) in dev {
                    row.push((
                        format!("[dev] {name}"),
                        format!("{value:.4}"),
                        String::new(),
                    ));
                }
            }
        }

        out.push(row);
        if out.len() >= max_records {
            break;
        }
    }

    Ok(out)
}

/// Parse a `.fit` file at the given path into a `FitActivity`.
///
/// # Example
/// ```no_run
/// let activity = fit_core::parse_fit_file("my_run.fit").unwrap();
/// println!("{} records", activity.records.len());
/// ```
pub fn parse_fit_file<P: AsRef<Path>>(path: P) -> Result<FitActivity, ParseError> {
    // ── Pass 1: build developer field store from raw bytes ───────────────────
    // fitparser 0.6 does not expose developer fields through its API, so we
    // read the raw bytes once and extract them ourselves.
    let raw = fs::read(path.as_ref())?;
    let dev_store = build_dev_field_store(&raw);

    // ── Pass 2: parse standard fields with fitparser ─────────────────────────
    let mut file = File::open(path)?;
    let fit_data = fitparser::from_reader(&mut file)?;

    let mut sport: Option<String> = None;
    let mut records: Vec<FitRecord> = Vec::new();

    for data_record in fit_data {
        match data_record.kind() {
            fitparser::profile::MesgNum::Record => {
                if let Some(record) = decode_record(data_record.fields(), &dev_store) {
                    records.push(record);
                }
            }
            fitparser::profile::MesgNum::Sport => {
                sport = find_string(data_record.fields(), "sport");
            }
            _ => {}
        }
    }

    Ok(FitActivity { sport, records })
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// The FIT protocol stores lat/lon as 32-bit signed integers called "semicircles".
/// Multiply by this constant to get degrees.
const SEMICIRCLES_TO_DEGREES: f64 = 180.0 / (2u64.pow(31) as f64);

fn decode_record(fields: &[FitDataField], dev_store: &DevFieldStore) -> Option<FitRecord> {
    // Every record must have a timestamp; if it doesn't, skip it.
    // find_u32 converts fitparser's Timestamp value to UNIX epoch seconds,
    // which matches the keys in dev_store (also UNIX epoch).
    let timestamp = find_u32(fields, "timestamp")?;

    // Look up developer fields for this exact timestamp.
    let dev = dev_store.get(&timestamp);
    let get_dev = |name: &str| -> Option<f64> { dev.and_then(|m| m.get(name)).copied() };

    Some(FitRecord {
        timestamp,

        // ── GPS ────────────────────────────────────────────────────────────────
        latitude:  find_i32(fields, "position_lat").map(|v| v as f64 * SEMICIRCLES_TO_DEGREES),
        longitude: find_i32(fields, "position_long").map(|v| v as f64 * SEMICIRCLES_TO_DEGREES),

        // ── Movement ───────────────────────────────────────────────────────────
        altitude: find_f64(fields, "enhanced_altitude")
                      .or_else(|| find_f64(fields, "altitude")),
        speed:    find_f64(fields, "enhanced_speed")
                      .or_else(|| find_f64(fields, "speed")),
        distance: find_f64(fields, "distance"),

        // ── Physiology ─────────────────────────────────────────────────────────
        heart_rate: find_u8(fields, "heart_rate"),
        cadence:    find_u8(fields, "cadence"),
        power:      find_u32(fields, "power"),

        // ── Standard running dynamics ──────────────────────────────────────────
        // fitparser applies the FIT SDK scale/offset automatically, so both of
        // these arrive in their final units (mm and ms respectively).
        vertical_oscillation: find_f64(fields, "vertical_oscillation"),
        stance_time:          find_f64(fields, "stance_time"),

        // ── Coros running dynamics (repurposed standard FIT fields) ────────────
        // Coros stores proprietary metrics in standard FIT field numbers.
        // The raw integer values need ÷10 to reach the real unit.
        //
        // Field 83 ("motor_power" in FIT SDK) → Coros stride height, mm
        // Field 85                            → Coros stride length, mm
        stride_height: find_f64(fields, "motor_power").map(|v| v / 10.0),
        stride_length: find_f64(fields, "unknown_85").map(|v| v / 10.0),

        // ── Developer fields (Coros/Stryd) ─────────────────────────────────────
        // These come from our binary parser in dev_fields.rs.
        // Field names are the exact strings stored in the FIT FieldDescription
        // messages and confirmed by binary inspection of long_run.fit.
        form_power:           get_dev("Form Power"),
        leg_spring_stiffness: get_dev("Leg Spring Stiffness"),
        air_power:            get_dev("Air Power"),
        impact_loading_rate:  get_dev("Impact Loading Rate"),
    })
}

// ── Field extractors — each looks up a named field and casts its Value ───────

fn find_field<'a>(fields: &'a [FitDataField], name: &str) -> Option<&'a Value> {
    fields.iter().find(|f| f.name() == name).map(|f| f.value())
}

fn find_u32(fields: &[FitDataField], name: &str) -> Option<u32> {
    match find_field(fields, name)? {
        Value::Timestamp(t) => Some(t.timestamp() as u32),
        Value::UInt32(v)    => Some(*v),
        Value::UInt16(v)    => Some(*v as u32),
        Value::UInt8(v)     => Some(*v as u32),
        _                   => None,
    }
}

fn find_i32(fields: &[FitDataField], name: &str) -> Option<i32> {
    match find_field(fields, name)? {
        Value::SInt32(v) => Some(*v),
        Value::SInt16(v) => Some(*v as i32),
        _                => None,
    }
}

fn find_u8(fields: &[FitDataField], name: &str) -> Option<u8> {
    match find_field(fields, name)? {
        Value::UInt8(v)  => Some(*v),
        Value::UInt16(v) => Some(*v as u8),
        _                => None,
    }
}

fn find_f64(fields: &[FitDataField], name: &str) -> Option<f64> {
    match find_field(fields, name)? {
        Value::Float64(v) => Some(*v),
        Value::Float32(v) => Some(*v as f64),
        Value::UInt32(v)  => Some(*v as f64),
        Value::UInt16(v)  => Some(*v as f64),
        Value::UInt8(v)   => Some(*v as f64),
        Value::SInt32(v)  => Some(*v as f64),
        Value::SInt16(v)  => Some(*v as f64),
        _                 => None,
    }
}

fn find_string(fields: &[FitDataField], name: &str) -> Option<String> {
    match find_field(fields, name)? {
        Value::String(s) => Some(s.clone()),
        _                => None,
    }
}
