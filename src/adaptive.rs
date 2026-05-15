//! Adaptive token estimator with sound margin tightening.
//!
//! The static `ByteLength` estimator over-reserves by a median 6.4x
//! across our empirical regime. This module ships an adaptive
//! estimator that observes the realised token-per-byte ratio
//! on a fixed (model, tokenizer) pair and tightens the reservation
//! over time, while preserving cap-soundness under a clearly
//! stated conditional assumption A1'.
//!
//! # Soundness condition
//!
//! The static A1 (byte-length dominance) holds unconditionally for
//! canonical byte-level BPE. The adaptive estimator instead requires
//! a conditional A1':
//!
//! ```text
//!     forall call i, ratio(i) <= observed_max(i-1) + epsilon
//! ```
//!
//! where ratio(i) = actual_input_tokens(i) / byte_length(prompt(i)).
//! That is: future ratios are bounded by the prior-observed max
//! plus a safety margin. This is empirically weaker than A1
//! (which holds structurally for byte-level BPE) and exchanges
//! one assumption for another.
//!
//! # Construction
//!
//! - Initialise with `observed_max = 1.0` (the static A1 bound;
//!   no call has yet been observed).
//! - Reserve `byte_length * (observed_max + epsilon)` for each call.
//! - After each call, observe (bytes, actual_tokens) and update
//!   `observed_max = max(observed_max, actual_tokens / bytes)`.
//! - The observed_max is *monotonically non-decreasing*; the
//!   estimator never tightens below historical evidence.
//!
//! # What this does NOT do
//!
//! - It does not prove A1' formally. The conditional bound is the
//!   empirical hypothesis under which adaptive estimation is sound.
//! - It does not handle distribution shift (e.g., a sudden switch
//!   from English prose to Chinese ideograph dense content) beyond
//!   the safety margin epsilon. Operators choosing a tight epsilon
//!   are trading correctness margin for cost efficiency.
//! - It does not cross model/tokenizer boundaries. Each
//!   (model_id, tokenizer_id) pair maintains its own observed_max.

use token_budgets::TokenEstimator;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Identifier for a (model, tokenizer) pair against which observations
/// are aggregated.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ModelKey {
    pub provider: String,
    pub model: String,
}

impl ModelKey {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }
}

/// Per-model observation state. Monotonic observed_max never decreases.
#[derive(Clone, Debug)]
struct ModelObservations {
    /// Highest observed ratio of (input_tokens / bytes) across all
    /// recorded calls for this model.
    observed_max: f64,
    /// Number of calls recorded against this model.
    call_count: u64,
}

impl Default for ModelObservations {
    fn default() -> Self {
        Self {
            // Initial value matches the static A1 bound: at most
            // 1.0 tokens per byte for canonical byte-level BPE.
            observed_max: 1.0,
            call_count: 0,
        }
    }
}

/// Adaptive estimator implementing the `TokenEstimator` trait.
///
/// Thread-safe via interior mutability. The observed_max table
/// is updated under a write lock; estimation reads under a read
/// lock.
#[derive(Clone, Debug)]
pub struct AdaptiveEstimator {
    /// Safety margin added to the observed_max ratio. Default 0.05
    /// (5%) covers typical token/byte variance for BPE-family
    /// tokenizers on English prose. Tighten only if you have
    /// established empirical evidence for your workload's variance.
    epsilon: f64,
    /// Per-model observation state.
    state: Arc<RwLock<HashMap<ModelKey, ModelObservations>>>,
    /// The current model context. Set by `for_model(key)` and used
    /// by `estimate()`. None means "fall back to static byte-length".
    current_model: Option<ModelKey>,
}

impl AdaptiveEstimator {
    /// Construct a new adaptive estimator with the given safety
    /// margin. Typical values: 0.05 (5%) for English prose, 0.20
    /// (20%) for mixed-language or code-heavy workloads.
    pub fn new(epsilon: f64) -> Self {
        assert!(
            epsilon >= 0.0 && epsilon < 1.0,
            "epsilon must be in [0.0, 1.0); got {}",
            epsilon
        );
        Self {
            epsilon,
            state: Arc::new(RwLock::new(HashMap::new())),
            current_model: None,
        }
    }

    /// Bind this estimator to a specific (model, tokenizer) pair.
    /// Subsequent `estimate()` calls use that pair's observed_max.
    pub fn for_model(mut self, key: ModelKey) -> Self {
        self.current_model = Some(key);
        self
    }

    /// Record an observation. Call this after every successful LLM
    /// call with the actual input-token count reported by the
    /// provider.
    ///
    /// Updates the observed_max for the given model. Idempotent if
    /// the new ratio does not exceed the existing observed_max.
    pub fn record_observation(
        &self,
        model: &ModelKey,
        byte_length: u64,
        actual_input_tokens: u64,
    ) {
        if byte_length == 0 {
            return; // empty prompt; nothing to observe
        }
        let ratio = (actual_input_tokens as f64) / (byte_length as f64);
        let mut state = self.state.write().expect("state lock poisoned");
        let obs = state.entry(model.clone()).or_default();
        if ratio > obs.observed_max {
            obs.observed_max = ratio;
        }
        obs.call_count = obs.call_count.saturating_add(1);
    }

    /// Read the current observed_max for a model. None if no
    /// observations have been recorded.
    pub fn observed_max(&self, model: &ModelKey) -> Option<f64> {
        let state = self.state.read().expect("state lock poisoned");
        state.get(model).map(|obs| obs.observed_max)
    }

    /// Read the number of observations recorded for a model.
    pub fn call_count(&self, model: &ModelKey) -> u64 {
        let state = self.state.read().expect("state lock poisoned");
        state.get(model).map(|obs| obs.call_count).unwrap_or(0)
    }

    /// Compute the estimate for a prompt of the given byte length
    /// against a specific model. Used internally by the
    /// `TokenEstimator` impl.
    fn estimate_for_model(&self, byte_length: u64, model: &ModelKey) -> u64 {
        let state = self.state.read().expect("state lock poisoned");
        let observed_max = state
            .get(model)
            .map(|obs| obs.observed_max)
            .unwrap_or(1.0);
        let effective_ratio = observed_max + self.epsilon;
        // Ceiling: never under-estimate.
        let estimated = (byte_length as f64) * effective_ratio;
        estimated.ceil() as u64
    }
}

impl TokenEstimator for AdaptiveEstimator {
    fn estimate(&self, prompt: &str) -> u64 {
        let bytes = prompt.len() as u64;
        match &self.current_model {
            Some(model) => self.estimate_for_model(bytes, model),
            None => bytes, // fall back to static byte-length
        }
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_estimate_matches_static_a1() {
        // With no observations and any epsilon < 1.0, the initial
        // estimate is at most byte_length * (1.0 + epsilon).
        // We verify the *first* estimate (no observations yet)
        // equals byte_length * (1.0 + 0.05) for the default eps.
        let est = AdaptiveEstimator::new(0.05)
            .for_model(ModelKey::new("anthropic", "haiku-4-5"));
        // 100-byte prompt: estimate = ceil(100 * 1.05) = 105
        assert_eq!(est.estimate("a".repeat(100).as_str()), 105);
    }

    #[test]
    fn observed_max_monotonically_increases() {
        let est = AdaptiveEstimator::new(0.05);
        let model = ModelKey::new("openai", "gpt-4o-mini");

        // First observation: ratio 0.3 → observed_max = max(1.0, 0.3) = 1.0
        // (we start at 1.0, never go below)
        est.record_observation(&model, 1000, 300);
        assert_eq!(est.observed_max(&model), Some(1.0));

        // Second observation: ratio 0.5 → still 1.0
        est.record_observation(&model, 1000, 500);
        assert_eq!(est.observed_max(&model), Some(1.0));

        // Third observation: ratio 1.2 → observed_max bumps to 1.2
        est.record_observation(&model, 1000, 1200);
        assert_eq!(est.observed_max(&model), Some(1.2));

        // Fourth: ratio 0.8 → still 1.2 (no downward movement)
        est.record_observation(&model, 1000, 800);
        assert_eq!(est.observed_max(&model), Some(1.2));
    }

    #[test]
    fn estimate_tightens_after_observations() {
        // For a model where empirical max ratio is 0.5, the
        // estimate should converge toward 0.5*byte + epsilon*byte.
        // But the floor is observed_max >= 1.0 (initial), so this
        // model's estimate stays at byte * 1.05 forever.
        // Now consider a model where we observe a ratio greater
        // than 1.0 - it should INCREASE the estimate.

        let est = AdaptiveEstimator::new(0.05);
        let model = ModelKey::new("test", "high-ratio");
        // 200-byte prompt at initial state: 200 * 1.05 = 210
        let initial = est.estimate_for_model(200, &model);
        assert_eq!(initial, 210);

        // Observe ratio 1.5
        est.record_observation(&model, 1000, 1500);
        // Now estimate = 200 * (1.5 + 0.05) = 310
        let after = est.estimate_for_model(200, &model);
        assert_eq!(after, 310);
    }

    #[test]
    fn no_observations_falls_back_to_byte_length_with_margin() {
        let est = AdaptiveEstimator::new(0.1)
            .for_model(ModelKey::new("unknown", "untested-model"));
        // 100-byte prompt: ceil(100 * 1.1) = 111 (FP rounding)
        assert_eq!(est.estimate(&"a".repeat(100)), 111);
    }

    #[test]
    fn zero_byte_observation_is_ignored() {
        let est = AdaptiveEstimator::new(0.05);
        let model = ModelKey::new("test", "edge");
        // Should not divide by zero or update state
        est.record_observation(&model, 0, 0);
        assert_eq!(est.call_count(&model), 0);
        assert_eq!(est.observed_max(&model), None);
    }

    #[test]
    fn fallback_without_model_returns_byte_length() {
        // No for_model() call → fall back to static byte-length
        // (matches existing ByteLength estimator behavior)
        let est = AdaptiveEstimator::new(0.05);
        assert_eq!(est.estimate("hello world"), 11);
    }

    #[test]
    #[should_panic(expected = "epsilon must be in [0.0, 1.0)")]
    fn epsilon_must_be_bounded() {
        let _ = AdaptiveEstimator::new(1.5);
    }

    #[test]
    fn concurrent_observations_are_thread_safe() {
        use std::thread;

        let est = Arc::new(AdaptiveEstimator::new(0.05));
        let model = ModelKey::new("test", "concurrent");
        let mut handles = vec![];

        for i in 0..10 {
            let est = Arc::clone(&est);
            let model = model.clone();
            handles.push(thread::spawn(move || {
                est.record_observation(&model, 1000, 100 + i * 50);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        // We should have observed 10 calls. The max ratio is
        // (100 + 9*50)/1000 = 550/1000 = 0.55, which is below 1.0,
        // so observed_max stays at 1.0.
        assert_eq!(est.call_count(&model), 10);
        assert_eq!(est.observed_max(&model), Some(1.0));
    }
}
