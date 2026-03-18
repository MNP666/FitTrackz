// parser.rs — wraps the `fitparser` crate and converts its generic types
// into our own FitActivity / FitRecord types.
//
// Key learning moments here:
//   - std::fs::File + passing it to a library by mutable reference
//   - Iterating over heterogeneous message types with match
//   - Converting fitparser's Value enum to Rust primitives
//   - The FIT semicircle coordinate system
//   - How Coros repurposes standard FIT field numbers for proprietary metrics

use std::{fs::File, path::Path};

use fitparser::{FitDataField, Value};

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
/// messages that contain developer fields.  Used by `fit-cli dump` to discover
/// exactly what names fitparser assigns so we can fix the parser if needed.
///
/// Each inner Vec is one record: `(field_name, display_value, units)`.
pub fn dump_raw_records<P: AsRef<Path>>(
    path: P,
    max_records: usize,
) -> Result<Vec<Vec<(String, String, String)>>, ParseError> {
    let mut file = File::open(path)?;
    let fit_data = fitparser::from_reader(&mut file)?;

    let mut out = Vec::new();

    for data_record in fit_data {
        if data_record.kind() != fitparser::profile::MesgNum::Record {
            continue;
        }
        let fields = data_record.fields();

        // Only show records that have more than the basic fields (likely have
        // developer data).  10 is a reasonable threshold based on the file.
        if fields.len() < 10 {
            continue;
        }

        let row: Vec<(String, String, String)> = fields
            .iter()
            .map(|f| {
                (
                    f.name().to_string(),
                    format!("{:?}", f.value()),
                    f.units().to_string(),
                )
            })
            .collect();

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
    let mut file = File::open(path)?;

    // fitparser::from_reader returns a Vec of FitDataRecord, one per FIT message.
    let fit_data = fitparser::from_reader(&mut file)?;

    let mut sport: Option<String> = None;
    let mut records: Vec<FitRecord> = Vec::new();

    for data_record in fit_data {
        match data_record.kind() {
            fitparser::profile::MesgNum::Record => {
                if let Some(record) = decode_record(data_record.fields()) {
                    records.push(record);
                }
            }
            fitparser::profile::MesgNum::Sport => {
                // The "sport" message tells us what type of activity this is.
                sport = find_string(data_record.fields(), "sport");
            }
            // There are many other message types (Session, Lap, DeviceInfo…).
            // We ignore them for now — great place to extend later.
            _ => {}
        }
    }

    Ok(FitActivity { sport, records })
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// The FIT protocol stores lat/lon as 32-bit signed integers called "semicircles".
/// Multiply by this constant to get degrees.
const SEMICIRCLES_TO_DEGREES: f64 = 180.0 / (2u64.pow(31) as f64);

fn decode_record(fields: &[FitDataField]) -> Option<FitRecord> {
    // Every record must have a timestamp; if it doesn't, skip it.
    let timestamp = find_u32(fields, "timestamp")?;

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

        // ── Coros running dynamics (repurposed FIT fields) ─────────────────────
        //
        // Coros stores proprietary metrics in standard FIT field numbers that are
        // either unused or mean something different in the FIT SDK.  The values
        // are raw integers that need ÷10 to reach the real unit.
        //
        // Field 83 — FIT SDK name: "motor_power" (an e-bike field).
        //            Coros uses it for a proprietary stride height metric (mm).
        //
        // Field 85 — FIT SDK name varies by profile version.
        //            Coros uses it for stride length (mm).
        //            Try "unknown_85" first; if that returns None, uncomment
        //            the next .or_else() lines to try alternative names.
        stride_height: find_f64(fields, "motor_power")
                           .map(|v| v / 10.0),

        stride_length: find_f64(fields, "unknown_85")
                           // Depending on fitparser version the field may have
                           // a profile-derived name instead — uncomment if needed:
                           // .or_else(|| find_f64(fields, "enhanced_respiration_rate"))
                           // .or_else(|| find_f64(fields, "left_pedal_power_phase_peak"))
                           .map(|v| v / 10.0),

        // ── Coros developer fields ─────────────────────────────────────────────
        // fitparser reads the FieldDescription messages embedded in the file and
        // names these fields by their exact string from the watch firmware.
        // We confirmed the names by inspecting the binary: "Form Power", etc.
        form_power:           find_f64(fields, "Form Power"),
        leg_spring_stiffness: find_f64(fields, "Leg Spring Stiffness"),
        air_power:            find_f64(fields, "Air Power"),
        impact_loading_rate:  find_f64(fields, "Impact Loading Rate"),
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
