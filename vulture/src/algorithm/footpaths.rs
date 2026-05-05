//! Per-round footpath relaxation: [`relax_footpaths_round`] for the
//! general case (multi-source Dijkstra to a fixed point) and
//! [`relax_footpaths_round_closed`] for adapters whose relation is
//! already transitively closed (single-pass O(E)).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use fixedbitset::FixedBitSet;
use smallvec::SmallVec;

use crate::K;
use crate::Timetable;
use crate::algorithm::boarding::BoardingTree;
use crate::algorithm::boarding::Step;
use crate::algorithm::label_bag::LabelBag;
use crate::algorithm::label_bag::insert_into_bag;
use crate::ids::StopIdx;
use crate::label::Label;
use crate::time::SecondOfDay;

/// Single-pass relaxation for adapters that report `true` from
/// [`Timetable::footpaths_are_transitively_closed`]. One walk per
/// source – chained walks are unnecessary because every reachable
/// pair is already a direct edge in the relation.
///
/// `O(E)` per round, no heap. Always sound when the closure
/// precondition holds; produces the same labels and boarding tree as
/// [`relax_footpaths_round`] in that case.
#[allow(clippy::too_many_arguments)]
pub(crate) fn relax_footpaths_round_closed<T: Timetable + ?Sized, L: Label>(
    timetable: &T,
    k: K,
    labels: &mut [Vec<LabelBag<L>>],
    best_arrival: &mut [LabelBag<L>],
    board_detail: &mut BoardingTree,
    sources: &FixedBitSet,
    pt_threshold: SecondOfDay,
    out: &mut Vec<StopIdx>,
    ever_reached: &mut FixedBitSet,
) {
    // Walk each source bag's labels once. With closure, chained walks
    // are unnecessary because every reachable pair is already a direct
    // edge in the relation.
    let mut staged: SmallVec<[L; 8]> = SmallVec::new();
    for stop_bit in sources.ones() {
        let stop = StopIdx::new(stop_bit as u32);
        if labels[k][stop.idx()].is_empty() {
            continue;
        }
        // Snapshot the source bag so we don't iterate while mutating
        // labels[k] (which can include the source itself).
        staged.clear();
        staged.extend(labels[k][stop.idx()].iter().copied());

        for &p_dash in timetable.get_footpaths_from(stop) {
            let walk = timetable.get_transfer_time(stop, p_dash);
            for source_label in &staged {
                let via_walk = source_label.extend_by_footpath(walk);
                insert_into_bag(
                    labels,
                    best_arrival,
                    board_detail,
                    out,
                    ever_reached,
                    pt_threshold,
                    k,
                    p_dash,
                    via_walk,
                    Step::Walked {
                        from: stop,
                        parent_arrival: source_label.arrival(),
                    },
                );
            }
        }
    }
}

/// Relax footpaths from every stop in `sources` at round `k`, propagating
/// to a fixed point.
///
/// Iteratively improves `labels[k][p_dash]` and the τ\* table
/// (`best_arrival`) until no walk produces a strictly better arrival.
/// Each improvement records a `Walked` step in the boarding tree so that
/// `reconstruct_journey` can later trace back through the walk leg.
///
/// Because relaxation iterates to a fixed point, the trait's footpath
/// relation does **not** need to be transitively closed: if A→B and B→C
/// are both in the relation, the algorithm chains them within a single
/// round and reaches C with the combined walk time.
///
/// Stops that should be added to the marked set for the next round are
/// pushed onto `out`, gated by `pt_threshold` (the current best effective
/// arrival at any target). Caller drains `out` between calls.
#[allow(clippy::too_many_arguments)]
pub(crate) fn relax_footpaths_round<T: Timetable + ?Sized, L: Label>(
    timetable: &T,
    k: K,
    labels: &mut [Vec<LabelBag<L>>],
    best_arrival: &mut [LabelBag<L>],
    board_detail: &mut BoardingTree,
    sources: &FixedBitSet,
    pt_threshold: SecondOfDay,
    out: &mut Vec<StopIdx>,
    heap: &mut BinaryHeap<Reverse<(SecondOfDay, u32)>>,
    ever_reached: &mut FixedBitSet,
) {
    // Multi-source Dijkstra over the footpath graph at round `k`,
    // ordered by `LabelBag::min_arrival()`. Each source's initial
    // priority is its current bag's minimum arrival; transfer times
    // are non-negative so Dijkstra is sound. Uses lazy deletion —
    // stale heap entries are skipped on pop.
    //
    // For multi-criterion bags this propagates only the min-arrival
    // label per stop; non-min labels in a source bag don't drive
    // further relaxation. This is sound for arrival-time pruning but
    // may miss Pareto-optimal walk chains where a non-min-arrival
    // label dominates downstream – a v0.12 concern.
    heap.clear();
    for bit in sources.ones() {
        let min_arr = labels[k][bit].min_arrival();
        if min_arr != SecondOfDay::MAX {
            heap.push(Reverse((min_arr, bit as u32)));
        }
    }

    let mut staged: SmallVec<[L; 8]> = SmallVec::new();
    while let Some(Reverse((arrival, stop_bit))) = heap.pop() {
        let stop = StopIdx::new(stop_bit);
        let stop_min = labels[k][stop.idx()].min_arrival();
        // Skip stale entries – a strictly better label was popped earlier.
        if arrival > stop_min {
            continue;
        }
        staged.clear();
        staged.extend(labels[k][stop.idx()].iter().copied());

        for &p_dash in timetable.get_footpaths_from(stop) {
            let walk = timetable.get_transfer_time(stop, p_dash);
            let mut any_added = false;
            for source_label in &staged {
                let via_walk = source_label.extend_by_footpath(walk);
                if insert_into_bag(
                    labels,
                    best_arrival,
                    board_detail,
                    out,
                    ever_reached,
                    pt_threshold,
                    k,
                    p_dash,
                    via_walk,
                    Step::Walked {
                        from: stop,
                        parent_arrival: source_label.arrival(),
                    },
                ) {
                    any_added = true;
                }
            }
            if any_added {
                heap.push(Reverse((
                    labels[k][p_dash.idx()].min_arrival(),
                    p_dash.get(),
                )));
            }
        }
    }
}
