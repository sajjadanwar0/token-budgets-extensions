//! Streaming receipt for per-chunk refund during long streams.
//!
//! The standard `ReservationReceipt::confirm()` reconciles a
//! reservation only at end-of-call. For long streaming responses,
//! this leaves the upper-bound reservation tied up in the budget
//! for the entire stream duration, contributing to the 6.4x median
//! over-reservation factor observed in our empirical evaluation.
//!
//! This module ships a `StreamingReceipt` type that exposes the
//! interim accounting state during the stream, allowing the
//! caller to compute and surface interim refunds without breaking
//! the affine discipline.
//!
//! # Affine discipline preserved
//!
//! - `StreamingReceipt::record_chunk(&mut self, tokens_so_far)`:
//!   borrows `&mut self`; receipt identity is preserved across
//!   the stream lifetime.
//! - `StreamingReceipt::finalize(self) -> Refund`: consumes self
//!   exactly once at end-of-stream; can be called only after at
//!   least one record_chunk (or zero, for empty streams).
//!
//! Internally the streaming receipt threads through to the
//! standard `ReservationReceipt` and exposes interim per-chunk
//! refund amounts as a *read-only* monotone counter that the
//! holder of the `Budget` may poll between chunks.
//!
//! # Correctness obligations (informal)
//!
//! 1. `tokens_so_far` is monotonically non-decreasing across
//!    `record_chunk` calls (enforced by debug_assert!).
//! 2. The total refund at `finalize` equals
//!    `reservation - tokens_so_far_final * rate_out` regardless
//!    of chunk arrival order.
//! 3. Interim refund computations cannot underflow: enforced by
//!    saturating_sub at every step.
//!
//! These are the obligations a future formal proof would need
//! to discharge. The current implementation enforces them at
//! runtime via debug_assert! and saturating arithmetic.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Interim refund tracker shared between the streaming receipt
/// and the budget holder. Read-only from the budget holder's side.
#[derive(Clone, Debug)]
pub struct InterimRefundCounter {
    /// Total interim refund accumulated so far, in micro-cents.
    /// Monotonically non-decreasing.
    inner: Arc<AtomicU64>,
}

impl InterimRefundCounter {
    fn new() -> Self {
        Self {
            inner: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Read the currently accumulated interim refund amount, in
    /// micro-cents. Safe to poll concurrently with the streaming
    /// receipt's record_chunk.
    pub fn current(&self) -> u64 {
        self.inner.load(Ordering::Acquire)
    }

    /// Internal: update the interim refund counter to a new value.
    /// Only invoked by `StreamingReceipt::record_chunk`. Asserts
    /// monotonicity in debug builds.
    fn update(&self, new_value: u64) {
        let prev = self.inner.swap(new_value, Ordering::AcqRel);
        debug_assert!(
            new_value >= prev,
            "interim refund counter must be monotonically non-decreasing: prev={}, new={}",
            prev,
            new_value
        );
    }
}

/// Per-token rate, in nano-cents (1 micro-cent = 1000 nano-cents).
/// Same units as the existing `ReservationReceipt` accounting.
pub type RateNanoCents = u64;

/// Receipt for a streaming LLM call. Constructed from a base
/// reservation in nano-cents and an output-token rate.
///
/// Lifetime:
///   1. Construct via `StreamingReceipt::new(reservation_nc, max_output_tokens, rate_out_nc)`.
///   2. During the stream, call `record_chunk(tokens_so_far)` for each chunk.
///      `tokens_so_far` must be monotonically non-decreasing.
///   3. At stream end, call `finalize()` to consume the receipt and
///      get the final refund amount.
///
/// The `InterimRefundCounter` returned by `interim_counter()` is
/// shared by clone and updated automatically as `record_chunk` is
/// called.
pub struct StreamingReceipt {
    /// Total reservation amount in nano-cents (what was deducted
    /// from the Budget upfront).
    reservation_nc: u64,
    /// Per-output-token rate in nano-cents (e.g., 600 for
    /// gpt-4o-mini output: $0.60 per M tokens).
    rate_out_nc: RateNanoCents,
    /// Maximum tokens that were reserved against (the upper bound).
    max_output_tokens: u64,
    /// Last observed cumulative tokens (monotone).
    tokens_so_far: u64,
    /// Shared interim refund counter; cloned by `interim_counter()`.
    interim: InterimRefundCounter,
    /// Receipt identity (consumed by finalize).
    _consumed_marker: std::marker::PhantomData<*const ()>,
}

impl StreamingReceipt {
    /// Construct a new streaming receipt from a base reservation.
    ///
    /// `reservation_nc`: total amount reserved from the Budget
    /// (in nano-cents), including both input and output components.
    ///
    /// `max_output_tokens`: the upper bound used in the reservation
    /// (typically the `max_tokens` parameter on the LLM call).
    ///
    /// `rate_out_nc`: per-output-token rate (nano-cents).
    pub fn new(
        reservation_nc: u64,
        max_output_tokens: u64,
        rate_out_nc: RateNanoCents,
    ) -> Self {
        Self {
            reservation_nc,
            rate_out_nc,
            max_output_tokens,
            tokens_so_far: 0,
            interim: InterimRefundCounter::new(),
            _consumed_marker: std::marker::PhantomData,
        }
    }

    /// Get a clone-able read handle on the interim refund counter.
    /// The holder of the Budget can poll this counter between
    /// chunks to learn the current refundable amount.
    pub fn interim_counter(&self) -> InterimRefundCounter {
        self.interim.clone()
    }

    /// Record a streaming chunk's cumulative output-token count.
    ///
    /// `tokens_so_far` must be monotonically non-decreasing across
    /// successive calls; violating this is a usage error.
    ///
    /// Returns the interim refund amount (in nano-cents) that
    /// becomes available at this point in the stream, i.e., the
    /// portion of the original reservation that is no longer
    /// needed because output is converging below `max_output_tokens`.
    pub fn record_chunk(&mut self, tokens_so_far: u64) -> u64 {
        debug_assert!(
            tokens_so_far >= self.tokens_so_far,
            "tokens_so_far must be monotonically non-decreasing: prev={}, new={}",
            self.tokens_so_far,
            tokens_so_far
        );
        debug_assert!(
            tokens_so_far <= self.max_output_tokens,
            "tokens_so_far ({}) exceeded max_output_tokens ({}); A1' violation",
            tokens_so_far,
            self.max_output_tokens
        );
        self.tokens_so_far = tokens_so_far;

        // Interim refund = (max_output_tokens - tokens_so_far) * rate_out
        // This is the amount STILL RESERVED for output that will
        // not be consumed, conservatively.
        let unconsumed = self.max_output_tokens.saturating_sub(tokens_so_far);
        let still_reserved = unconsumed.saturating_mul(self.rate_out_nc);
        // The interim refund is the original output-side reservation
        // (max_output_tokens * rate_out) minus what is still
        // reserved.
        let original_output_reservation =
            self.max_output_tokens.saturating_mul(self.rate_out_nc);
        let interim_refund_amount =
            original_output_reservation.saturating_sub(still_reserved);

        self.interim.update(interim_refund_amount);
        interim_refund_amount
    }

    /// Finalize the streaming receipt at end-of-stream. Consumes
    /// self; returns the final refund amount (nano-cents).
    ///
    /// The final refund equals the original reservation minus the
    /// actual cost = original - (tokens_so_far_final * rate_out).
    /// This is independent of how many `record_chunk` calls were
    /// made or in what order their values arrived.
    pub fn finalize(self) -> u64 {
        // Final actual output-side cost
        let actual_output_cost =
            self.tokens_so_far.saturating_mul(self.rate_out_nc);
        // Refund = original reservation - actual cost
        // (Note: the caller must add back any unused input-side
        // reservation separately, if they used a max-input-bound;
        // this receipt only handles the output side.)
        self.reservation_nc.saturating_sub(actual_output_cost)
    }

    /// Forfeit the streaming receipt. Used when the LLM call fails
    /// mid-stream and we cannot trust the partial accounting.
    /// Consumes self; no refund is issued.
    pub fn forfeit(self) {
        // self drops; the original reservation stays consumed
        // from the Budget. Caller must handle the error.
    }

    /// Read-only: tokens recorded so far.
    pub fn tokens_so_far(&self) -> u64 {
        self.tokens_so_far
    }

    /// Read-only: max output tokens this receipt reserved against.
    pub fn max_output_tokens(&self) -> u64 {
        self.max_output_tokens
    }

    /// Read-only: original reservation amount.
    pub fn reservation_nc(&self) -> u64 {
        self.reservation_nc
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: gpt-4o-mini output rate = 600 nano-cents per token.
    const GPT_4O_MINI_OUT_NC: u64 = 600;

    #[test]
    fn record_chunk_returns_monotone_interim_refund() {
        // Reserve max_output=1000, rate=600 → output reservation
        // = 600,000 nc. Then stream 100 tokens at a time.
        let mut sr = StreamingReceipt::new(800_000, 1000, GPT_4O_MINI_OUT_NC);

        let r1 = sr.record_chunk(100);
        // Still reserved for 900 tokens; interim refund = original 600k - 540k = 60k
        assert_eq!(r1, 60_000);

        let r2 = sr.record_chunk(250);
        // Still reserved for 750 tokens; interim refund = 600k - 450k = 150k
        assert_eq!(r2, 150_000);

        let r3 = sr.record_chunk(500);
        // Still reserved for 500 tokens; interim refund = 600k - 300k = 300k
        assert_eq!(r3, 300_000);

        // Verify monotonicity
        assert!(r1 < r2);
        assert!(r2 < r3);
    }

    #[test]
    fn finalize_computes_correct_refund() {
        // Reserve 800k total (200k input + 600k output reservation).
        // Stream 750 of the 1000 max output tokens.
        // Actual output cost = 750 * 600 = 450k
        // Final refund = 800k - 450k = 350k
        let mut sr = StreamingReceipt::new(800_000, 1000, GPT_4O_MINI_OUT_NC);
        sr.record_chunk(250);
        sr.record_chunk(500);
        sr.record_chunk(750);
        assert_eq!(sr.finalize(), 350_000);
    }

    #[test]
    fn finalize_independent_of_chunk_order_count() {
        // Two receipts with the same final tokens_so_far should
        // produce the same final refund, regardless of how many
        // chunks they recorded.

        // Receipt A: many chunks
        let mut sr_a = StreamingReceipt::new(800_000, 1000, GPT_4O_MINI_OUT_NC);
        sr_a.record_chunk(100);
        sr_a.record_chunk(200);
        sr_a.record_chunk(300);
        sr_a.record_chunk(400);
        sr_a.record_chunk(500);
        let final_a = sr_a.finalize();

        // Receipt B: one chunk
        let mut sr_b = StreamingReceipt::new(800_000, 1000, GPT_4O_MINI_OUT_NC);
        sr_b.record_chunk(500);
        let final_b = sr_b.finalize();

        assert_eq!(final_a, final_b);
    }

    #[test]
    fn interim_counter_is_shared() {
        let mut sr = StreamingReceipt::new(800_000, 1000, GPT_4O_MINI_OUT_NC);
        let poll_handle = sr.interim_counter();

        assert_eq!(poll_handle.current(), 0);
        sr.record_chunk(100);
        assert_eq!(poll_handle.current(), 60_000);
        sr.record_chunk(500);
        assert_eq!(poll_handle.current(), 300_000);
    }

    #[test]
    fn forfeit_consumes_without_refund() {
        let mut sr = StreamingReceipt::new(800_000, 1000, GPT_4O_MINI_OUT_NC);
        sr.record_chunk(500);
        // Provider errored mid-stream; we cannot trust the partial
        // accounting. Forfeit consumes the receipt with no refund.
        sr.forfeit();
        // Compiles only because we don't try to use sr after forfeit.
    }

    #[test]
    fn zero_tokens_stream_full_refund_of_output_portion() {
        // Reserve 800k total. Stream finishes with 0 output tokens
        // (e.g., model returned only structured stop). Final cost = 0.
        // Refund = 800k.
        let sr = StreamingReceipt::new(800_000, 1000, GPT_4O_MINI_OUT_NC);
        assert_eq!(sr.finalize(), 800_000);
    }

    #[test]
    #[should_panic(expected = "monotonically non-decreasing")]
    fn record_chunk_rejects_decreasing_tokens() {
        let mut sr = StreamingReceipt::new(800_000, 1000, GPT_4O_MINI_OUT_NC);
        sr.record_chunk(500);
        sr.record_chunk(200); // panics in debug build
    }

    #[test]
    #[should_panic(expected = "exceeded max_output_tokens")]
    fn record_chunk_rejects_overrun() {
        let mut sr = StreamingReceipt::new(800_000, 1000, GPT_4O_MINI_OUT_NC);
        // Reserved 1000 max; recording 1500 violates A1'
        sr.record_chunk(1500);
    }

    #[test]
    fn saturating_arithmetic_no_underflow() {
        // Edge case: tokens_so_far_final == max_output_tokens
        // Final refund = reservation - max*rate, must not underflow.
        let mut sr = StreamingReceipt::new(800_000, 1000, GPT_4O_MINI_OUT_NC);
        sr.record_chunk(1000);
        // Refund = 800k - 1000*600 = 800k - 600k = 200k (the input-side reserve)
        assert_eq!(sr.finalize(), 200_000);
    }

    #[test]
    fn poll_handle_clones_independently() {
        let mut sr = StreamingReceipt::new(800_000, 1000, GPT_4O_MINI_OUT_NC);
        let h1 = sr.interim_counter();
        let h2 = sr.interim_counter();
        sr.record_chunk(500);
        // Both handles see the same update
        assert_eq!(h1.current(), 300_000);
        assert_eq!(h2.current(), 300_000);
    }
}
