pub mod adaptive;
pub mod streaming;

pub use adaptive::{AdaptiveEstimator, ModelKey};
pub use streaming::{InterimRefundCounter, StreamingReceipt};

pub trait TokenEstimator {
    fn estimate(&self, prompt: &str) -> u64;
}