pub mod adaptive;

pub use adaptive::{AdaptiveEstimator, ModelKey};

pub trait TokenEstimator {
    fn estimate(&self, prompt: &str) -> u64;
}