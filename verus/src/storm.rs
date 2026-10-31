// Apiary — the verified core, part two: the storm never knocks on an empty hive.
//
//   verus verus/src/storm.rs
//
// The storm runs against a fleet configured with `create_on_demand = false`,
// so touching a hive whose file does not exist is a 404. `expand` in
// `crates/hive/src/storm.rs` turns a stream of (hive, write?) picks into ops
// and inserts a `Provision` the first time it meets a hive. This file is that
// function, and two properties of its output:
//
//   provisions_first — every Write or Read on hive h comes after a
//                      Provision of h;
//   provisioned_once — no hive is provisioned twice (a second provision is
//                      a 409 from the fleet).
//
// plus: every op names a hive inside the population.
//
// ── The modelling assumptions ───────────────────────────────────────────────
//
// 1. `Op` here uses named fields (`Provision { hive }`) where the runtime uses
//    a tuple variant (`Provision(u32)`). Same information, easier quantifiers.
// 2. The runtime numbers each hive's writes with `saturating_add(1)`; here it
//    is the equivalent explicit `if w < u32::MAX { w + 1 } else { w }`. Nothing
//    below depends on the numbering.
// 3. Where the picks come from (the seeded xorshift) is not modelled: the
//    theorem holds for ANY picks, which is stronger.

use vstd::prelude::*;

verus! {

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Provision { hive: u32 },
    Write { hive: u32, seq: u32 },
    Read { hive: u32 },
}

pub open spec fn hive_of(op: Op) -> u32 {
    match op {
        Op::Provision { hive } => hive,
        Op::Write { hive, seq } => hive,
        Op::Read { hive } => hive,
    }
}

pub open spec fn is_provision(op: Op) -> bool {
    match op {
        Op::Provision { hive } => true,
        _ => false,
    }
}

/// SPEC 4.1.
pub open spec fn provisions_first(ops: Seq<Op>) -> bool {
    forall|j: int|
        #![trigger ops[j]]
        0 <= j < ops.len() && !is_provision(ops[j]) ==> exists|k: int|
            #![trigger ops[k]]
            0 <= k < j && is_provision(ops[k]) && hive_of(ops[k]) == hive_of(ops[j])
}

/// SPEC 4.2.
pub open spec fn provisioned_once(ops: Seq<Op>) -> bool {
    forall|i: int, j: int|
        #![trigger ops[i], ops[j]]
        0 <= i < j < ops.len() && is_provision(ops[i]) && is_provision(ops[j]) ==> hive_of(
            ops[i],
        ) != hive_of(ops[j])
}

pub open spec fn in_population(ops: Seq<Op>, hives: u32) -> bool {
    forall|j: int| #![trigger ops[j]] 0 <= j < ops.len() ==> hive_of(ops[j]) < hives
}

fn filled<T: Copy>(n: usize, value: T) -> (v: Vec<T>)
    ensures
        v@.len() == n,
        forall|i: int| 0 <= i < n ==> v@[i] == value,
{
    let mut v: Vec<T> = Vec::new();
    let mut i: usize = 0;
    while i < n
        invariant
            i <= n,
            v@.len() == i,
            forall|k: int| 0 <= k < i ==> v@[k] == value,
        decreases n - i,
    {
        v.push(value);
        i = i + 1;
    }
    v
}

/// SPEC 4.3. `expand`, as `crates/hive/src/storm.rs` writes it.
pub fn expand(picks: &Vec<(u32, bool)>, hives: u32) -> (out: Vec<Op>)
    ensures
        provisions_first(out@),
        provisioned_once(out@),
        in_population(out@, hives),
{
    let n = hives as usize;
    let mut provisioned: Vec<bool> = filled(n, false);
    let mut writes: Vec<u32> = filled(n, 0u32);
    let mut out: Vec<Op> = Vec::new();
    // first[h]: where hive h's Provision sits in `out`, once it has one.
    let ghost mut first: Seq<int> = Seq::new(n as nat, |h: int| -1int);
    let mut p: usize = 0;
    while p < picks.len()
        invariant
            n == hives as usize,
            provisioned@.len() == n,
            writes@.len() == n,
            first.len() == n,
            p <= picks.len(),
            // Every op is on a hive in the population.
            forall|j: int| #![trigger out@[j]] 0 <= j < out@.len() ==> hive_of(out@[j]) < hives,
            // A provisioned hive's Provision is where `first` says.
            forall|h: int|
                #![trigger provisioned@[h]]
                0 <= h < n && provisioned@[h] ==> 0 <= first[h] < out@.len() && is_provision(
                    out@[first[h]],
                ) && hive_of(out@[first[h]]) == h,
            // Every Provision is its hive's first, and its hive is marked.
            forall|j: int|
                #![trigger out@[j]]
                0 <= j < out@.len() && is_provision(out@[j]) ==> provisioned@[hive_of(
                    out@[j],
                ) as int] && first[hive_of(out@[j]) as int] == j,
            // Every touch comes after its hive's Provision.
            forall|j: int|
                #![trigger out@[j]]
                0 <= j < out@.len() && !is_provision(out@[j]) ==> provisioned@[hive_of(
                    out@[j],
                ) as int] && first[hive_of(out@[j]) as int] < j,
        decreases picks.len() - p,
    {
        let pick = picks[p];
        let hive = pick.0;
        let write = pick.1;
        let i = hive as usize;
        if i < n {
            if !provisioned[i] {
                proof {
                    first = first.update(i as int, out@.len() as int);
                }
                provisioned.set(i, true);
                out.push(Op::Provision { hive });
            }
            if write {
                let w = writes[i];
                let next = if w < u32::MAX {
                    w + 1
                } else {
                    w
                };
                writes.set(i, next);
                out.push(Op::Write { hive, seq: next });
            } else {
                out.push(Op::Read { hive });
            }
        }
        p = p + 1;
    }
    proof {
        assert forall|j: int|
            #![trigger out@[j]]
            0 <= j < out@.len() && !is_provision(out@[j]) implies exists|k: int|
            #![trigger out@[k]]
            0 <= k < j && is_provision(out@[k]) && hive_of(out@[k]) == hive_of(out@[j]) by {
            let h = hive_of(out@[j]) as int;
            let k = first[h];
            assert(0 <= k < j);
            assert(is_provision(out@[k]) && hive_of(out@[k]) == h);
        }
        assert forall|i: int, j: int|
            #![trigger out@[i], out@[j]]
            0 <= i < j < out@.len() && is_provision(out@[i]) && is_provision(
                out@[j],
            ) implies hive_of(out@[i]) != hive_of(out@[j]) by {
            if hive_of(out@[i]) == hive_of(out@[j]) {
                assert(first[hive_of(out@[i]) as int] == i);
                assert(first[hive_of(out@[j]) as int] == j);
            }
        }
    }
    out
}

fn main() {
}

} // verus!
