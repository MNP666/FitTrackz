// exponential.rs — Exponential Moving Average (EMA).
//
// Each output point is a weighted blend of the current input and the previous
// output:  y[i] = alpha * x[i] + (1 - alpha) * y[i-1]
//
// alpha close to 1 → very little smoothing (tracks the signal tightly)
// alpha close to 0 → heavy smoothing (slow to respond to changes)
//
// A good starting point for GPS/HR data: alpha = 0.1–0.3.

use super::Smoother;

pub struct ExponentialMA {
    /// Smoothing factor in the range (0, 1].
    pub alpha: f64,
}

impl ExponentialMA {
    pub fn new(alpha: f64) -> Self {
        assert!(alpha > 0.0 && alpha <= 1.0, "alpha must be in (0, 1]");
        Self { alpha }
    }
}

impl Smoother for ExponentialMA {
    fn smooth(&self, data: &[f64]) -> Vec<f64> {
        if data.is_empty() {
            return Vec::new();
        }

        let mut out = Vec::with_capacity(data.len());
        // Initialise with the first data point so there's no lag at the start.
        out.push(data[0]);

        for &x in &data[1..] {
            let prev = *out.last().unwrap();
            out.push(self.alpha * x + (1.0 - self.alpha) * prev);
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_signal_is_unchanged() {
        let data = vec![10.0; 20];
        let smoothed = ExponentialMA::new(0.2).smooth(&data);
        for v in smoothed {
            assert!((v - 10.0).abs() < 1e-9);
        }
    }

    #[test]
    fn output_length_matches_input() {
        let data: Vec<f64> = (0..30).map(|x| x as f64).collect();
        let smoothed = ExponentialMA::new(0.3).smooth(&data);
        assert_eq!(smoothed.len(), data.len());
    }

    #[test]
    fn alpha_one_is_identity() {
        // alpha = 1.0 means y[i] = x[i], no smoothing at all.
        let data = vec![1.0, 5.0, 3.0, 8.0, 2.0];
        let smoothed = ExponentialMA::new(1.0).smooth(&data);
        for (s, d) in smoothed.iter().zip(data.iter()) {
            assert!((s - d).abs() < 1e-9);
        }
    }
}
