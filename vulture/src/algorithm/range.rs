//! Range-query specialisation: rRAPTOR scan
//! ([`raptor_range_rrap_arrival`]) plus its Pareto-front filter
//! ([`filter_range_pareto_front`]) and the helper that marks stops
//! whose trips just became catchable in a new τ window
//! ([`newly_active_stops_into`]).

use fixedbitset::FixedBitSet;

use crate::RangeJourney;
use crate::RaptorCache;
use crate::Timetable;
use crate::algorithm::boarding::extract_target_journeys;
use crate::algorithm::per_call::run_raptor_rounds;
use crate::endpoints::Endpoints;
use crate::ids::RouteIdx;
use crate::label::ArrivalTime;
use crate::label::Label;
use crate::time::SecondOfDay;

/// Pareto-profile filter for range-query output: keep entries that are
/// not weakly dominated by another on `(later depart, fewer transfers,
/// dominated label)`. Sort so the entries we'd prefer to keep come
/// first (later departure, fewer transfers, earlier arrival), then
/// sweep with mutual-domination removal.
pub(crate) fn filter_range_pareto_front<L: Label>(
    mut all: Vec<RangeJourney<L>>,
) -> Vec<RangeJourney<L>> {
    all.sort_by(|a, b| {
        b.depart
            .cmp(&a.depart)
            .then(a.journey.plan.len().cmp(&b.journey.plan.len()))
            .then(a.journey.arrival().cmp(&b.journey.arrival()))
    });
    let mut front: Vec<RangeJourney<L>> = Vec::with_capacity(all.len());
    'outer: for r in all {
        for f in &front {
            if f.depart >= r.depart
                && f.journey.plan.len() <= r.journey.plan.len()
                && f.journey.label.dominates(&r.journey.label)
            {
                continue 'outer;
            }
        }
        front.retain(|f| {
            !(r.depart >= f.depart
                && r.journey.plan.len() <= f.journey.plan.len()
                && r.journey.label.dominates(&f.journey.label))
        });
        front.push(r);
    }
    front
}

/// Mark stops where any route through them has a trip whose departure
/// time at that position falls in `[lo, hi)`. Used by rRAPTOR between
/// τ scans to find stops that need rescanning because new trips just
/// became catchable.
///
/// `marked` is the destination bitset; bits already set are preserved.
/// One `get_earliest_trip` lookup per (route, position) plus one
/// `get_departure_time` if the lookup hits – overall
/// O(n_routes × max_route_len) calls per invocation, each O(log
/// n_trips_per_route) inside the trait impl.
pub(crate) fn newly_active_stops_into<T: Timetable + ?Sized>(
    tt: &T,
    lo: SecondOfDay,
    hi: SecondOfDay,
    marked: &mut FixedBitSet,
) {
    if lo >= hi {
        return;
    }
    let n_routes = tt.n_routes() as u32;
    for r in 0..n_routes {
        let route = RouteIdx::new(r);
        let stops = tt.get_stops_after(route, 0);
        for (pos_offset, &stop) in stops.iter().enumerate() {
            let pos = pos_offset as u32;
            if let Some(trip) = tt.get_earliest_trip(route, lo, pos)
                && tt.get_departure_time(trip, pos) < hi
            {
                marked.insert(stop.idx());
            }
        }
    }
}

/// Range-query algorithm specialised for `Label = ArrivalTime`. Single
/// reverse-chronological scan that reuses labels across departure
/// events (rRAPTOR, paper §4). For non-`ArrivalTime` labels, the
/// parallel paths (`Query::run_par` / `Query::run_with_pool`)
/// still cover this case via the naïve batch.
///
/// `departures` must be sorted descending and deduped (the
/// `.depart_in_window(...)` builder normalises this).
///
/// **Why descending τ order:** for τ' < τ_prev, a journey valid at
/// τ_prev is also valid at τ' (the same trip with depart ≥ τ_prev ≥
/// τ' is still catchable). So descending order makes each new seed
/// monotonically improve the bag, and no improvement ever needs to
/// be undone.
///
/// **Why labels accumulate (cache reset only once):** the per-call
/// algorithm resets between queries; rRAPTOR resets only at the start
/// of the whole scan. Labels written at scan τ remain valid for any
/// τ' < τ, so persisting them across scans is correct, not stale —
/// and avoids re-deriving information already discovered.
///
/// **Why `best_arrival` accumulates too:** the per-round carry-forward
/// overwrites `labels[k][X]` at the start of each round k, but
/// `best_arrival` is never cleared. This gives the `pt_threshold`
/// pruning a tight bound across τ scans.
pub(crate) fn raptor_range_rrap_arrival<T: Timetable + ?Sized>(
    tt: &T,
    cache: &mut RaptorCache<ArrivalTime>,
    transfers: usize,
    departures: &[SecondOfDay],
    origins: Endpoints,
    targets: Endpoints,
) -> Vec<RangeJourney<ArrivalTime>> {
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

    // origin_set is constant across τ scans; populate once.
    origin_set.clear();
    for &(source, _) in origins {
        origin_set.insert(source.idx());
    }

    let mut output: Vec<RangeJourney<ArrivalTime>> = Vec::new();
    let mut prev_tau: Option<SecondOfDay> = None;

    for &tau in departures {
        // (a) Seed sources at τ + walk. `LabelBag::insert` returns
        // true iff the new label strictly improves the bag; for τ <
        // prev_tau the new label dominates the prior one, so
        // re-seeding tightens the source bag. For the first scan
        // there's nothing to dominate and the seed is added directly.
        marked_stops.clear();
        for &(source, walk) in origins {
            let seed = ArrivalTime(tau + walk);
            if labels[0][source.idx()].insert(seed) {
                best_arrival[source.idx()].insert(seed);
                marked_stops.insert(source.idx());
                ever_reached.insert(source.idx());
            }
        }

        // (b) Mark stops with newly-catchable trips for this τ. The
        // window is half-open `[tau, prev_tau)` – trips departing at
        // exactly `tau` are first-catchable in this scan, while trips
        // at `prev_tau` were already covered by the previous scan.
        if let Some(prev) = prev_tau {
            newly_active_stops_into(tt, tau, prev, marked_stops);
        }

        // (c) Round-0 footpath relax + rounds 1..=transfers, sharing
        // labels with previous scans.
        run_raptor_rounds(
            tt,
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
            targets,
        );

        // (d) Snapshot per-target journeys for this τ.
        let snapshot =
            extract_target_journeys(labels, board_detail, origin_set, targets, transfers);
        for journey in snapshot {
            output.push(RangeJourney {
                depart: tau,
                journey,
            });
        }

        prev_tau = Some(tau);
    }

    filter_range_pareto_front(output)
}
