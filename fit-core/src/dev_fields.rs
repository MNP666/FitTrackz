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

// ── Main entry point ──────────────────────────────────────────────────────────

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
