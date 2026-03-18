// fit-core: the heart of FitTrackz.
// This crate is pure Rust with no Python dependency — all the real logic lives here.
// fit-cli uses it for the command line, and eventually fit-py will wrap it for Python.

pub mod models;
pub mod parser;
pub mod smoothing;

// Re-export the most useful types so callers can write `fit_core::FitActivity`
// instead of `fit_core::models::FitActivity`.
pub use models::{FitActivity, FitRecord};
pub use parser::parse_fit_file;
