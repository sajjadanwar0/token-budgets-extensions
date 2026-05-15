# Token Budgets — Extensions

> Two extensions to the [`token-budgets`](https://github.com/sajjadanwar0/token-budgets)
> affine-resource discipline:
>
> 1. **Adaptive estimator** — tighter input reservation than `ByteLength` via observed-max bookkeeping. Trades A1 for a conditional A1'.
> 2. **Verus skeleton** (OPEN WORK) — honest scaffolding for the operational-refinement obligation (Conjecture 1). The refinement itself is **admitted**.

[![Adaptive](https://img.shields.io/badge/adaptive_estimator-implemented-brightgreen)](#adaptive-estimator)
[![Verus skel](https://img.shields.io/badge/verus_skeleton-admitted-orange)](#verus-skeleton-conjecture-1-open-work)
[![License](https://img.shields.io/badge/license-MIT_OR_Apache--2.0-blue)](LICENSE-MIT)

> **Note**: `StreamingReceipt` lives in the main
> [`token-budgets`](https://github.com/sajjadanwar0/token-budgets) crate
> (via `Budget::spend_streaming` → `StreamingReceipt::confirm_chunk`),
> not here.

## What these extensions are NOT

These are not the headline contributions of the paper. The main affine
discipline lives in [`token-budgets`](https://github.com/sajjadanwar0/token-budgets)
and stands on its own evidence — cap-soundness mechanized in
[`token-budgets-formals`](https://github.com/sajjadanwar0/token-budgets-formals)
across five independent provers (TLAPS 497 obligations, TLC 252
distinct states, Coq with 0 Admitted, Dafny 23 verified, Verus
66 theorems). Empirical validation across 5,424 live API row-events
is in [`token-budgets-experiments`](https://github.com/sajjadanwar0/token-budgets-experiments).

This repository explores **two directions reviewers asked about**:

1. "Why not adapt the reservation to observed usage instead of using
   the byte-length upper bound?" → Adaptive estimator below.
2. "What about the trace-level refinement to a running Tokio binary?"
   → Verus skeleton (open; the gap is not closed here).

## Adaptive estimator

**Status**: implemented and tested.

The default `ByteLength` estimator in the main crate gives a sound but
loose upper bound. For English text, average input-tokens-per-byte is
roughly 0.25; the reservation is therefore ~4× the actual usage on
typical prompts (empirically 6.20× across our 5,190-sample corpus —
see Notes below). Over the long run, the refund mechanism reconciles
this, but the *reservation* still has to fit within the budget at the
moment of reservation — meaning a tighter estimator increases the
fraction of calls that can be reserved against a small budget.

### Design

```rust
use token_budgets_extensions::AdaptiveEstimator;

let est = AdaptiveEstimator::new()
.for_model("claude-haiku-4-5-20251001")
.with_safety_eps(0.05);

// On every observed (prompt, actual_input_tokens) pair, the estimator
// updates an internal observed_max ratio.
est.observe(prompt, actual_in_tokens);

// Future estimates use byte_length * (observed_max + eps).
let estimate = est.estimate(future_prompt);
```

### What this trades

The trade-off is explicit in the paper (§VII):

| Aspect | Static `ByteLength` (main crate) | `AdaptiveEstimator` (this crate) |
|---|---|---|
| Soundness | **A1**: unconditional UTF-8 byte-length dominance | **A1'**: future prompts respect prior-observed max + ε |
| Tightness | ~4× over-reservation typical | ~1.05-1.2× over-reservation after warm-up |
| Cold-start | Sound from call 1 | Falls back to byte-length until observed_max stabilizes |
| Failure mode | None (A1 is universal) | A novel prompt with unusually high tokens-per-byte ratio overshoots; A1 violation detected at confirm time |

### Build and test

```bash
cargo test --release
```

### Empirical note

The empirical mean over-reservation ratio across our 5,190 valid
margin_ratio samples in `refund-live` is **6.20×** (range 1.02× to
131.43×). The paper's "6.4× indicative" figure is consistent with
this. The adaptive estimator's value proposition is largest for
high-throughput single-tenant workloads where the same provider/model
pair is hit thousands of times in quick succession.

## Verus skeleton (Conjecture 1, OPEN WORK)

**Status**: skeleton only; the refinement is **ADMITTED**.

This extension is the scaffolding for Conjecture 1 from the main
paper: that the abstract `Budget` state machine operationally refines
the concrete Rust API running under Tokio. **The refinement is not
closed by this skeleton.** Closing it is multi-person-month research.

### Where this fits relative to the paper's five-tier stack

The paper's five mechanized tools — TLAPS (497 obligations), TLC (252
distinct states), Coq stdlib (`budget.v`, 0 Admitted), Dafny (23
verified), and Verus (66 theorems on Rust source) — together prove
cap-soundness from five independent angles:

- TLAPS, TLC, Coq, and Dafny all verify the **abstract state machine**
- Verus verifies the **actual Rust source code**

What none of them prove is that the *running Tokio binary* under
work-stealing scheduling and Drop-on-unwind semantics observably
matches the source-level Verus mechanization. That gap is **Conjecture
1**, and this skeleton is the entry point for closing it.

### What the skeleton provides

The directory `verus-skeleton/` contains a separate Verus crate (not
checked by stock `cargo`; requires the Verus toolchain). It:

1. **Specifies** the tokenized state machine via
   `tokenized_state_machine! { BudgetSM { ... } }`.
2. **Proves** the conservation invariant (`available + reserved +
   spent + refunded = initial_cap`) for the four abstract transitions
   (`initial`, `reserve`, `confirm`, `abort`).
3. **States** the refinement theorem connecting BudgetSM to the
   concrete Rust API.
4. **Admits** the refinement theorem via `#[verifier::external_body]`.

The four conservation-preservation proofs go through Verus's SMT
solver mechanically. The refinement admission is the open obligation
— see `verus-skeleton/docs/STATUS.md` for what's proven vs admitted,
and `verus-skeleton/docs/PROOF_PLAN.md` for an estimate of what
closing the gap would entail (2-4 person-months via Verus, 4-6 via
Coq/Iris/RustBelt).

### Why ship a skeleton at all

Three reasons:

1. **Precision**: stating the obligation as a Verus signature is more
   precise than prose. A reviewer can see exactly what the missing
   simulation relation would need to prove.
2. **Resume-completeness**: when someone closes the gap, the state
   machine, invariants, and structural lemmas don't need to be
   redone.
3. **Honesty over hand-waving**: leaving the obligation unstated
   would let readers assume the empirical evidence is the proof.
   Stating it (and admitting it) makes the gap explicit.

### Run the skeleton (Verus required)

```bash
cd verus-skeleton/
verus src/lib.rs
# Expect: structural-lemma proofs to verify, refinement to be
# accepted as external_body
```

If you don't have Verus installed, see
https://verus-lang.github.io/verus/guide/getting_started.html

## Repository layout

```
token-budgets-extensions/
├── src/
│   ├── lib.rs            # Public re-exports
│   └── adaptive.rs       # AdaptiveEstimator + observed_max bookkeeping
├── verus-skeleton/        # Open Conjecture 1 scaffolding
│   ├── src/lib.rs
│   ├── docs/STATUS.md
│   └── docs/PROOF_PLAN.md
├── tests/
│   └── adaptive.rs
├── Cargo.toml
├── README.md              # This file
├── LICENSE-MIT
└── LICENSE-APACHE
```

## Build all (stock Cargo)

```bash
cargo build --release
cargo test --release
```

(The `verus-skeleton/` subdirectory is a separate workspace and is
not built by the parent `cargo`. See its own README.)

## Paper

```bibtex
@article{khan-token-budgets-2026,
  author  = {Khan, Sajjad},
  title   = {Token Budgets: An Affine-Resource Discipline for LLM Cost Caps in Rust},
  journal = {arXiv preprint arXiv:TBD},
  year    = {2026}
}
```

The adaptive estimator is discussed in paper §VII; Conjecture 1 and
its open status are in §VII as well.

## Related repositories

| Repository | What it contains |
|---|---|
| [`token-budgets`](https://github.com/sajjadanwar0/token-budgets) | Main affine-API library + 167-entry catalog |
| [`token-budgets-extensions`](https://github.com/sajjadanwar0/token-budgets-extensions) | This repo |
| [`token-budgets-formals`](https://github.com/sajjadanwar0/token-budgets-formals) | 5-tier mechanization (TLAPS / TLC / Coq / Dafny / Verus) |
| [`token-budgets-experiments`](https://github.com/sajjadanwar0/token-budgets-experiments) | Empirical validation (5,424 row-events) |
| [`rig-budget`](https://github.com/sajjadanwar0/rig-budget) | Integration with the `rig` LLM framework |

## License

Dual MIT/Apache-2.0. See `LICENSE-MIT` and `LICENSE-APACHE`.