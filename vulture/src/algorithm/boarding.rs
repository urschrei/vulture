//! Boarding-tree types ([`Step`], [`BoardingTree`]) and the journey
//! reconstruction helpers that walk them ([`reconstruct_journey`],
//! [`extract_target_journeys`], [`best_to_any_target`]).

use std::collections::BTreeMap;

use fixedbitset::FixedBitSet;

use crate::K;
use crate::algorithm::label_bag::LabelBag;
use crate::ids::RouteIdx;
use crate::ids::StopIdx;
use crate::journey::Journey;
use crate::label::Label;
use crate::time::Duration;
use crate::time::SecondOfDay;

/// One reconstructable step in a journey: either a transit boarding event
/// (route-scan) or a walk along a footpath. Walks do not consume a round —
/// they happen *within* round `k` at the stop they alight on – so the
/// reconstruction logic chains through walk entries without decrementing
/// the round index.
///
/// `parent_arrival` disambiguates which Pareto-optimal label at the
/// parent stop (in the parent stop's bag) was extended to produce
/// the label this step belongs to. For single-criterion `ArrivalTime`
/// the parent bag is size-1, so this field is redundant; for
/// multi-criterion impls it lets reconstruction follow the right
/// label through the bag.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Step {
    Boarded {
        from: StopIdx,
        route: RouteIdx,
        parent_arrival: SecondOfDay,
    },
    Walked {
        from: StopIdx,
        parent_arrival: SecondOfDay,
    },
}

/// Boarding tree key: `(round, stop, label_arrival)`. The third
/// component disambiguates Pareto-optimal labels with distinct
/// arrival times in the same `(round, stop)` bag.
pub(crate) type BoardingTree = BTreeMap<(K, StopIdx, SecondOfDay), Step>;

/// Returns the minimum of `best_arrival[t].min_arrival() + w` across
/// all `(t, w)` in `targets`, saturating on overflow. Returns
/// `SecondOfDay::MAX` if every target is unreached.
pub(crate) fn best_to_any_target<L: Label>(
    best_arrival: &[LabelBag<L>],
    targets: &[(StopIdx, Duration)],
) -> SecondOfDay {
    targets
        .iter()
        .map(|&(t, w)| best_arrival[t.idx()].min_arrival() + w)
        .min()
        .unwrap_or(SecondOfDay::MAX)
}

/// Reconstruct a single candidate plan terminating at the target
/// label `(pt, target_arrival)` at round `k`. Traces back through
/// the boarding tree (which is keyed on `(round, stop, label_arrival)`
/// to disambiguate Pareto-optimal labels in the same bag). Returns
/// `Some((origin, plan))` if the trace reaches some stop in `origins`,
/// `None` otherwise.
pub(crate) fn reconstruct_journey(
    tree: &BoardingTree,
    origins: &FixedBitSet,
    pt: StopIdx,
    target_arrival: SecondOfDay,
    k: K,
) -> Option<(StopIdx, Vec<(RouteIdx, StopIdx)>)> {
    if tree.is_empty() {
        return None;
    }

    let mut plan = Vec::with_capacity(k);
    let mut parent = pt;
    let mut parent_arrival = target_arrival;
    let mut inner_k = k;
    // Defensive bound to avoid pathological loops; 100 walk-hops per
    // round is well beyond anything realistic.
    let mut budget = (k + 1) * 100;

    while !origins.contains(parent.idx()) && budget > 0 {
        budget -= 1;

        let Some(step) = tree.get(&(inner_k, parent, parent_arrival)).copied() else {
            break;
        };

        match step {
            Step::Boarded {
                from,
                route,
                parent_arrival: pa,
            } => {
                plan.push((route, parent));
                parent = from;
                parent_arrival = pa;
                if inner_k == 0 {
                    break;
                }
                inner_k -= 1;
            }
            Step::Walked {
                from,
                parent_arrival: pa,
            } => {
                parent = from;
                parent_arrival = pa;
                // walks do not consume a round
            }
        }
    }

    if !plan.is_empty() && origins.contains(parent.idx()) {
        plan.reverse();
        Some((parent, plan))
    } else {
        None
    }
}

/// Reconstruct one Pareto-front-of-trip-counts worth of journeys
/// for the given targets at the current cache state. Used by both
/// the per-call algorithm (after running rounds) and the rRAPTOR
/// scan (snapshot at end of each τ scan).
///
/// Walks each target × round × label-bag entry, traces back via
/// `reconstruct_journey`, and applies the target's walk-time offset.
/// Output is unfiltered (caller applies any Pareto front filtering).
pub(crate) fn extract_target_journeys<L: Label>(
    labels: &[Vec<LabelBag<L>>],
    board_detail: &BoardingTree,
    origin_set: &FixedBitSet,
    targets: &[(StopIdx, Duration)],
    transfers: usize,
) -> Vec<Journey<L>> {
    let mut journeys: Vec<Journey<L>> = Vec::new();
    for &(target, walk) in targets {
        #[allow(clippy::needless_range_loop)]
        for k in 1..=transfers {
            for raw_label in labels[k][target.idx()].iter() {
                let raw_arr = raw_label.arrival();
                if raw_arr == SecondOfDay::MAX {
                    continue;
                }
                let Some((origin, plan)) =
                    reconstruct_journey(board_detail, origin_set, target, raw_arr, k)
                else {
                    continue;
                };
                let label = raw_label.extend_by_footpath(walk);
                journeys.push(Journey {
                    origin,
                    target,
                    plan,
                    label,
                });
            }
        }
    }
    journeys
}
