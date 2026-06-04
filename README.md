# token-budgets-extensions

Extensions to [token-budgets](https://github.com/sajjadanwar0/token-budgets)
that are **not** part of the main paper's contribution but build on its
foundations. Part of the *Token Budgets* artifact (preprint, 2026).

## What's here

### `adaptive-estimator/`

An online-learning extension to `AnthropicEstimator` that adapts its margin per
(model, prompt-distribution) tuple instead of a fixed 2.0x safety factor.

The main paper uses a fixed 2.0x margin and documents that the margin is
*load-bearing* (paper §5.30–§5.31): at margin 1.0x, A1 holds on 1/3 of audited
classes; at margin 2.0x, A1 holds 30/30. The fixed-margin design is intentionally
conservative. The adaptive estimator instead keeps a rolling histogram of
byte-length-to-token ratios per (model, prompt-class) and uses the 99th
percentile plus headroom, with a 1.2x safety floor.

The paper reports a live-API validation of the adaptive estimator (paper §5.28).
On the broad corpus, the static estimator's over-reservation is 6.20x mean
(2.51x median; N=5,190 per-call events, `token-budgets-experiments/refund-live/`).
On an adversarial corpus, the adaptive estimator cuts the *median effective
margin* from 3.92x (static 2.0x) to 2.11x
(`token-budgets-experiments/multiway/adaptive_adversarial_summary.csv`), with
zero A1 violations on the audited corpora. Treat those figures as audited-corpus
results, not a deployment guarantee: re-validate on your own prompt distribution
before relying on a tighter margin.

**Status: research extension.** The adaptive estimator's safety properties are
not mechanised and are validated only on the audited corpora. The fixed-margin
estimator in the main crate is the recommended default.

### `verus-skeleton/`

A Verus skeleton for the adaptive estimator with pre/post-conditions on
`update(ratio)` and `current_margin()`, and one unproven obligation: that if the
histogram is well-formed and the safety floor is honoured, the cap-soundness
result lifts to the adaptive case. Proving it would require a probabilistic
envelope over the histogram and likely an Iris-style separation logic.

## Open research questions

1. What is the worst-case safety guarantee under a 1.2x floor on adversarial
   prompt distributions?
2. How fast does the per-class histogram converge? (Empirically ~50 calls.)
3. Can this extend to non-tokenizer cost models (reasoning tokens, image, audio)?

## Companion components

- [token-budgets](https://github.com/sajjadanwar0/token-budgets) — main library (validated defaults)
- token-budgets-formals — mechanized cross-checks of the main library
- token-budgets-experiments — empirical evaluation

## Citation

```bibtex
@misc{khan2026tokenbudgets,
  title         = {Token Budgets: An Empirical Catalog of 63 LLM-Agent
                   Budget-Overrun Incidents, with an Affine-Typed Rust
                   Mitigation as a Case Study},
  author        = {Khan, Sajjad},
  year          = {2026},
  eprint        = {2606.04056},
  archivePrefix = {arXiv},
  primaryClass  = {cs.SE},
  doi           = {10.48550/arXiv.2606.04056},
  url           = {https://arxiv.org/abs/2606.04056}
}
```

## License

Paper: CC BY 4.0 (arXiv). Code: see the repository `LICENSE` file.