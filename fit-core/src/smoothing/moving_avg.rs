// moving_avg.rs — simple centred moving average.
//
// For a window of size W, each output point is the mean of the W nearest
// input points. Points near the edges use a smaller window so the output
// length always equals the input length.

use super::Smoother;

pub struct MovingAverage {
    /// Number of points in the averaging window. Should be odd for a
    /// centred window (e.g. 5 means 2 points on each side + centre).
    pub window: usize,
}

impl MovingAverage {
    pub fn new(window: usize) -> Self {
        assert!(window >= 1, "window must be at least 1");
        Self { window }
    }
}

impl Smoother for MovingAverage {
    fn smooth(&self, data: &[f64]) -> Vec<f64> {
        let n = data.len();
        let half = self.window / 2;

        (0..n)
            .map(|i| {
                // Clamp window to valid index range around i.
                let lo = i.saturating_sub(half);
                let hi = (i + half + 1).min(n);
                let slice = &data[lo..hi];
                slice.iter().sum::<f64>() / slice.len() as f64
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_signal_is_unchanged() {
        let data = vec![5.0; 10];
        let smoothed = MovingAverage::new(5).smooth(&data);
        for v in smoothed {
            assert!((v - 5.0).abs() < 1e-9);
        }
    }

    #[test]
    fn output_length_matches_input() {
        let data: Vec<f64> = (0..17).map(|x| x as f64).collect();
        let smoothed = MovingAverage::new(5).smooth(&data);
        assert_eq!(smoothed.len(), data.len());
    }
}
