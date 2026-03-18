// models.rs — the data types that represent a parsed FIT activity.
//
// Why Option<T> everywhere?
//   FIT files are sparse: a run won't have power, an indoor ride won't have GPS,
//   a record might be missing altitude due to a dropout. Option<T> forces you to
//   handle "this field might not exist" at compile time — no silent NaN surprises.

/// A single data point in an activity (one "record" message in the FIT protocol).
/// Timestamps are raw FIT epoch seconds (seconds since 1989-12-31 00:00:00 UTC).
#[derive(Debug, Clone)]
pub struct FitRecord {
    pub timestamp: u32,

    // ── GPS ────────────────────────────────────────────────────────────────────
    pub latitude:  Option<f64>,  // degrees  (converted from FIT semicircles)
    pub longitude: Option<f64>,  // degrees

    // ── Movement ───────────────────────────────────────────────────────────────
    pub altitude:  Option<f64>,  // metres
    pub speed:     Option<f64>,  // m/s   ← used as the threshold channel for red-light filtering
    pub distance:  Option<f64>,  // cumulative metres

    // ── Physiology ─────────────────────────────────────────────────────────────
    pub heart_rate: Option<u8>,  // bpm
    pub cadence:    Option<u8>,  // rpm or steps/min depending on sport
    pub power:      Option<u32>, // watts

    // ── Standard running dynamics (FIT SDK fields, present on most GPS watches) ─
    /// Vertical displacement of the centre of mass per stride, in mm.
    /// FIT field 39.  Raw uint16 ÷ 10 = mm.
    pub vertical_oscillation: Option<f64>,  // mm

    /// Ground contact time per step, in ms.
    /// FIT field 41.  Raw uint16 ÷ 10 = ms.
    pub stance_time: Option<f64>,  // ms

    // ── Coros running dynamics ─────────────────────────────────────────────────
    // Coros repurposes two standard FIT field numbers for their own running
    // metrics.  The raw integer values need ÷10 to get the real unit.
    //
    // If these come back as None, run `cargo run --bin fit-cli -- my.fit scan`
    // to see all field numbers present in the binary and their value ranges.
    // Identify the field whose values match expected stride sizes, then update
    // the field numbers in decode_standard_field() in dev_fields.rs.

    /// Coros proprietary stride height (mm). Stored in FIT field 83.
    /// Raw integer ÷ 10 = mm.  Very similar to vertical_oscillation but
    /// computed differently by Coros firmware.
    pub stride_height: Option<f64>,  // mm

    /// Coros stride length per step (mm). Stored in FIT field 85.
    /// Raw integer ÷ 10 = mm.  Typical running values: 900–1300 mm.
    pub stride_length: Option<f64>,  // mm

    // ── Coros developer fields (named in the file via FieldDescription messages) ─
    /// Running power attributed to maintaining form, in watts.
    pub form_power: Option<f64>,  // W

    /// Leg spring stiffness, in KN/m.  Higher = stiffer leg spring.
    pub leg_spring_stiffness: Option<f64>,  // KN/m

    /// Power cost of moving through air, in watts.
    pub air_power: Option<f64>,  // W

    /// Rate of impact force at foot strike, in body-weights per second (BW/s).
    pub impact_loading_rate: Option<f64>,  // BW/s
}

impl FitRecord {
    /// Convenience: extract a named field as f64 so smoothers can work
    /// on any channel without pattern matching at the call site.
    pub fn get_field(&self, name: &str) -> Option<f64> {
        match name {
            // Movement
            "altitude"   => self.altitude,
            "speed"      => self.speed,
            "distance"   => self.distance,
            // Physiology
            "heart_rate" => self.heart_rate.map(|v| v as f64),
            "cadence"    => self.cadence.map(|v| v as f64),
            "power"      => self.power.map(|v| v as f64),
            // GPS
            "latitude"   => self.latitude,
            "longitude"  => self.longitude,
            // Standard running dynamics
            "vertical_oscillation" => self.vertical_oscillation,
            "stance_time"          => self.stance_time,
            // Coros running dynamics
            "stride_height" => self.stride_height,
            "stride_length" => self.stride_length,
            // Coros developer fields
            "form_power"           => self.form_power,
            "leg_spring_stiffness" => self.leg_spring_stiffness,
            "air_power"            => self.air_power,
            "impact_loading_rate"  => self.impact_loading_rate,
            _                      => None,
        }
    }
}

/// The top-level container for one parsed FIT file.
#[derive(Debug)]
pub struct FitActivity {
    pub sport:   Option<String>,
    pub records: Vec<FitRecord>,
}

impl FitActivity {
    /// Extract all values for a given channel, keeping only records where that
    /// field is present.  Returns (index, value) pairs so callers can correlate
    /// back to timestamps.
    pub fn extract_channel(&self, name: &str) -> Vec<(usize, f64)> {
        self.records
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.get_field(name).map(|v| (i, v)))
            .collect()
    }

    /// Same as `extract_channel`, but skips records where speed is below
    /// `min_speed_ms` (metres per second).
    ///
    /// Use this to drop red-light / standing-still records before smoothing.
    /// The threshold cleanly separates standing (≈0 m/s) from running (>2 m/s),
    /// so a value around 1.0 m/s works well in practice.
    ///
    /// Records with no speed field at all are also excluded, since we cannot
    /// know whether the watch was moving.
    ///
    /// # Example
    /// ```ignore
    /// // Only include records where the runner is actually moving
    /// let data = activity.extract_channel_filtered("heart_rate", 1.0);
    /// ```
    pub fn extract_channel_filtered(
        &self,
        name: &str,
        min_speed_ms: f64,
    ) -> Vec<(usize, f64)> {
        self.records
            .iter()
            .enumerate()
            // Gate: only keep records where speed is known AND above the threshold
            .filter(|(_, r)| {
                r.speed
                    .map(|s| s >= min_speed_ms)
                    .unwrap_or(false)
            })
            // Then extract the requested channel the same way as extract_channel
            .filter_map(|(i, r)| r.get_field(name).map(|v| (i, v)))
            .collect()
    }
}
