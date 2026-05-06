//! Per-call RAPTOR driver: [`run_per_call_query`] is the entry point
//! invoked by `Query::run_with_cache`; [`run_raptor_rounds`] is the
//! shared rounds loop reused by the rRAPTOR scan.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use fixedbitset::FixedBitSet;
use smallvec::SmallVec;

use crate::RaptorCache;
use crate::Timetable;
use crate::algorithm::boarding::BoardingTree;
use crate::algorithm::boarding::Step;
use crate::algorithm::boarding::best_to_any_target;
use crate::algorithm::boarding::extract_target_journeys;
use crate::algorithm::footpaths::relax_footpaths_round;
use crate::algorithm::footpaths::relax_footpaths_round_closed;
use crate::algorithm::label_bag::LabelBag;
use crate::endpoints::IntoEndpoints;
use crate::ids::RouteIdx;
use crate::ids::StopIdx;
use crate::ids::TripIdx;
use crate::journey::Journey;
use crate::label::Label;
use crate::time::Duration;
use crate::time::SecondOfDay;

// `earliest_accessible_trip` lives on the `Timetable` trait now so
// adapters with direct access to per-route trip lists can walk by
// trip-index instead of by time, avoiding the simultaneous-departure
// leapfrog bug. See `Timetable::earliest_accessible_trip`.

/// Run round-0 footpath relaxation followed by rounds `1..=transfers`
/// against an already-seeded cache. Shared by the per-call algorithm
/// (which seeds a single τ) and the rRAPTOR scan (which re-seeds for
/// each τ in descending order); the only difference between callers
/// is the outer τ-loop that handles seeding.
///
/// **Caller invariants on entry:**
///   - `marked_stops` contains the round-0 starting set: at minimum
///     the sources, plus (for rRAPTOR) any stops where new trips just
///     became catchable at the current τ.
///   - `labels[0]` is seeded at every source with that source's
///     departure label; the corresponding `best_arrival` and
///     `ever_reached` bits are set.
///   - `best_arrival` and `ever_reached` are coherent with `labels`
///     (the helper reads and writes both).
///
/// On exit, `marked_stops` is empty (drained by the early-out check
/// or by the natural marked-stops migration through rounds).
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_raptor_rounds<T: Timetable + ?Sized, L: Label>(
    tt: &T,
    ctx: &L::Ctx,
    labels: &mut [Vec<LabelBag<L>>],
    best_arrival: &mut [LabelBag<L>],
    board_detail: &mut BoardingTree,
    marked_stops: &mut FixedBitSet,
    q_entry: &mut [Option<u32>],
    q_routes: &mut Vec<RouteIdx>,
    walked_buf: &mut Vec<StopIdx>,
    relax_heap: &mut BinaryHeap<Reverse<(SecondOfDay, u32)>>,
    ever_reached: &mut FixedBitSet,
    transfers: usize,
    require_wheelchair_accessible: bool,
    targets: &[(StopIdx, Duration)],
) {
    let mut pt_threshold = best_to_any_target(best_arrival, targets);

    // Pick the per-round footpath relaxation strategy once. Closed
    // graphs use a single-pass O(E) walk; non-closed graphs need
    // multi-source Dijkstra to chain walks to a fixed point.
    let footpaths_closed = tt.footpaths_are_transitively_closed();

    if footpaths_closed {
        relax_footpaths_round_closed(
            tt,
            ctx,
            0,
            labels,
            best_arrival,
            board_detail,
            marked_stops,
            pt_threshold,
            walked_buf,
            ever_reached,
        );
    } else {
        relax_footpaths_round(
            tt,
            ctx,
            0,
            labels,
            best_arrival,
            board_detail,
            marked_stops,
            pt_threshold,
            walked_buf,
            relax_heap,
            ever_reached,
        );
    }
    for s in walked_buf.drain(..) {
        marked_stops.insert(s.idx());
    }
    pt_threshold = best_to_any_target(best_arrival, targets);

    for k in 1..=transfers {
        // Sparse carry-forward: only clone bags at stops ever
        // reached so far this query. For real feeds the reached
        // set is small (<10% of stops in typical city queries),
        // so this avoids the 50k-stop dense memcpy that would
        // otherwise dominate per-round overhead.
        let (prev_labels, this_labels) = labels.split_at_mut(k);
        let src = &prev_labels[k - 1];
        let dst = &mut this_labels[0];
        for bit in ever_reached.ones() {
            dst[bit] = src[bit].clone();
        }

        // Build the route queue for this round. Each entry pairs a
        // route with the earliest position on that route from which we
        // can board this round. Stored positions are folded with `min`
        // so multiple marked stops on the same route resolve to the
        // earliest one.
        for stop_bit in marked_stops.ones() {
            let marked_stop = StopIdx::new(stop_bit as u32);
            for &(route, pos) in tt.get_routes_serving_stop(marked_stop) {
                match q_entry[route.idx()] {
                    None => {
                        q_entry[route.idx()] = Some(pos);
                        q_routes.push(route);
                    }
                    Some(prev_pos) => {
                        if pos < prev_pos {
                            q_entry[route.idx()] = Some(pos);
                        }
                    }
                }
            }
        }

        marked_stops.clear();

        // Per-route Pareto bag of riding entries. Each entry =
        // (boarding_label, current_trip, boarding_stop, board_pos); a
        // single entry suffices for `ArrivalTime` (size-1 stop bags)
        // but multi-criterion impls may carry multiple Pareto-optimal
        // entries that boarded different trips. `board_pos` is
        // threaded through so labels with per-trip context (e.g. fares)
        // can know where the rider got on.
        let mut route_bag: SmallVec<[(L, TripIdx, StopIdx, u32); 8]> = SmallVec::new();
        let mut staged: SmallVec<[L; 8]> = SmallVec::new();

        for &route in q_routes.iter() {
            let p_pos = q_entry[route.idx()].expect("route in q_routes must have an entry");
            route_bag.clear();

            for (offset, &pi) in tt.get_stops_after(route, p_pos).iter().enumerate() {
                let pos = p_pos + offset as u32;

                // 1. Alight every active riding entry at pi.
                let stop_accessible_ok =
                    !require_wheelchair_accessible || tt.stop_wheelchair_accessible(pi);
                for &(boarding_label, trip, boarding_stop, board_pos) in route_bag.iter() {
                    // Skip stops where this trip doesn't allow drop-off
                    // (GTFS drop_off_type = 1). The trip still passes
                    // through `pi`, but the rider cannot disembark, so
                    // we don't update arrival labels here.
                    if !tt.drop_off_allowed(trip, pos) {
                        continue;
                    }
                    // Same skip when the query requires wheelchair
                    // access and `pi` is marked inaccessible.
                    if !stop_accessible_ok {
                        continue;
                    }
                    let arr = tt.get_arrival_time(trip, pos);
                    let best_to_pi = best_arrival[pi.idx()].min_arrival();
                    let time_to_beat = best_to_pi.min(pt_threshold);
                    if arr >= time_to_beat {
                        continue;
                    }
                    let new_label = boarding_label.extend_by_trip(
                        ctx,
                        crate::label::TripContext {
                            trip,
                            route,
                            board_stop: boarding_stop,
                            board_pos,
                            alight_stop: pi,
                            alight_pos: pos,
                            arrival: arr,
                        },
                    );
                    if labels[k][pi.idx()].insert(new_label) {
                        best_arrival[pi.idx()].insert(new_label);
                        board_detail.insert(
                            (k, pi, new_label.arrival()),
                            Step::Boarded {
                                from: boarding_stop,
                                route,
                                parent_arrival: boarding_label.arrival(),
                            },
                        );
                        marked_stops.insert(pi.idx());
                        ever_reached.insert(pi.idx());
                    }
                }

                // 2. Try to extend route_bag with labels from
                //    labels[k-1][pi] that can catch a trip on this
                //    route at pi. Snapshot first to avoid aliasing.
                staged.clear();
                staged.extend(labels[k - 1][pi.idx()].iter().copied());
                for candidate in &staged {
                    let cand_arr = candidate.arrival();
                    let trip = match tt.earliest_accessible_trip(
                        route,
                        cand_arr,
                        pos,
                        require_wheelchair_accessible,
                    ) {
                        Some(t) => t,
                        None => continue,
                    };
                    let trip_dep = tt.get_departure_time(trip, pos);

                    // Redundancy check: existing route_bag entry
                    // dominates candidate AND boards an at-or-earlier
                    // trip at pi → candidate is redundant.
                    let mut redundant = false;
                    for &(l_existing, t_existing, _, _) in route_bag.iter() {
                        let existing_dep = tt.get_departure_time(t_existing, pos);
                        if l_existing.dominates(candidate) && existing_dep <= trip_dep {
                            redundant = true;
                            break;
                        }
                    }
                    if redundant {
                        continue;
                    }

                    // Remove entries strictly dominated by candidate.
                    route_bag.retain(|&mut (l_existing, t_existing, _, _)| {
                        let existing_dep = tt.get_departure_time(t_existing, pos);
                        !(candidate.dominates(&l_existing) && trip_dep <= existing_dep)
                    });
                    route_bag.push((*candidate, trip, pi, pos));
                }
            }
        }

        // Sparse-set reset of the route queue.
        for r in q_routes.drain(..) {
            q_entry[r.idx()] = None;
        }

        // Refresh target threshold after the route scan, then run
        // footpath relax. Refresh again after the footpath round so
        // the next round's boarding decisions use a current threshold.
        pt_threshold = best_to_any_target(best_arrival, targets);
        if footpaths_closed {
            relax_footpaths_round_closed(
                tt,
                ctx,
                k,
                labels,
                best_arrival,
                board_detail,
                marked_stops,
                pt_threshold,
                walked_buf,
                ever_reached,
            );
        } else {
            relax_footpaths_round(
                tt,
                ctx,
                k,
                labels,
                best_arrival,
                board_detail,
                marked_stops,
                pt_threshold,
                walked_buf,
                relax_heap,
                ever_reached,
            );
        }
        for s in walked_buf.drain(..) {
            marked_stops.insert(s.idx());
        }
        pt_threshold = best_to_any_target(best_arrival, targets);

        if marked_stops.is_clear() {
            break;
        }
    }
}

/// Per-call RAPTOR query: seed origins at `depart`, run rounds, then
/// reconstruct the Pareto front of journeys at the targets.
///
/// Invoked by `Query::run_with_cache` for single-departure queries and
/// by the parallel rRAPTOR path for each fan-out departure.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_per_call_query<T: Timetable + ?Sized, L: Label>(
    tt: &T,
    ctx: &L::Ctx,
    cache: &mut RaptorCache<L>,
    transfers: usize,
    depart: SecondOfDay,
    origins: impl IntoEndpoints,
    targets: impl IntoEndpoints,
    require_wheelchair_accessible: bool,
) -> Vec<Journey<L>> {
    let origins = origins.into_endpoints();
    let targets = targets.into_endpoints();
    let origins = origins.as_slice();
    let targets = targets.as_slice();
    cache.reset_for_query(transfers, tt.n_stops() as u32, tt.n_routes() as u32);
    let RaptorCache {
        labels,
        best_arrival,
        board_detail,
        marked_stops,
        q_entry,
        q_routes,
        walked_buf,
        origin_set,
        relax_heap,
        ever_reached,
        ..
    } = cache;

    // Clear and populate the origin set used by reconstruction.
    origin_set.clear();
    for &(o, _) in origins {
        origin_set.insert(o.idx());
    }

    // Seed labels for each origin at depart + its walk-time offset.
    // Reconstruction breaks the trace loop when it hits an origin
    // (origin_set bit is set), so origins don't need a Step entry.
    for &(o, walk) in origins {
        let t = depart + walk;
        let seed = L::from_departure(ctx, t);
        if labels[0][o.idx()].insert(seed) {
            best_arrival[o.idx()].insert(seed);
            marked_stops.insert(o.idx());
            ever_reached.insert(o.idx());
        }
    }

    run_raptor_rounds(
        tt,
        ctx,
        labels,
        best_arrival,
        board_detail,
        marked_stops,
        q_entry,
        q_routes,
        walked_buf,
        relax_heap,
        ever_reached,
        transfers,
        require_wheelchair_accessible,
        targets,
    );

    // For each target stop, enumerate every Pareto-optimal label
    // in the target's bag at every round and reconstruct its plan.
    // Effective label = bag label extended by the target's walk
    // time offset.
    let mut journeys =
        extract_target_journeys(ctx, labels, board_detail, origin_set, targets, transfers);

    // Output-side Pareto filter on (trip count, label). For any two
    // returned journeys neither weakly dominates the other on the
    // pair (plan.len, label). For single-criterion `ArrivalTime`
    // this collapses to "strictly decreasing arrival as trip count
    // increases" (the v0.10 contract); for multi-criterion impls
    // it preserves Pareto-incomparable journeys (e.g. faster but
    // more walking vs. slower but less walking).
    //
    // Sorted by plan.len ascending, then by `arrival()` ascending
    // as a tiebreaker so the iteration order is deterministic.
    journeys.sort_by_key(|j| (j.plan.len(), j.arrival()));
    let mut front: Vec<Journey<L>> = Vec::with_capacity(journeys.len());
    'outer: for j in journeys {
        for f in &front {
            if f.plan.len() <= j.plan.len() && f.label.dominates(&j.label) {
                continue 'outer;
            }
        }
        front.retain(|f| !(j.plan.len() <= f.plan.len() && j.label.dominates(&f.label)));
        front.push(j);
    }
    front
}
