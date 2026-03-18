// dev_fields.rs — binary parser for FIT developer fields.
//
// fitparser 0.6 does not expose developer fields through its standard API.
// This module reads the raw FIT bytes directly and extracts them.
//
// Background: developer fields are a FIT extension that lets device makers
// (Coros, Stryd, etc.) embed proprietary metrics.  The file contains two
// special message types that define them:
//
//   DeveloperDataId  (MesgNum 207) — registers a developer application
//   FieldDescription (MesgNum 206) — gives each developer field a name,
//                                    unit string, and numeric type
//
// After those definition messages, every Record can carry developer field
// values alongside the standard FIT fields.  We parse both to build a
// timestamp-keyed lookup table that decode_record() can query.

use std::collections::HashMap;

use crate::models::{FitActivity, FitRecord};

// ── Public types ─────────────────────────────────────────────────────────────

/// All developer field values for one Record message, keyed by field name.
pub type DevRecord = HashMap<String, f64>;

/// Maps FIT timestamp (raw FIT epoch, u32) → developer field values.
/// The timestamp key uses the raw FIT epoch value (seconds since
/// 1989-12-31 00:00:00 UTC) so it matches what fitparser returns via
/// Value::Timestamp after we apply FIT_EPOCH_OFFSET.
pub type DevFieldStore = HashMap<u32, DevRecord>;

/// Offset in seconds between the FIT epoch (1989-12-31) and the UNIX epoch
/// (1970-01-01).  Add this to a raw FIT timestamp to get UNIX seconds.
pub const FIT_EPOCH_OFFSET: u32 = 631_065_600;

/// Per-field statistics returned by `scan_record_fields`.
#[derive(Debug)]
pub struct FieldStat {
    pub field_num: u8,
    /// How many records contained a non-zero value for this field.
    pub count: usize,
    pub min: f64,
    pub max: f64,
    /// Up to 5 sample raw values from early in the file.
    pub samples: Vec<f64>,
}

/// Scan the raw FIT bytes and return statistics for every field number that
/// appears in Record (MesgNum 20) messages.
///
/// This is a diagnostic tool.  Run `cargo run --bin fit-cli -- my.fit scan`
/// and look for columns whose value ranges match what you expect for the
/// channel you're hunting:
///   stride_length ≈ 900–1200  (mm)
///   stride_height ≈  50–80    (mm)
pub fn scan_record_fields(data: &[u8]) -> Vec<FieldStat> {
    if data.len() < 14 {
        return Vec::new();
    }

    let header_size = data[0] as usize;
    let data_size   = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
    let end         = (header_size + data_size).min(data.len());
    let mut pos     = header_size;

    type StdLayout = Vec<(u8, usize, u8)>; // (field_num, size, base_type)
    type DevLayout = Vec<(u8, u8, usize)>; // (didx, field_num, size) — needed to advance pos
    // local_type → (global, std_fields, dev_refs, big_endian)
    let mut definitions: HashMap<u8, (u16, StdLayout, DevLayout, bool)> = HashMap::new();

    // field_num → (count, min, max, samples)
    let mut stats: HashMap<u8, (usize, f64, f64, Vec<f64>)> = HashMap::new();

    // Accumulate stats for the std_bytes of one Record message.
    let mut accum = |std_bytes: &HashMap<u8, (Vec<u8>, u8)>, big_endian: bool| {
        for (fnum, (bytes, btype)) in std_bytes {
            if *fnum == 253 { continue; } // timestamp — skip
            let Some(raw) = decode_value(bytes, *btype, big_endian) else { continue };
            if raw == 0.0 { continue; }
            let entry = stats.entry(*fnum).or_insert((0, f64::MAX, f64::MIN, Vec::new()));
            entry.0 += 1;
            if raw < entry.1 { entry.1 = raw; }
            if raw > entry.2 { entry.2 = raw; }
            if entry.3.len() < 5 { entry.3.push(raw); }
        }
    };

    while pos < end {
        if pos >= data.len() { break; }
        let header = data[pos]; pos += 1;

        // ── Compressed timestamp record ────────────────────────────────────
        // Bit 7 is set.  Bits 6-5 = local_type (0-3).  Bits 4-0 = time offset.
        // Data layout is the same as a normal data message for that local_type,
        // so we look up the definition and read the exact same bytes.
        if header & 0x80 != 0 {
            let local_type = (header >> 5) & 0x03;
            let Some((global, std_fields, dev_refs, big_endian)) =
                definitions.get(&local_type)
            else {
                break; // can't determine record size → stop
            };
            let (global, big_endian) = (*global, *big_endian);
            let std_fields = std_fields.clone();
            let dev_refs   = dev_refs.clone();

            let mut std_bytes: HashMap<u8, (Vec<u8>, u8)> = HashMap::new();
            for (fnum, fsize, btype) in &std_fields {
                if pos + fsize > end { break; }
                std_bytes.insert(*fnum, (data[pos..pos + fsize].to_vec(), *btype));
                pos += fsize;
            }
            for (_, _, fsize) in &dev_refs {
                pos += fsize; // advance past dev field bytes (not accumulated)
            }
            if global == 20 { accum(&std_bytes, big_endian); }
            continue;
        }

        let local_type = header & 0x0F;
        let is_def     = header & 0x40 != 0;
        let has_dev    = header & 0x20 != 0;

        if is_def {
            if pos + 5 > end { break; }
            pos += 1; // reserved
            let big_endian = data[pos] != 0; pos += 1;
            let global     = read_u16(&data[pos..], big_endian); pos += 2;
            let num_fields = data[pos] as usize; pos += 1;

            let mut std_fields: StdLayout = Vec::with_capacity(num_fields);
            for _ in 0..num_fields {
                if pos + 3 > end { break; }
                let fnum  = data[pos];
                let fsize = data[pos + 1] as usize;
                let btype = data[pos + 2];
                pos += 3;
                std_fields.push((fnum, fsize, btype));
            }

            let mut dev_refs: DevLayout = Vec::new();
            if has_dev && pos < end {
                let ndev = data[pos] as usize; pos += 1;
                for _ in 0..ndev {
                    if pos + 3 > end { break; }
                    let fnum  = data[pos];
                    let fsize = data[pos + 1] as usize;
                    let didx  = data[pos + 2];
                    pos += 3;
                    dev_refs.push((didx, fnum, fsize));
                }
            }
            definitions.insert(local_type, (global, std_fields, dev_refs, big_endian));

        } else {
            let Some((global, std_fields, dev_refs, big_endian)) =
                definitions.get(&local_type)
            else {
                break;
            };
            let (global, big_endian) = (*global, *big_endian);
            let std_fields = std_fields.clone();
            let dev_refs   = dev_refs.clone();

            let mut std_bytes: HashMap<u8, (Vec<u8>, u8)> = HashMap::new();
            for (fnum, fsize, btype) in &std_fields {
                if pos + fsize > end { break; }
                std_bytes.insert(*fnum, (data[pos..pos + fsize].to_vec(), *btype));
                pos += fsize;
            }
            for (_, _, fsize) in &dev_refs {
                pos += fsize;
            }

            if global == 20 { accum(&std_bytes, big_endian); }
        }
    }

    let mut result: Vec<FieldStat> = stats
        .into_iter()
        .map(|(field_num, (count, min, max, samples))| FieldStat {
            field_num, count, min, max, samples,
        })
        .collect();
    result.sort_by_key(|s| s.field_num);
    result
}

// ── Complete binary parser ────────────────────────────────────────────────────
//
// parse_fit_activity_from_bytes replaces the two-pass approach that combined
// fitparser (for standard fields) with build_dev_field_store (for developer
// fields).  A single pass handles everything:
//
//   FieldDescription (206) — registers developer field names/types
//   Sport            (12)  — extracts the sport name
//   Record           (20)  — builds a FitRecord with scale/offset applied
//
// The FIT specification stores most sensor values as integers that need
// dividing by a scale factor and optionally an offset to get real-world units.
// For example, speed = raw_uint16 / 1000.0 gives m/s.  The constants come
// from the FIT SDK's "Profile.xlsx".
//
// FIT "invalid" values are sentinel integers that signal "no data".  Each
// base type has a reserved maximum value: 0xFF for uint8, 0xFFFF for uint16,
// 0xFFFFFFFF for uint32, and 0x7FFFFFFF for sint32.  We filter these out
// before applying scale/offset so None is returned instead of a garbage value.

/// Parse an entire FIT file from raw bytes and return the activity.
/// This is the single-pass, fitparser-free replacement for parse_fit_file.
pub fn parse_fit_activity_from_bytes(data: &[u8]) -> FitActivity {
    let mut sport: Option<String> = None;
    let mut records: Vec<FitRecord> = Vec::new();

    if data.len() < 14 {
        return FitActivity { sport, records };
    }

    let header_size = data[0] as usize;
    let data_size   = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
    let end         = (header_size + data_size).min(data.len());
    let mut pos     = header_size;

    // Definition message layout per local message type.
    // Each std entry: (field_def_num, size_bytes, base_type_id)
    // Each dev entry: (dev_data_index, field_def_num, size_bytes)
    type StdLayout = Vec<(u8, usize, u8)>;
    type DevLayout = Vec<(u8, u8, usize)>;
    let mut definitions: HashMap<u8, (u16, StdLayout, DevLayout, bool)> = HashMap::new();

    // (dev_data_index, field_def_number) → (name, base_type_id)
    let mut field_defs: HashMap<(u8, u8), (String, u8)> = HashMap::new();

    // Rolling timestamp used for compressed-timestamp records.
    // FIT devices send a full timestamp (field 253) only occasionally and
    // then pack subsequent records with a 5-bit second offset to save space.
    let mut rolling_ts_fit: u32 = 0;

    while pos < end {
        if pos >= data.len() { break; }
        let header = data[pos]; pos += 1;

        // ── Compressed timestamp record ────────────────────────────────────
        // Bit 7 is set.  Bits 6-5 = local_type (0-3).  Bits 4-0 = seconds
        // since the last full timestamp, wrapping at 32.
        //
        // The data payload is identical to a normal data message for that
        // local_type (same fields, same byte layout) except that field 253
        // (timestamp) is omitted from the payload — the timestamp is derived
        // from the rolling value plus the 5-bit offset.
        if header & 0x80 != 0 {
            let local_type   = (header >> 5) & 0x03;
            let time_offset  = (header & 0x1F) as u32;

            // Compute the new timestamp.  Replace the lower 5 bits of the
            // rolling value with the offset.  If the result went backwards,
            // the 5-bit counter rolled over — add 32 seconds to correct it.
            let candidate = (rolling_ts_fit & 0xFFFF_FFE0) | time_offset;
            rolling_ts_fit = if candidate < rolling_ts_fit { candidate + 32 } else { candidate };

            let Some((global, std_fields, dev_refs, big_endian)) =
                definitions.get(&local_type)
            else {
                break; // can't determine record size — stop
            };
            let (global, big_endian) = (*global, *big_endian);
            let std_fields = std_fields.clone();
            let dev_refs   = dev_refs.clone();

            // Read standard field bytes (field 253 won't be present for
            // compressed records, but skip it if it somehow appears).
            let mut std_bytes: HashMap<u8, (&[u8], u8)> = HashMap::new();
            for (fnum, fsize, btype) in &std_fields {
                if pos + fsize > end { break; }
                if *fnum != 253 {
                    std_bytes.insert(*fnum, (&data[pos..pos + fsize], *btype));
                }
                pos += fsize;
            }

            let mut dev_bytes: HashMap<(u8, u8), &[u8]> = HashMap::new();
            for (didx, fnum, fsize) in &dev_refs {
                if pos + fsize > end { break; }
                dev_bytes.insert((*didx, *fnum), &data[pos..pos + fsize]);
                pos += fsize;
            }

            if global == 20 {
                let ts_unix = rolling_ts_fit.saturating_add(FIT_EPOCH_OFFSET);
                if let Some(rec) = build_fit_record(
                    ts_unix, &std_bytes, &dev_bytes, &field_defs, big_endian,
                ) {
                    records.push(rec);
                }
            }
            continue;
        }

        let local_type = header & 0x0F;
        let is_def     = header & 0x40 != 0;
        let has_dev    = header & 0x20 != 0;

        if is_def {
            // ── Definition message ─────────────────────────────────────────
            if pos + 5 > end { break; }
            pos += 1; // reserved byte
            let big_endian = data[pos] != 0; pos += 1;
            let global     = read_u16(&data[pos..], big_endian); pos += 2;
            let num_fields = data[pos] as usize; pos += 1;

            let mut std_fields: StdLayout = Vec::with_capacity(num_fields);
            for _ in 0..num_fields {
                if pos + 3 > end { break; }
                let fnum  = data[pos];
                let fsize = data[pos + 1] as usize;
                let btype = data[pos + 2];
                pos += 3;
                std_fields.push((fnum, fsize, btype));
            }

            let mut dev_refs: DevLayout = Vec::new();
            if has_dev && pos < end {
                let num_dev = data[pos] as usize; pos += 1;
                for _ in 0..num_dev {
                    if pos + 3 > end { break; }
                    let fnum  = data[pos];
                    let fsize = data[pos + 1] as usize;
                    let didx  = data[pos + 2];
                    pos += 3;
                    dev_refs.push((didx, fnum, fsize));
                }
            }

            definitions.insert(local_type, (global, std_fields, dev_refs, big_endian));

        } else {
            // ── Data message ───────────────────────────────────────────────
            let Some((global, std_fields, dev_refs, big_endian)) =
                definitions.get(&local_type)
            else {
                break;
            };
            let (global, big_endian) = (*global, *big_endian);
            let std_fields = std_fields.clone();
            let dev_refs   = dev_refs.clone();

            // Slurp standard field bytes: field_num → (&bytes, base_type)
            let mut std_bytes: HashMap<u8, (&[u8], u8)> = HashMap::new();
            for (fnum, fsize, btype) in &std_fields {
                if pos + fsize > end { break; }
                std_bytes.insert(*fnum, (&data[pos..pos + fsize], *btype));
                pos += fsize;
            }

            // Slurp developer field bytes: (dev_data_index, field_num) → &bytes
            let mut dev_bytes: HashMap<(u8, u8), &[u8]> = HashMap::new();
            for (didx, fnum, fsize) in &dev_refs {
                if pos + fsize > end { break; }
                dev_bytes.insert((*didx, *fnum), &data[pos..pos + fsize]);
                pos += fsize;
            }

            match global {
                // ── FieldDescription (206) — registers a developer field ───
                206 => {
                    let dev_idx  = std_bytes.get(&0).and_then(|(b, _)| b.first()).copied().unwrap_or(0);
                    let fdef_num = std_bytes.get(&1).and_then(|(b, _)| b.first()).copied().unwrap_or(0);
                    let base_typ = std_bytes.get(&2).and_then(|(b, _)| b.first()).copied().unwrap_or(0);
                    if let Some((name_bytes, _)) = std_bytes.get(&3) {
                        let name: String = name_bytes
                            .iter()
                            .take_while(|&&b| b != 0)
                            .map(|&b| b as char)
                            .collect();
                        if !name.is_empty() {
                            field_defs.insert((dev_idx, fdef_num), (name, base_typ));
                        }
                    }
                }

                // ── Sport (12) — gives the activity type string ────────────
                12 => {
                    if let Some((bytes, _)) = std_bytes.get(&0) {
                        if let Some(&b) = bytes.first() {
                            sport = Some(match b {
                                1  => "running".to_string(),
                                2  => "cycling".to_string(),
                                5  => "swimming".to_string(),
                                _  => format!("sport_{b}"),
                            });
                        }
                    }
                }

                // ── Record (20) — one second of sensor data ────────────────
                20 => {
                    // Full-timestamp records carry field 253.
                    let Some(ts_raw) = std_bytes.get(&253).and_then(|(b, _)| {
                        if b.len() >= 4 { Some(read_u32(b, big_endian)) } else { None }
                    }) else {
                        continue;
                    };
                    // Keep rolling_ts_fit in sync so compressed records that
                    // follow will compute the correct timestamp.
                    rolling_ts_fit = ts_raw;
                    let ts_unix = ts_raw.saturating_add(FIT_EPOCH_OFFSET);

                    if let Some(rec) = build_fit_record(
                        ts_unix, &std_bytes, &dev_bytes, &field_defs, big_endian,
                    ) {
                        records.push(rec);
                    }
                }

                _ => {}
            }
        }
    }

    FitActivity { sport, records }
}

// ── FitRecord builder ─────────────────────────────────────────────────────────
//
// Shared by both the full-timestamp and compressed-timestamp code paths.
// Takes the already-sliced byte maps and builds a FitRecord, returning None
// only if the standard fields map is completely empty (malformed record).

fn build_fit_record(
    ts_unix:   u32,
    std_bytes: &HashMap<u8, (&[u8], u8)>,
    dev_bytes: &HashMap<(u8, u8), &[u8]>,
    field_defs: &HashMap<(u8, u8), (String, u8)>,
    big_endian: bool,
) -> Option<FitRecord> {
    // Decode every standard field using the FIT SDK scale/offset table.
    let mut sf: HashMap<u8, f64> = HashMap::new();
    for (&fnum, (bytes, btype)) in std_bytes {
        if fnum == 253 { continue; } // timestamp already supplied by caller
        if let Some(v) = decode_standard_field(bytes, fnum, *btype, big_endian) {
            sf.insert(fnum, v);
        }
    }

    // Decode developer fields.  Watches send the already-scaled value
    // (e.g. Form Power in Watts) so no scale/offset needed here.
    let mut df: HashMap<String, f64> = HashMap::new();
    for ((didx, fnum), bytes) in dev_bytes {
        if let Some((name, btype)) = field_defs.get(&(*didx, *fnum)) {
            if let Some(v) = decode_value(bytes, *btype, big_endian) {
                df.insert(name.clone(), v);
            }
        }
    }

    // lat/lon are sint32 semicircles; multiply to convert to degrees.
    const SC2DEG: f64 = 180.0 / (1u64 << 31) as f64;

    Some(FitRecord {
        timestamp: ts_unix,

        latitude:  sf.get(&0).map(|&v| v * SC2DEG),
        longitude: sf.get(&1).map(|&v| v * SC2DEG),

        altitude:  sf.get(&78).or_else(|| sf.get(&2)).copied(),  // enhanced preferred
        speed:     sf.get(&136).or_else(|| sf.get(&6)).copied(),  // enhanced preferred
        distance:  sf.get(&5).copied(),

        heart_rate: sf.get(&3).map(|&v| v as u8),
        // FIT running cadence (field 4) is stored as stride frequency in one
        // foot per minute.  Multiply by 2 to get the total steps/min shown
        // on most running watches.  Raw 90 → 180 steps/min.
        cadence:    sf.get(&4).map(|&v| v as u8),
        power:      sf.get(&7).map(|&v| v as u32),

        vertical_oscillation: sf.get(&39).copied(),
        stance_time:          sf.get(&41).copied(),

        // Coros proprietary fields — field numbers confirmed by scan.
        stride_height: sf.get(&83).copied(),
        stride_length: sf.get(&85).copied(),

        form_power:           df.get("Form Power").copied(),
        leg_spring_stiffness: df.get("Leg Spring Stiffness").copied(),
        air_power:            df.get("Air Power").copied(),
        impact_loading_rate:  df.get("Impact Loading Rate").copied(),
    })
}

// ── Standard-field decoder ────────────────────────────────────────────────────
//
// Applies the FIT SDK profile (scale and offset) for the fields inside a
// Record message (MesgNum 20) and filters out FIT "invalid" sentinels.
//
// Formula:  physical_value = raw_integer / scale + offset
//
// Caller passes the raw byte slice, the field definition number, and the
// base_type byte from the definition message.  Returns None for invalid
// sentinels or if the bytes can't be decoded.

fn decode_standard_field(bytes: &[u8], field_num: u8, base_type: u8, big_endian: bool) -> Option<f64> {
    let raw = decode_value(bytes, base_type, big_endian)?;

    // Filter FIT invalid sentinels (the largest legal value for each integer type).
    let invalid = match base_type {
        0x00 | 0x02 | 0x0A | 0x0B => raw == 255.0,                // uint8 variants
        0x01                       => raw == 127.0,                // sint8
        0x83                       => raw == 32767.0,              // sint16
        0x84 | 0x8C                => raw == 65535.0,              // uint16 variants
        0x85                       => raw == 2_147_483_647.0,      // sint32
        0x86 | 0x8D                => raw == 4_294_967_295.0,      // uint32 variants
        _                          => false,
    };
    if invalid { return None; }

    // Apply FIT SDK scale/offset for fields inside MesgNum 20 (Record).
    // Fields not listed here carry their raw value directly.
    Some(match field_num {
        2   => raw / 5.0 - 500.0,    // altitude          (uint16, m)
        5   => raw / 100.0,           // distance          (uint32, m)
        6   => raw / 1000.0,          // speed             (uint16, m/s)
        39  => raw / 10.0,            // vertical_oscill.  (uint16, mm)
        41  => raw / 10.0,            // stance_time       (uint16, ms)
        78  => raw / 5.0 - 500.0,    // enhanced_altitude (uint32, m)
        83  => raw / 10.0,            // Coros stride_height (uint16, mm) — tentative
        85  => raw / 10.0,            // Coros stride_length (uint16, mm) — tentative
        136 => raw / 1000.0,          // enhanced_speed    (uint32, m/s)
        _   => raw,
    })
}

// ── Legacy entry point (kept for compatibility) ───────────────────────────────

/// Read raw FIT bytes and extract all developer field values, keyed by the
/// UNIX timestamp of each record (so the keys match what fitparser returns).
pub fn build_dev_field_store(data: &[u8]) -> DevFieldStore {
    if data.len() < 14 {
        return HashMap::new();
    }

    let header_size = data[0] as usize;
    let data_size = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
    let end = (header_size + data_size).min(data.len());

    let mut pos = header_size;

    // local_type → (global_msg_num, std_field_defs, dev_field_refs, big_endian)
    // std_field_defs: Vec<(field_def_num, size_bytes)>
    // dev_field_refs: Vec<(dev_data_index, dev_field_num, size_bytes)>
    type Layout = Vec<(u8, usize)>;
    type DevLayout = Vec<(u8, u8, usize)>;
    let mut definitions: HashMap<u8, (u16, Layout, DevLayout, bool)> = HashMap::new();

    // (dev_data_index, field_def_number) → (name, base_type_id)
    let mut field_defs: HashMap<(u8, u8), (String, u8)> = HashMap::new();

    let mut store: DevFieldStore = HashMap::new();

    while pos < end {
        if pos >= data.len() {
            break;
        }
        let header = data[pos];
        pos += 1;

        // Compressed timestamp header — skip (rare, used for dense data)
        if header & 0x80 != 0 {
            continue;
        }

        let local_type  = header & 0x0F;
        let is_def      = header & 0x40 != 0;
        let has_dev     = header & 0x20 != 0;

        if is_def {
            // ── Definition message ────────────────────────────────────────
            if pos + 5 > end {
                break;
            }
            pos += 1; // reserved byte
            let big_endian = data[pos] != 0;
            pos += 1;

            let global = read_u16(&data[pos..], big_endian);
            pos += 2;

            let num_fields = data[pos] as usize;
            pos += 1;

            let mut std_fields: Layout = Vec::with_capacity(num_fields);
            for _ in 0..num_fields {
                if pos + 3 > end {
                    break;
                }
                let fnum  = data[pos];
                let fsize = data[pos + 1] as usize;
                pos += 3; // field_def_num, size, base_type
                std_fields.push((fnum, fsize));
            }

            let mut dev_refs: DevLayout = Vec::new();
            if has_dev && pos < end {
                let num_dev = data[pos] as usize;
                pos += 1;
                for _ in 0..num_dev {
                    if pos + 3 > end {
                        break;
                    }
                    let fnum  = data[pos];
                    let fsize = data[pos + 1] as usize;
                    let didx  = data[pos + 2];
                    pos += 3;
                    dev_refs.push((didx, fnum, fsize));
                }
            }

            definitions.insert(local_type, (global, std_fields, dev_refs, big_endian));
        } else {
            // ── Data message ──────────────────────────────────────────────
            let Some((global, std_fields, dev_refs, big_endian)) =
                definitions.get(&local_type)
            else {
                // No definition for this local type — file is malformed; stop.
                break;
            };
            let global     = *global;
            let big_endian = *big_endian;
            let std_fields = std_fields.clone();
            let dev_refs   = dev_refs.clone();

            // Read standard field bytes, keyed by field definition number.
            let mut std_bytes: HashMap<u8, &[u8]> = HashMap::new();
            for (fnum, fsize) in &std_fields {
                if pos + fsize > end {
                    break;
                }
                std_bytes.insert(*fnum, &data[pos..pos + fsize]);
                pos += fsize;
            }

            // Read developer field bytes, keyed by (dev_data_index, field_num).
            let mut dev_bytes: HashMap<(u8, u8), &[u8]> = HashMap::new();
            for (didx, fnum, fsize) in &dev_refs {
                if pos + fsize > end {
                    break;
                }
                dev_bytes.insert((*didx, *fnum), &data[pos..pos + fsize]);
                pos += fsize;
            }

            // ── FieldDescription (MesgNum 206) ────────────────────────────
            // Registers the name and type of a developer field.
            // Field numbers: 0=dev_data_index, 1=field_def_num, 2=base_type, 3=name
            if global == 206 {
                let dev_idx  = std_bytes.get(&0).and_then(|b| b.first()).copied().unwrap_or(0);
                let fdef_num = std_bytes.get(&1).and_then(|b| b.first()).copied().unwrap_or(0);
                let base_typ = std_bytes.get(&2).and_then(|b| b.first()).copied().unwrap_or(0);

                if let Some(name_bytes) = std_bytes.get(&3) {
                    let name: String = name_bytes
                        .iter()
                        .take_while(|&&b| b != 0)
                        .map(|&b| b as char)
                        .collect();
                    if !name.is_empty() {
                        field_defs.insert((dev_idx, fdef_num), (name, base_typ));
                    }
                }
                continue;
            }

            // ── Record (MesgNum 20) — standard + developer fields ─────────
            if global != 20 {
                continue;
            }

            // Timestamp is standard field 253 (4-byte uint32, FIT epoch).
            let Some(ts_raw) = std_bytes.get(&253).and_then(|b| {
                if b.len() >= 4 {
                    Some(read_u32(b, big_endian))
                } else {
                    None
                }
            }) else {
                continue;
            };

            // Convert FIT epoch → UNIX epoch to match what fitparser returns.
            let ts_unix = ts_raw.saturating_add(FIT_EPOCH_OFFSET);

            let mut record: DevRecord = HashMap::new();

            for ((didx, fnum), bytes) in &dev_bytes {
                let Some((name, base_type)) = field_defs.get(&(*didx, *fnum)) else {
                    continue;
                };

                let value = decode_value(bytes, *base_type, big_endian);
                if let Some(v) = value {
                    record.insert(name.clone(), v);
                }
            }

            // ── Coros-repurposed standard FIT fields ───────────────────────
            // Coros PACE Pro stores proprietary stride metrics in standard FIT
            // field numbers rather than as developer fields.  We extract them
            // here so that decode_record in parser.rs can use the same
            // get_dev() path for all Coros channels.
            //
            // Field 83 → stride_height (mm).  Raw uint16 ÷ 10 = mm.
            // Field 85 → stride_length (mm).  Raw uint16 ÷ 10 = mm.
            if let Some(bytes) = std_bytes.get(&83) {
                if bytes.len() >= 2 {
                    let raw = read_u16(bytes, big_endian) as f64;
                    if raw > 0.0 {
                        record.insert("stride_height".to_string(), raw / 10.0);
                    }
                }
            }
            if let Some(bytes) = std_bytes.get(&85) {
                if bytes.len() >= 2 {
                    let raw = read_u16(bytes, big_endian) as f64;
                    if raw > 0.0 {
                        record.insert("stride_length".to_string(), raw / 10.0);
                    }
                }
            }

            if !record.is_empty() {
                store.insert(ts_unix, record);
            }
        }
    }

    store
}

// ── Binary decoding helpers ───────────────────────────────────────────────────

fn read_u16(bytes: &[u8], big_endian: bool) -> u16 {
    if bytes.len() < 2 {
        return 0;
    }
    if big_endian {
        u16::from_be_bytes([bytes[0], bytes[1]])
    } else {
        u16::from_le_bytes([bytes[0], bytes[1]])
    }
}

fn read_u32(bytes: &[u8], big_endian: bool) -> u32 {
    if bytes.len() < 4 {
        return 0;
    }
    if big_endian {
        u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    } else {
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }
}

/// Decode a raw byte slice to f64 given the FIT base type identifier.
/// Returns None for types we don't handle (strings, arrays, etc.).
fn decode_value(bytes: &[u8], base_type: u8, big_endian: bool) -> Option<f64> {
    match (base_type, bytes.len()) {
        // uint8 variants (enum, uint8, uint8z, byte)
        (0x00 | 0x02 | 0x0A | 0x0B, 1) => Some(bytes[0] as f64),
        // sint8
        (0x01, 1) => Some(bytes[0] as i8 as f64),
        // uint16 / uint16z
        (0x84 | 0x8C, 2) => Some(read_u16(bytes, big_endian) as f64),
        // sint16
        (0x83, 2) => {
            let v = if big_endian {
                i16::from_be_bytes([bytes[0], bytes[1]])
            } else {
                i16::from_le_bytes([bytes[0], bytes[1]])
            };
            Some(v as f64)
        }
        // float32  ← Leg Spring Stiffness, Effort Pace use this
        (0x88, 4) => {
            let f = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            Some(f as f64)
        }
        // uint32 / uint32z
        (0x86 | 0x8D, 4) => Some(read_u32(bytes, big_endian) as f64),
        // sint32
        (0x85, 4) => {
            let v = if big_endian {
                i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
            } else {
                i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
            };
            Some(v as f64)
        }
        // float64
        (0x89, 8) => {
            let f = f64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3],
                bytes[4], bytes[5], bytes[6], bytes[7],
            ]);
            Some(f)
        }
        // Anything else (strings, unknown types) — skip
        _ => None,
    }
}
