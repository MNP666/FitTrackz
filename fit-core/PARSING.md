# FIT Binary Parsing — Technical Notes

This document explains how the FIT binary format works and how `dev_fields.rs` parses it. It is written as a learning reference: the FIT spec is well-designed but has several non-obvious aspects that caused real bugs during development.

---

## What is a FIT file?

FIT (Flexible and Interoperable Data Transfer) is a binary format developed by Garmin for sports device data. The spec is open but the format is compact and entirely binary — no text, no delimiters. Every GPS watch from Garmin, Coros, Polar, Wahoo and others writes the same format.

A `.fit` file is a single flat byte stream structured as follows:

```
[File Header 14 bytes]
[Record 1]
[Record 2]
...
[Record N]
[CRC 2 bytes]
```

---

## File header

The first 14 bytes are the file header:

```
Byte 0:     header size (always 14 for FIT 2.x)
Byte 1:     protocol version
Bytes 2–3:  profile version (little-endian)
Bytes 4–7:  data size in bytes, not counting header or CRC (little-endian)
Bytes 8–11: ".FIT" ASCII magic
Bytes 12–13: CRC of the header
```

The parser reads bytes 0 and 4–7 to know where the data ends.

---

## Record headers

Every record starts with a single header byte. The high bit (bit 7) determines which of two formats it uses.

### Normal record header (bit 7 = 0)

```
Bit 7:    always 0
Bit 6:    1 = definition message,  0 = data message
Bit 5:    1 = has developer fields in definition  (only meaningful for definitions)
Bit 4:    reserved, always 0
Bits 3–0: local message type (0–15)
```

### Compressed timestamp header (bit 7 = 1)

```
Bit 7:    always 1
Bits 6–5: local message type (0–3)
Bits 4–0: 5-bit time offset in seconds
```

Compressed timestamp records save 4 bytes per record by omitting the timestamp field entirely. Instead, they encode a 5-bit offset (0–31) relative to the last seen full timestamp. The receiver replaces the lower 5 bits of the rolling timestamp with this offset. If the result is smaller than the previous timestamp, the 5-bit counter rolled over and 32 is added.

Coros watches use compressed timestamps for almost every record in a run, with a full timestamp only every 30–60 seconds. **Failing to handle compressed timestamps causes the parser to stop after the first handful of records.** This was the hardest bug to diagnose in this project.

---

## Definition messages

Before any data can be read, the device sends a definition message for each local message type it will use. The definition describes what fields will appear in subsequent data messages for that local type:

```
Byte 0:  reserved (always 0)
Byte 1:  architecture: 0 = little-endian, 1 = big-endian
Bytes 2–3: global message number (identifies the message type)
Byte 4:  number of standard fields (N)
Then N × 3 bytes, each:
    Byte 0: field definition number
    Byte 1: field size in bytes
    Byte 2: base type identifier
If the header had bit 5 set (has_dev):
    Byte 0: number of developer fields (M)
    Then M × 3 bytes, each:
        Byte 0: developer field definition number
        Byte 1: field size in bytes
        Byte 2: developer data index
```

The local message type (0–15) is just a local label. The global message number is what identifies the message semantics — for example, global 20 is always a Record (one second of sensor data), regardless of which local type the device assigned to it.

The parser stores each definition in a `HashMap<u8, (u16, StdLayout, DevLayout, bool)>` keyed by local type. When a data message arrives with a given local type, the stored definition tells us exactly how many bytes to read and how to interpret them.

---

## Data messages

A data message has no header beyond the record header byte. The payload is simply the fields laid out in the same order and sizes as defined in the most recent definition message for that local type. There is no per-field tag or length in the payload — you must already know the layout from the definition.

For compressed timestamp data messages, the payload is identical to a normal data message of the same local type, **except that field 253 (timestamp) is omitted from the payload** because the timestamp is computed from the header offset instead.

---

## Global message numbers we care about

| Global | Name | Purpose |
|--------|------|---------|
| 20 | Record | One second of sensor data (HR, GPS, speed, cadence, …) |
| 12 | Sport | Activity type (running, cycling, …) |
| 206 | FieldDescription | Registers a developer field name and type |
| 207 | DeveloperDataId | Registers a developer data source (we ignore this) |

---

## Developer fields

The FIT format lets device manufacturers embed proprietary metrics that are not in the standard profile. Coros and Stryd both use this to add running power, leg spring stiffness, and other fields.

The mechanism has two parts:

**FieldDescription (global 206)** announces one developer field. The key fields inside it are:

| Field num | Name | Meaning |
|-----------|------|---------|
| 0 | dev_data_index | Which developer registered this field |
| 1 | field_def_number | The field's ID number within that developer's namespace |
| 2 | fit_base_type_id | The numeric type (uint8, uint16, float32, …) |
| 3 | field_name | Human-readable name as a null-terminated ASCII string |
| 6 | units | Unit string (e.g. "Watts", "KN/m") |

**Every Record (global 20)** that carries developer fields has them appended after the standard fields, with the layout described in the dev_refs section of the definition message.

The developer field values in a Coros file are **already scaled**. Form Power arrives as `71.0` (Watts), not as a raw integer. No scale/offset step needed for developer fields.

### Coros PACE Pro developer fields

Confirmed by binary inspection of a real long_run.fit:

| Field name | Type | Unit |
|------------|------|------|
| Form Power | uint16 | W |
| Leg Spring Stiffness | float32 | kN/m |
| Air Power | uint16 | W |
| Impact Loading Rate | uint16 | BW/s |
| Effort Pace | float32 | m/s |

---

## Standard field scale and offset

Standard FIT fields store sensor values as integers scaled by a constant. The physical value is:

```
physical = raw_integer / scale + offset
```

For the fields used in Record messages (global 20):

| Field num | Name | Base type | Scale | Offset | Unit |
|-----------|------|-----------|-------|--------|------|
| 0 | position_lat | sint32 | — | — | semicircles (× 180/2³¹ = degrees) |
| 1 | position_long | sint32 | — | — | semicircles |
| 2 | altitude | uint16 | 5 | −500 | m |
| 3 | heart_rate | uint8 | 1 | 0 | bpm |
| 4 | cadence | uint8 | 1 | 0 | strides/min (one foot) |
| 5 | distance | uint32 | 100 | 0 | m |
| 6 | speed | uint16 | 1000 | 0 | m/s |
| 7 | power | uint16 | 1 | 0 | W |
| 39 | vertical_oscillation | uint16 | 10 | 0 | mm |
| 41 | stance_time | uint16 | 10 | 0 | ms |
| 78 | enhanced_altitude | uint32 | 5 | −500 | m |
| 83 | *Coros stride_height* | uint16 | 10 | 0 | mm |
| 85 | *Coros stride_length* | uint16 | 10 | 0 | mm |
| 136 | enhanced_speed | uint32 | 1000 | 0 | m/s |
| 253 | timestamp | uint32 | 1 | 0 | FIT epoch seconds |

Fields 83 and 85 are officially named `motor_power` and an unnamed field in the standard FIT profile, but Coros repurposes them for stride metrics. The field scan tool (`cargo run --bin fit-cli -- file.fit scan`) was used to identify them — their value ranges (raw 560–660 → 56–66 mm for height; raw 9600–11000 → 960–1100 mm for length) match known stride characteristics.

---

## FIT invalid sentinels

Every base type reserves its maximum unsigned value as an "invalid" sentinel meaning "no data":

| Base type | Invalid value |
|-----------|---------------|
| uint8 | 255 (0xFF) |
| sint8 | 127 (0x7F) |
| uint16 | 65535 (0xFFFF) |
| sint16 | 32767 (0x7FFF) |
| uint32 | 4294967295 (0xFFFFFFFF) |
| sint32 | 2147483647 (0x7FFFFFFF) |

The parser's `decode_standard_field` function checks for these before applying scale/offset and returns `None` if matched. This is why, for example, altitude returns `None` at the start of the run before GPS locks — the watch writes 0xFFFF into the altitude field, not 0.

---

## Why no third-party FIT parser?

We started with the `fitparser 0.6` crate. It was removed for two reasons:

1. **Developer fields are silently dropped.** The `fields()` API on a decoded message does not include developer fields. There is no alternative API. This meant all the Coros running dynamics (form power, leg spring stiffness, etc.) were invisible.

2. **Field naming is opaque.** For the two Coros-repurposed fields (83 and 85), fitparser assigns its own internal names (`motor_power`, `unknown_85`) based on an old version of the profile. Matching by name is fragile.

The solution was to write a binary parser from scratch. The parser is not complex — the FIT format is well-specified — and it gives full control over every byte.

---

## How the parser is structured

```
parse_fit_activity_from_bytes(data: &[u8]) -> FitActivity
    │
    ├── Parse file header → get data_end position
    ├── Main loop over records:
    │   ├── Read record header byte
    │   ├── If compressed timestamp (bit 7 = 1):
    │   │   ├── Extract local_type from bits 6–5
    │   │   ├── Extract 5-bit time offset from bits 4–0
    │   │   ├── Compute new timestamp from rolling_ts + offset
    │   │   ├── Look up definition for local_type
    │   │   ├── Read data bytes (no field 253 in payload)
    │   │   └── If global == 20: build_fit_record(computed_ts, ...)
    │   ├── If definition message (bit 6 = 1):
    │   │   └── Parse and store layout in definitions HashMap
    │   └── If normal data message (bit 6 = 0):
    │       ├── Look up definition for local_type
    │       ├── Read standard field bytes into std_bytes HashMap
    │       ├── Read developer field bytes into dev_bytes HashMap
    │       ├── global == 206: register developer field name/type
    │       ├── global == 12:  extract sport string
    │       └── global == 20:  build_fit_record(field_253_ts, ...)
    │
    └── Return FitActivity { sport, records }

build_fit_record(ts, std_bytes, dev_bytes, field_defs, big_endian) -> Option<FitRecord>
    ├── For each std field: decode_standard_field(bytes, field_num, base_type)
    │   ├── decode_value (bytes → raw f64 by base type)
    │   ├── Filter invalid sentinels (0xFF, 0xFFFF, 0xFFFFFFFF, ...)
    │   └── Apply scale/offset (e.g. speed: raw / 1000.0)
    ├── For each dev field: look up name in field_defs, decode_value
    └── Return FitRecord with all fields set or None
```

The rolling timestamp (`rolling_ts_fit`) is maintained in FIT epoch seconds (not UNIX). The UNIX timestamp used in `FitRecord` is computed by adding `FIT_EPOCH_OFFSET = 631_065_600` (the number of seconds between 1989-12-31 and 1970-01-01).

---

## Diagnostic tools

**`scan`** — reads the whole file and reports statistics for every field number found in Record messages: record count, min, max, and the first 5 raw values. Use this when a channel returns no data:

```bash
cargo run --bin fit-cli -- activity.fit scan
```

Look for columns whose value range matches what you expect:
- stride_length ≈ 9000–11000 raw (÷10 = 900–1100 mm)
- stride_height ≈ 500–700 raw (÷10 = 50–70 mm)

**`dump`** — decodes a representative mid-run record and prints every channel with its physical value and unit. The function skips the start of the run to avoid pre-GPS and pre-developer-field records.
