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
/// boarding tree. Updates `best_arrival[stop]` and marks the stop
/// in `out`. Returns `true` if any insertion happened.
///
/// Labels weakly dominated by the `pt_threshold` bag are rejected before
/// the bag insert: such a label cannot lead to a Pareto-optimal journey
/// at any target (a known better label already exists at some target),
/// so any boarding-tree entry it would produce is a ghost step that
/// reconstruction would later surface as a dominated journey. The route-
/// scan inner loop applies the same Pareto-aware guard; without it here
/// the footpath-relax path silently re-introduces dominated journeys
/// whenever a multi-target query has one target reachable via a faster
/// walk-only path than another target's trip-based arrival.
#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_into_bag<L: Label>(
    labels: &mut [Vec<LabelBag<L>>],
    best_arrival: &mut [LabelBag<L>],
    board_detail: &mut BoardingTree,
    out: &mut Vec<StopIdx>,
    ever_reached: &mut FixedBitSet,
    pt_threshold: &LabelBag<L>,
    k: K,
    stop: StopIdx,
    label: L,
    step: Step,
) -> bool {
    if pt_threshold.dominates_label(&label) {
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
