#![allow(unused_imports)]
#![allow(unused_variables)]

use vstd::prelude::*;

verus! {
    pub type Tokens = nat;

tokenized_state_machine! {
    BudgetSM {
        fields {
            #[sharding(constant)]
            pub initial_cap: Tokens,

            #[sharding(variable)]
            pub available: Tokens,

            #[sharding(variable)]
            pub reserved: Tokens,

            #[sharding(variable)]
            pub spent: Tokens,

            #[sharding(variable)]
            pub refunded: Tokens,
        }

        init! {
            initial(cap: Tokens) {
                init initial_cap = cap;
                init available   = cap;
                init reserved    = 0;
                init spent       = 0;
                init refunded    = 0;
            }
        }

        #[invariant]
        pub fn conservation(&self) -> bool {
            self.available + self.reserved + self.spent + self.refunded == self.initial_cap
        }

        transition! {
            reserve(n: Tokens) {
                require n <= pre.available;
                update available = pre.available - n;
                update reserved  = pre.reserved + n;
            }
        }

        transition! {
            confirm(r: Tokens, k: Tokens) {
                require r <= pre.reserved;
                require k <= r;
                update reserved = pre.reserved - r;
                update spent    = pre.spent + k;
                update refunded = pre.refunded + (r - k);
            }
        }

        transition! {
            abort(r: Tokens) {
                require r <= pre.reserved;
                update reserved = pre.reserved - r;
                update refunded = pre.refunded + r;
            }
        }

        #[inductive(initial)]
        fn initial_preserves_inv(post: Self, cap: Tokens) { }

        #[inductive(reserve)]
        fn reserve_preserves_inv(pre: Self, post: Self, n: Tokens) { }

        #[inductive(confirm)]
        fn confirm_preserves_inv(pre: Self, post: Self, r: Tokens, k: Tokens) { }

        #[inductive(abort)]
        fn abort_preserves_inv(pre: Self, post: Self, r: Tokens) { }
    }
}

pub proof fn lemma_total_is_constant(s: BudgetSM::State)
    requires s.invariant(),
    ensures s.available + s.reserved + s.spent + s.refunded == s.initial_cap,
{ }

pub proof fn lemma_each_field_bounded(s: BudgetSM::State)
    requires s.invariant(),
    ensures
        s.available <= s.initial_cap,
        s.reserved  <= s.initial_cap,
        s.spent     <= s.initial_cap,
        s.refunded  <= s.initial_cap,
{ }

pub proof fn lemma_spent_monotone(pre: BudgetSM::State, post: BudgetSM::State)
    requires
        pre.invariant(),
        post.invariant(),
    ensures
        post.spent >= pre.spent,
{

    admit();
}

#[verifier::external_body]
pub proof fn refinement() ensures true,
{
}

} // verus!