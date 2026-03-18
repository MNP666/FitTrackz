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

    // GPS
    pub latitude:  Option<f64>,  // degrees  (converted from FIT semicircles)
    pub longitude: Option<f64>,  // degrees

    // Movement
    pub altitude:  Option<f64>,  // meters
    pub speed:     Option<f64>,  // m/s
    pub distance:  Option<f64>,  // cumulative meters

    // Physiology
    pub heart_rate: Option<u8>,  // bpm
    pub cadence:    Option<u8>,  // rpm or steps/min depending on sport
    pub power:      Option<u32>, // watts
}

impl FitRecord {
    /// Convenience: extract a named field as f64 so smoothers can work
    /// on any channel without pattern matching at the call site.
    pub fn get_field(&self, name: &str) -> Option<f64> {
        match name {
            "altitude"   => self.altitude,
            "speed"      => self.speed,
            "distance"   => self.distance,
            "heart_rate" => self.heart_rate.map(|v| v as f64),
            "cadence"    => self.cadence.map(|v| v as f64),
            "power"      => self.power.map(|v| v as f64),
            "latitude"   => self.latitude,
            "longitude"  => self.longitude,
            _            => None,
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
    /// Extract all values for a given channel, keeping only records
    /// where that field is present.  Returns (index, value) pairs so
    /// callers can still correlate back to timestamps.
    pub fn extract_channel(&self, name: &str) -> Vec<(usize, f64)> {
        self.records
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.get_field(name).map(|v| (i, v)))
            .collect()
    }
}
