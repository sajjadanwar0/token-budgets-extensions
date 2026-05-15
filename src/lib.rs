//! budget-typed-cap-extensions
//!
//! Optional extension modules for `budget-typed-cap`:
//!
//! - [`adaptive`]: adaptive token estimator with sound margin
//!   tightening (closes part of the 6.4x over-reservation gap
//!   for stable per-(model,tokenizer) workloads).
//! - [`streaming`]: streaming receipt with interim refund
//!   counter (closes the 6.4x gap for long streaming responses).
//!
//! Both modules preserve the affine discipline of the base crate.
//! See module-level documentation for soundness conditions.

pub mod adaptive;
pub mod streaming;

/// Re-exports for convenience.
pub use adaptive::{AdaptiveEstimator, ModelKey};
pub use streaming::{InterimRefundCounter, StreamingReceipt};

// We mirror the base crate's TokenEstimator trait here so the
// extensions can compile standalone. In a real integration this
// would be `use token_budgets::TokenEstimator`.
pub trait TokenEstimator {
    fn estimate(&self, prompt: &str) -> u64;
}
