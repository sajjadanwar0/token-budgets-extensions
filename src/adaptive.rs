use token_budgets::TokenEstimator;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};


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

#[derive(Clone, Debug)]
struct ModelObservations {
    observed_max: f64,
    call_count: u64,
}

impl Default for ModelObservations {
    fn default() -> Self {
        Self {
            observed_max: 1.0,
            call_count: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AdaptiveEstimator {
    epsilon: f64,
    state: Arc<RwLock<HashMap<ModelKey, ModelObservations>>>,
    current_model: Option<ModelKey>,
}

impl AdaptiveEstimator {
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

    pub fn for_model(mut self, key: ModelKey) -> Self {
        self.current_model = Some(key);
        self
    }
    
    pub fn record_observation(
        &self,
        model: &ModelKey,
        byte_length: u64,
        actual_input_tokens: u64,
    ) {
        if byte_length == 0 {
            return; 
        }
        let ratio = (actual_input_tokens as f64) / (byte_length as f64);
        let mut state = self.state.write().expect("state lock poisoned");
        let obs = state.entry(model.clone()).or_default();
        
        if ratio > obs.observed_max {
            obs.observed_max = ratio;
        }
        obs.call_count = obs.call_count.saturating_add(1);
    }
    
    pub fn observed_max(&self, model: &ModelKey) -> Option<f64> {
        let state = self.state.read().expect("state lock poisoned");
        state.get(model).map(|obs| obs.observed_max)
    }

    pub fn call_count(&self, model: &ModelKey) -> u64 {
        let state = self.state.read().expect("state lock poisoned");
        state.get(model).map(|obs| obs.call_count).unwrap_or(0)
    }
    
    fn estimate_for_model(&self, byte_length: u64, model: &ModelKey) -> u64 {
        let state = self.state.read().expect("state lock poisoned");
        let observed_max = state
            .get(model)
            .map(|obs| obs.observed_max)
            .unwrap_or(1.0);
        let effective_ratio = observed_max + self.epsilon;
        let estimated = (byte_length as f64) * effective_ratio;
        estimated.ceil() as u64
    }
}

impl TokenEstimator for AdaptiveEstimator {
    fn estimate(&self, prompt: &str) -> u64 {
        let bytes = prompt.len() as u64;
        match &self.current_model {
            Some(model) => self.estimate_for_model(bytes, model),
            None => bytes, 
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_estimate_matches_static_a1() {
        let est = AdaptiveEstimator::new(0.05)
            .for_model(ModelKey::new("anthropic", "haiku-4-5"));
        assert_eq!(est.estimate("a".repeat(100).as_str()), 105);
    }

    #[test]
    fn observed_max_monotonically_increases() {
        let est = AdaptiveEstimator::new(0.05);
        let model = ModelKey::new("openai", "gpt-4o-mini");

        est.record_observation(&model, 1000, 300);
        assert_eq!(est.observed_max(&model), Some(1.0));

        est.record_observation(&model, 1000, 500);
        assert_eq!(est.observed_max(&model), Some(1.0));

        est.record_observation(&model, 1000, 1200);
        assert_eq!(est.observed_max(&model), Some(1.2));

        est.record_observation(&model, 1000, 800);
        assert_eq!(est.observed_max(&model), Some(1.2));
    }

    #[test]
    fn estimate_tightens_after_observations() {
        let est = AdaptiveEstimator::new(0.05);
        let model = ModelKey::new("test", "high-ratio");
        let initial = est.estimate_for_model(200, &model);
        assert_eq!(initial, 210);

        est.record_observation(&model, 1000, 1500);
        let after = est.estimate_for_model(200, &model);
        assert_eq!(after, 310);
    }

    #[test]
    fn no_observations_falls_back_to_byte_length_with_margin() {
        let est = AdaptiveEstimator::new(0.1)
            .for_model(ModelKey::new("unknown", "untested-model"));
        assert_eq!(est.estimate(&"a".repeat(100)), 111);
    }

    #[test]
    fn zero_byte_observation_is_ignored() {
        let est = AdaptiveEstimator::new(0.05);
        let model = ModelKey::new("test", "edge");
        est.record_observation(&model, 0, 0);
        assert_eq!(est.call_count(&model), 0);
        assert_eq!(est.observed_max(&model), None);
    }

    #[test]
    fn fallback_without_model_returns_byte_length() {
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
        
        assert_eq!(est.call_count(&model), 10);
        assert_eq!(est.observed_max(&model), Some(1.0));
    }
}
