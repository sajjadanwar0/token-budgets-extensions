# token-budgets-extensions

Extensions to the [token-budgets](https://github.com/sajjadanwar0/token-budgets) library that are not part of the main paper's contribution but build on its foundations.

## What's here

### `adaptive-estimator/`

An online-learning extension to the `AnthropicEstimator` that adapts its margin per (model, prompt-distribution) tuple, instead of using a fixed 2.0× safety factor.

The main paper uses a fixed 2.0× margin and documents that the margin is *load-bearing*: at margin 1.0×, A1 (UTF-8 byte-length dominance) holds only 1/3 of test cells; at margin 2.0×, A1 holds 30/30. This fixed-margin design is intentionally conservative.

The adaptive extension here explores a more aggressive design:

- Each (model, prompt-class) pair maintains a rolling-window histogram of byte-length → actual-tokens ratios.
- The 99th-percentile observed ratio plus a small headroom is used as the per-class margin.
- A safety floor of 1.2× prevents the adaptive margin from collapsing to unsafe values when sample sizes are small.

**Status: prototype.** The adaptive estimator's safety properties have NOT been mechanized; they have not been validated empirically beyond the unit tests in this repository. **Do not use this in production without further evaluation.** The fixed-margin estimator in the main crate is the validated, recommended default.

### `verus-skeleton/`

A Verus mechanization skeleton for the adaptive estimator, with:

- The abstract type `AdaptiveEstimator<H>` where `H` is a histogram parameter.
- Pre/post-conditions on `update(ratio)` and `current_margin()`.
- An unproven obligation: "if H is well-formed and the safety floor is honored, then the cap-soundness theorem (Verus tier 1 in the main paper) lifts to the adaptive case."

The unproven obligation is the natural next research question. It would require formalizing a probabilistic envelope over the histogram and may need an Iris-like separation logic for the proof.

## Why this lives in a separate repository

The main `token-budgets` crate is intentionally minimal and conservative. Every claim it makes is formally verified or empirically calibrated. The extensions here are speculative — interesting future directions, but not validated to the standard of the main paper.

Keeping them separate makes the main crate's contract clearer to readers and reviewers, while still providing a public landing zone for the follow-up work.

## How to use (if you must)

```toml
[dependencies]
token-budgets = "0.5"
token-budgets-extensions = { git = "https://github.com/sajjadanwar0/token-budgets-extensions" }
```

```rust
use token_budgets::Budget;
use token_budgets_extensions::AdaptiveEstimator;

let estimator = AdaptiveEstimator::new()
    .with_safety_floor(1.2);
let estimate = estimator.estimate(&prompt, ModelClass::Anthropic);
// ... use as a drop-in replacement for AnthropicEstimator
```

The API surface mirrors the main crate's `AnthropicEstimator`. Replacing one with the other is a one-line change.

## Open research questions

The adaptive estimator raises questions that are genuinely open:

1. **What is the worst-case safety guarantee?** With a 1.2× floor, A1 can still be violated on adversarial prompt distributions. What does a sound lower bound look like?
2. **How fast does the histogram converge?** Empirically, the per-class margin stabilizes within ~50 calls; theoretically, the convergence rate depends on the prompt distribution's tail.
3. **Can this be extended to non-tokenizer-based cost models?** GPT-OSS reasoning tokens, image inputs, audio — each has its own scaling law.

If you're a PhD student looking for a project, any of these is a credible MSc/PhD scope.

## Companion repositories

- [token-budgets](https://github.com/sajjadanwar0/token-budgets) — main library (validated, conservative defaults)
- [token-budgets-formals](https://github.com/sajjadanwar0/token-budgets-formals) — formal verification of the main library
- [token-budgets-experiments](https://github.com/sajjadanwar0/token-budgets-experiments) — empirical evaluation


## License

Dual MIT/Apache-2.0. See `LICENSE-MIT` and `LICENSE-APACHE`.