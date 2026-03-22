# Parsing FIT Metadata from Coros Devices: What Went Wrong and What Finally Worked

This documents the debugging journey of extracting activity-level metadata (manufacturer, device name, session stats) from `.fit` files produced by a **Coros Pace Pro**. The goal was to implement a lightweight `fit-cli <file> metadata` subcommand in Rust that outputs a JSON object without parsing every per-second record.

The solution is now in `fit-core/src/dev_fields.rs` (`parse_fit_metadata_from_bytes`). This write-up exists so you don't have to retrace the same steps.

---

## Background: The FIT File Format

A `.fit` file is a compact binary format. It begins with a 14-byte file header, followed by a stream of **messages**. Each message is either a **definition** message or a **data** message.

**Definition messages** describe the layout of the data records that follow: which fields are present, how wide each field is in bytes, and what base type it is (uint8, uint16, uint32, string, etc.).
**Data messages** carry the actual values, in exactly the layout defined by the most recent definition for that *local message type*.

The key concepts:

- **Local message type** (4 bits, 0–15): a short-hand ID used within the file to map a data record to its definition. Up to 16 definitions can be "active" at once.
- **Global message number**: the canonical identity of a message type in the FIT SDK. `file_id` = 0, `session` = 18, `record` = 20, `device_info` = 23, etc.
- **FIT epoch**: FIT timestamps are seconds since **1989-12-31 00:00:00 UTC**, not the Unix epoch. Add `631_065_600` to get Unix seconds.
- **Invalid sentinels**: a field that has no data is stored as the maximum value for its type — `0xFF` for uint8, `0xFFFF` for uint16, `0xFFFFFFFF` for uint32. These must be filtered before use.
- **Record header byte**: the single byte preceding each message encodes whether it is a definition or data message, its local type, and whether it uses the compressed-timestamp format.

A typical activity file looks like this (in order):

```
[file header]
[definition: local=0, global=0 (file_id)]
[data: file_id] ← manufacturer, product, serial, time_created
[definition: local=4, global=23 (device_info)]
[data: device_info] ← firmware version
... (other setup messages) ...
[definition: local=9, global=20 (record)]
[data: record] × thousands  ← per-second GPS/HR/cadence/power
... (Coros proprietary messages) ...
[definition: local=2, global=18 (session)]
[data: session] ← session totals, avg/max stats
[optional lap messages]
[2-byte CRC]
```

This structure is important: **file_id and device_info are near the very beginning; session is near the very end.**

---

## Attempt 1: Standard Forward Scan — Breaks at Byte 6,454

The initial implementation was a straightforward single-pass forward scan: read each message header, look up (or store) its definition, and decode the fields of interest (globals 0, 18, 23).

For every other global message number it would just advance `pos` past the payload and continue. This works perfectly on Garmin files.

On the Coros Pace Pro file (533 KB for a 30 km long run), the parser broke at **byte 6,454** — only 1.2% of the way through the file — with:

```
data record for local_type 5, but no definition found for local_type 5
```

The parser had `break` for this case (safe choice: without a definition, the record size is unknown, so you can't advance `pos` correctly). The session data at byte 533,074 was never reached.

**Why does this happen?** Coros writes proprietary messages in the middle of the file — between the record data and the session message. These messages use local types that were never preceded by a standard definition the parser can understand. This is technically spec-compliant: the FIT specification only requires that a definition appear before the *corresponding* data records. A conformant reader is expected to skip unknown messages, but to do that it must already know their size — which requires having seen the definition. It is a catch-22.

---

## Attempt 2: `continue` Instead of `break` — Still Broken

The fix seemed obvious: instead of `break` on an unknown local_type, use `continue`. We've already advanced `pos` past the 1-byte header, so the next iteration reads the following byte as a new header. Eventually we'll resync on the session definition.

This almost works, but has a fatal flaw.

When we scan byte-by-byte through an unknown record's payload, **some of those bytes will accidentally look like definition message headers** (bit 6 set, bit 7 clear — roughly 25% of all byte values). When we encounter one of these, the parser tries to parse a definition starting there: it reads a `reserved` byte, an `endian` byte, two bytes for the global message number, and one byte for `num_fields`. If `num_fields` happens to be large (say, 200), the parser then advances `pos` by `200 × 3 = 600` bytes to "read" the field descriptors — skipping right over real data.

In the worst case, one of these garbage "definitions" could store a corrupted definition for `local_type 2`, overwriting the slot that the real session definition will later occupy. More likely, it advances `pos` far enough to skip the actual session definition at byte 533,074.

The result: still all `null`.

---

## Root Cause: The Session Is at the End

Binary inspection of the actual file confirmed the positions:

| Message | Byte offset |
|---|---|
| `file_id` definition + data | ~14 |
| `device_info` definition + data | ~95 |
| First `record` definition | ~300 |
| 10,000+ record data messages | 300 – 532,000 |
| Coros proprietary messages | ~6,400 – ~533,000 |
| `session` definition + data | **533,074** |
| File CRC | 533,213 |

The file is 533,214 bytes. The session is 140 bytes from the end.

No forward-scan approach that can be confused by arbitrary bytes in the middle of the file is going to reliably reach byte 533,074. A different strategy is needed.

---

## What Actually Worked: A Two-Pass Approach

The key insight: **file_id and device_info are at the start; session is at the end.** We never needed a single pass.

### Pass 1 — Forward scan for file_id and device_info

A normal forward scan, but with two improvements:

1. **Early exit** as soon as `manufacturer`, `product_name`, and `time_created` are all populated — there's no point scanning further.
2. **`break` on unknown local_type** is *fine* here because both messages appear in the first ~100 bytes, before any Coros proprietary messages appear.

The sanity guard `if num_fields > 64 { break; }` prevents a single garbage "definition" with a huge field count from skipping hundreds of bytes.

### Pass 2 — Backwards pattern search for the session

Instead of scanning forward and hoping to maintain byte-sync, scan **backwards** through the raw bytes looking for a byte sequence that unambiguously identifies a session definition message:

```
byte 0:  0x40–0x4F or 0x60–0x6F   (bit7=0, bit6=1 — is a definition)
byte 1:  0x00                       (reserved byte, always zero in valid FIT)
byte 2:  0x00 or 0x01               (endianness: 0=LE, 1=BE)
byte 3:  0x12                       (lo byte of global message number 18)
byte 4:  0x00                       (hi byte)
byte 5:  1–50                       (num_fields sanity check)
```

The probability of this 6-byte pattern appearing at random is approximately 10⁻⁹ per position. In a 500 KB file, the expected number of false positives is essentially zero. Scanning backwards ensures we find the *last* session definition — the one that immediately precedes the actual session data record.

Once the definition is located, parsing the data record that follows it is straightforward: the record must be at `def_pos + 6 + (num_fields × 3) + dev_field_bytes`, and its header byte must be `0x00 | local_type`.

This approach is completely immune to mid-file Coros proprietary messages.

---

## Field Numbers on the Coros Pace Pro

The other half of the debugging was getting the **field numbers** right for the session message. The FIT SDK's `Profile.xlsx` is the authoritative reference, but some numbers had been entered incorrectly in the initial implementation.

The following were verified by decoding the actual binary data from a real Coros Pace Pro file:

### `file_id` (MesgNum 0)

| Field | Number | Type | Notes |
|---|---|---|---|
| type | 0 | enum | 4 = activity |
| manufacturer | 1 | uint16 | 294 = Coros |
| product | 2 | uint16 | 805 = Pace Pro |
| serial_number | 3 | uint32z | — |
| time_created | 4 | uint32 | FIT epoch |
| product_name | 8 | string | "COROS PACE Pro" |

### `device_info` (MesgNum 23) — Coros Pace Pro

The Coros Pace Pro only writes three standard fields in its device_info message:

| Field | Number | Type | Notes |
|---|---|---|---|
| timestamp | 253 | uint32 | FIT epoch |
| manufacturer | 2 | uint16 | 294 = Coros |
| product_name | 27 | string | "COROS PACE Pro" |

Field 5 (`software_version`, firmware) is **not written** by this device. It will be `null`.
Field 4 is `product` (model number), not firmware — an easy mix-up.

### `session` (MesgNum 18) — Coros Pace Pro

| Field | Number | Type | Scale | Decoded value (example) |
|---|---|---|---|---|
| sport | 5 | enum | — | 1 = running |
| start_time | 2 | uint32 | — | FIT epoch |
| timestamp | 253 | uint32 | — | FIT epoch |
| total_elapsed_time | 7 | uint32 | ÷1000 → s | 10151.6 s |
| total_timer_time | 8 | uint32 | ÷1000 → s | 9823.7 s |
| total_distance | 9 | uint32 | ÷100 → m | 30385.5 m |
| total_calories | 11 | uint16 | — | 2330 kcal |
| max_heart_rate | 17 | uint8 | — | 162 bpm |
| avg_heart_rate | 16 | uint8 | — | 143 bpm |
| total_ascent | 22 | uint16 | — | 218 m |
| total_descent | 23 | uint16 | — | 226 m |
| total_cycles | 10 | uint32 | — | 15023 strides |
| max_cadence | 19 | uint8 | — | 96 spm |
| avg_cadence | 18 | uint8 | — | 91 spm |
| max_speed | 15 | uint16 | ÷1000 → m/s | 3.846 m/s |
| avg_speed | 14 | uint16 | ÷1000 → m/s | 3.093 m/s |
| avg_power | 20 | uint16 | — | 252 W |
| training_stress_score | **91** | uint16 | ÷10 | 236.0 |

**Two gotchas:**

1. **TSS uses field 91, not field 35.** The FIT SDK standard puts training_stress_score at field 35 (scale ÷10). Coros uses field 91 with the same encoding. Neither `max_power` (field 21) nor `sub_sport` (field 6) are written by this device.

2. **TSS scale is ÷10, not ÷100.** The FIT SDK Profile.xlsx says `scale=10` for this field, meaning the stored integer is `TSS × 10`. A stored value of `2360` → TSS = 236.0.

---

## Inspecting an Unknown FIT File

If you need to do this kind of investigation yourself, the following Python script decodes the binary directly without depending on any FIT library:

```python
import struct, datetime

FIT_EPOCH = 631_065_600

BASE_TYPES = {
    0x00: ('enum', 1), 0x02: ('uint8', 1), 0x84: ('uint16', 2),
    0x86: ('uint32', 4), 0x07: ('string', 1),
}
INVALID = {1: 0xFF, 2: 0xFFFF, 4: 0xFFFFFFFF}

def find_session_definition(path):
    data = open(path, 'rb').read()
    hdr  = data[0]
    end  = min(hdr + struct.unpack_from('<I', data, 4)[0], len(data))

    for p in range(end - 6, hdr - 1, -1):
        b = data[p]
        if b & 0xC0 != 0x40: continue       # must be definition header
        if data[p+1] != 0x00: continue       # reserved
        if data[p+2] > 1: continue           # endian
        if data[p+3] != 0x12: continue       # global = 18 (lo byte)
        if data[p+4] != 0x00: continue       # global = 18 (hi byte)
        nf = data[p+5]
        if not (1 <= nf <= 50): continue     # sanity
        print(f"Session definition at byte {p}")
        pos = p + 6
        fields = [(data[pos + i*3], data[pos + i*3 + 1]) for i in range(nf)]
        pos += nf * 3
        if data[p] & 0x20:                   # has_dev
            nd = data[pos]; pos += 1 + nd * 3
        pos += 1                             # data record header
        for fn, fs in fields:
            raw = data[pos:pos+fs]; pos += fs
            if fs in (1, 2, 4):
                fmt = {1:'<B', 2:'<H', 4:'<I'}[fs]
                v = struct.unpack_from(fmt, raw)[0]
                if v != INVALID.get(fs):
                    if fn == 253:
                        print(f"  f{fn}: {datetime.datetime.utcfromtimestamp(v+FIT_EPOCH)} UTC")
                    else:
                        print(f"  f{fn}: {v}")
        return

find_session_definition("your_activity.fit")
```

Use this to verify field numbers from any device before hard-coding them in your parser.

---

## Summary

| Approach | Outcome | Why |
|---|---|---|
| Forward scan, `break` on unknown local_type | ❌ All session fields null | Parser stops at byte 6,454; session is at byte 533,074 |
| Forward scan, `continue` on unknown local_type | ❌ All session fields null | Garbage bytes misparse as definition headers, advance `pos` by hundreds of bytes, skip past the session definition |
| **Two-pass: forward scan for start + backwards pattern search for end** | ✅ All fields populated | Forward scan grabs file_id/device_info from first ~100 bytes; pattern search is immune to mid-file noise |

The backwards pattern search is the key technique. It requires no byte-sync maintenance and has a negligible false-positive rate. It is directly applicable to any message type that always appears at the end of a FIT file (session, lap summary, activity).
