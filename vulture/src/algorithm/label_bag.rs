//! [`LabelBag`] – Pareto front of [`Label`]s at a single `(round, stop)`
//! cell – and the [`insert_into_bag`] free helper used by the per-round
//! footpath relaxation routines.

use fixedbitset::FixedBitSet;
use smallvec::SmallVec;

use crate::K;
use crate::algorithm::boarding::BoardingTree;
use crate::algorithm::boarding::Step;
use crate::ids::StopIdx;
use crate::label::Label;
use crate::time::SecondOfDay;

/// A Pareto front of [`Label`]s at a single `(round, stop)` cell.
/// Backed by `SmallVec<[L; 8]>` – for single-criterion `ArrivalTime`
/// the bag is always size 1 and stays inline; for multi-criterion
/// impls it grows up to 8 inline before spilling.
#[derive(Debug, Clone)]
pub(crate) struct LabelBag<L: Label> {
    items: SmallVec<[L; 8]>,
}

impl<L: Label> LabelBag<L> {
    pub(crate) fn new() -> Self {
        Self {
            items: SmallVec::new(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &L> {
        self.items.iter()
    }

    /// Try to insert `new`. Returns `true` if added (and removes any
    /// items it strictly dominates); returns `false` if some existing
    /// item weakly dominates `new` (no change).
    pub(crate) fn insert(&mut self, new: L) -> bool {
        for item in &self.items {
            if item.dominates(&new) {
                return false;
            }
        }
        self.items.retain(|item| !new.dominates(item));
        self.items.push(new);
        true
    }

    /// Minimum `arrival()` across the bag, or `SecondOfDay::MAX` if empty.
    pub(crate) fn min_arrival(&self) -> SecondOfDay {
        self.items
            .iter()
            .map(|l| l.arrival())
            .min()
            .unwrap_or(SecondOfDay::MAX)
    }

    /// Returns `true` if any item in the bag weakly dominates `candidate`.
    /// Equivalent to "this candidate would be rejected by `insert` because
    /// the bag already has something at least as good". Used by the
    /// algorithm's pruning sites to skip exploring a label that is already
    /// dominated by a known better one, avoiding the cost of `extend_*` /
    /// boarding-tree insertion. For single-criterion `ArrivalTime` the bag
    /// is size 1 and this reduces to one `<=` comparison; for multi-
    /// criterion impls the comparison is Pareto-aware via `Label::dominates`.
    pub(crate) fn dominates_label(&self, candidate: &L) -> bool {
        self.items.iter().any(|item| item.dominates(candidate))
    }

    pub(crate) fn clear(&mut self) {
        self.items.clear();
    }
}

impl<L: Label> Default for LabelBag<L> {
    fn default() -> Self {
        Self::new()
    }
}

/// Try to insert `(label, step)` into `labels[k][stop]` and the
/// boarding tree. Updates `best_arrival[stop]` and marks the stop in
/// `out`. Returns `true` if any insertion happened.
///
/// Labels are rejected before the bag insert when *every* target's
/// `best_arrival` bag already weakly dominates the candidate: such a
/// label cannot lead to a Pareto-optimal journey at any target slot,
/// so any boarding-tree entry it would produce is a ghost step that
/// reconstruction would later surface as a dominated journey.
///
/// Crucially the check uses an *all-targets* quantifier, not the older
/// scalar "min effective" cross-target threshold. With per-target output
/// semantics, a label that is dominated by one target's best is still
/// worth keeping if it could lead to a non-dominated journey at *some
/// other* target (e.g. a walking-footpath label at an intermediate stop
/// that ends up at a target the label-emitter couldn't reach via
/// transit). The `target_walk` extension cancels out of the dominance
/// comparison (extending both sides by the same `target_walk` preserves
/// the dominance relation), so the check operates on raw labels against
/// raw `best_arrival` bags.
#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_into_bag<L: Label>(
    labels: &mut [Vec<LabelBag<L>>],
    best_arrival: &mut [LabelBag<L>],
    board_detail: &mut BoardingTree,
    out: &mut Vec<StopIdx>,
    ever_reached: &mut FixedBitSet,
    targets: &[(StopIdx, crate::time::Duration)],
    k: K,
    stop: StopIdx,
    label: L,
    step: Step,
) -> bool {
    if all_targets_dominate(best_arrival, targets, &label) {
        return false;
    }
    let added = labels[k][stop.idx()].insert(label);
    if !added {
        return false;
    }
    board_detail.insert((k, stop, label.arrival()), step);
    best_arrival[stop.idx()].insert(label);
    ever_reached.insert(stop.idx());
    out.push(stop);
    true
}

/// Returns `true` iff every target slot's `best_arrival` bag weakly
/// dominates `label`. The per-target pruning predicate used by the
/// algorithm's insert sites — see [`insert_into_bag`] for context.
/// Empty `targets` returns `false` (no targets means no slots dominate,
/// so the label is not pruned).
pub(crate) fn all_targets_dominate<L: Label>(
    best_arrival: &[LabelBag<L>],
    targets: &[(StopIdx, crate::time::Duration)],
    label: &L,
) -> bool {
    !targets.is_empty()
        && targets
            .iter()
            .all(|&(t, _)| best_arrival[t.idx()].dominates_label(label))
}
