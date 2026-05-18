use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct InterimRefundCounter {
    inner: Arc<AtomicU64>,
}

impl InterimRefundCounter {
    fn new() -> Self {
        Self {
            inner: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn current(&self) -> u64 {
        self.inner.load(Ordering::Acquire)
    }

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

pub type RateNanoCents = u64;

pub struct StreamingReceipt {
    reservation_nc: u64,
    rate_out_nc: RateNanoCents,
    max_output_tokens: u64,
    tokens_so_far: u64,
    interim: InterimRefundCounter,
    _consumed_marker: std::marker::PhantomData<*const ()>,
}

impl StreamingReceipt {
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

    pub fn interim_counter(&self) -> InterimRefundCounter {
        self.interim.clone()
    }

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

        let unconsumed = self.max_output_tokens.saturating_sub(tokens_so_far);
        let still_reserved = unconsumed.saturating_mul(self.rate_out_nc);

        let original_output_reservation =
            self.max_output_tokens.saturating_mul(self.rate_out_nc);
        let interim_refund_amount =
            original_output_reservation.saturating_sub(still_reserved);

        self.interim.update(interim_refund_amount);
        interim_refund_amount
    }

    pub fn finalize(self) -> u64 {
        let actual_output_cost =
            self.tokens_so_far.saturating_mul(self.rate_out_nc);

        self.reservation_nc.saturating_sub(actual_output_cost)
    }

    pub fn forfeit(self) { }

    pub fn tokens_so_far(&self) -> u64 {
        self.tokens_so_far
    }

    pub fn max_output_tokens(&self) -> u64 {
        self.max_output_tokens
    }

    pub fn reservation_nc(&self) -> u64 {
        self.reservation_nc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const GPT_4O_MINI_OUT_NC: u64 = 600;

    #[test]
    fn record_chunk_returns_monotone_interim_refund() {

        let mut sr = StreamingReceipt::new(800_000, 1000, GPT_4O_MINI_OUT_NC);

        let r1 = sr.record_chunk(100);
        assert_eq!(r1, 60_000);

        let r2 = sr.record_chunk(250);
        assert_eq!(r2, 150_000);

        let r3 = sr.record_chunk(500);
        assert_eq!(r3, 300_000);

        assert!(r1 < r2);
        assert!(r2 < r3);
    }

    #[test]
    fn finalize_computes_correct_refund() {
        let mut sr = StreamingReceipt::new(800_000, 1000, GPT_4O_MINI_OUT_NC);
        sr.record_chunk(250);
        sr.record_chunk(500);
        sr.record_chunk(750);
        assert_eq!(sr.finalize(), 350_000);
    }

    #[test]
    fn finalize_independent_of_chunk_order_count() {
        let mut sr_a = StreamingReceipt::new(800_000, 1000, GPT_4O_MINI_OUT_NC);
        sr_a.record_chunk(100);
        sr_a.record_chunk(200);
        sr_a.record_chunk(300);
        sr_a.record_chunk(400);
        sr_a.record_chunk(500);
        let final_a = sr_a.finalize();

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
        sr.forfeit();
    }

    #[test]
    fn zero_tokens_stream_full_refund_of_output_portion() {
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
        sr.record_chunk(1500);
    }

    #[test]
    fn saturating_arithmetic_no_underflow() {
        let mut sr = StreamingReceipt::new(800_000, 1000, GPT_4O_MINI_OUT_NC);
        sr.record_chunk(1000);
        assert_eq!(sr.finalize(), 200_000);
    }

    #[test]
    fn poll_handle_clones_independently() {
        let mut sr = StreamingReceipt::new(800_000, 1000, GPT_4O_MINI_OUT_NC);
        let h1 = sr.interim_counter();
        let h2 = sr.interim_counter();
        sr.record_chunk(500);
        assert_eq!(h1.current(), 300_000);
        assert_eq!(h2.current(), 300_000);
    }
}
