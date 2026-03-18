// smoothing/mod.rs — the Smoother trait and its implementations.
//
// The Smoother trait is the key abstraction: any algorithm that takes a slice
// of f64 values and returns a smoothed Vec<f64> of the same length qualifies.
// This lets callers swap algorithms without changing any other code.

pub mod moving_avg;
pub mod exponential;
// pub mod savitzky_golay;  // uncomment when you're ready for step 5

pub use moving_avg::MovingAverage;
pub use exponential::ExponentialMA;

/// The core smoothing abstraction.
///
/// Implementations must return a `Vec<f64>` of the **same length** as the input.
/// This contract makes it easy to zip smoothed values back against timestamps.
pub trait Smoother {
    fn smooth(&self, data: &[f64]) -> Vec<f64>;
}
