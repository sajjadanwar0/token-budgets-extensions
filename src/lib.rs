pub mod adaptive;

pub use adaptive::{AdaptiveEstimator, ModelKey};

/// Local trait alias for token-budgets-extensions consumers who want
/// a one-method trait without depending on the parent crate's
/// estimator module directly. The parent crate's `TokenEstimator` is
/// re-exported by the adaptive estimator and is the canonical trait;
/// keep this here only if downstream code already imports it.
pub trait TokenEstimator {
    fn estimate(&self, prompt: &str) -> u64;
}
