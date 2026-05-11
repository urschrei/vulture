//! Time-expanded multi-criterion Dijkstra reference solver.
//!
//! Optimise nothing. If we ever debug this, we have gone wrong somewhere.
//!
//! See `lib.rs` for the trip-count convention banner.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

use crate::spec::{NetworkSpec, close_footpaths};

/// Pre-computed per-trip stop schedules: per stop in the trip's
/// sequence, `(stop, arrival, departure, no_pickup, no_drop_off)`.
pub(super) type TripSchedule = Vec<(u8, u16, u16, bool, bool)>;

/// Pre-computation: per-stop relevant timepoints, per-trip schedules,
/// transitively-closed footpath matrix.
pub(super) struct Prep {
    /// Time-expanded node set: stop -> set of relevant timepoints. Built
    /// for the unit tests below; the Dijkstra core walks the trips/
    /// footpaths fields directly without consulting this map.
    #[cfg_attr(not(test), allow(dead_code))]
    pub nodes: BTreeMap<u8, BTreeSet<u16>>,
    pub trips: Vec<TripSchedule>,
    /// Per-trip `wheelchair_accessible`, parallel to `trips`.
    pub trip_wheelchair: Vec<bool>,
    pub footpaths: Vec<Vec<Option<u16>>>,
    pub n_stops: u8,
}

impl Prep {
    pub fn build(spec: &NetworkSpec, ps: u8, tau: u16) -> Self {
        let footpaths = close_footpaths(spec);

        let mut trips: Vec<TripSchedule> = Vec::new();
        let mut trip_wheelchair: Vec<bool> = Vec::new();
        for route in &spec.routes {
            for trip in &route.trips {
                let mut schedule: TripSchedule = Vec::with_capacity(route.stop_sequence.len());
                let mut arr = trip.first_dep;
                let mut dep = arr.saturating_add(trip.dwell_times[0]);
                let np = pickup_flag(&trip.no_pickup_at, 0);
                let nd = dropoff_flag(&trip.no_drop_off_at, 0);
                schedule.push((route.stop_sequence[0], arr, dep, np, nd));
                for i in 1..route.stop_sequence.len() {
                    arr = dep.saturating_add(trip.leg_durations[i - 1]);
                    dep = arr.saturating_add(trip.dwell_times[i]);
                    let np = pickup_flag(&trip.no_pickup_at, i);
                    let nd = dropoff_flag(&trip.no_drop_off_at, i);
                    schedule.push((route.stop_sequence[i], arr, dep, np, nd));
                }
                trips.push(schedule);
                trip_wheelchair.push(trip.wheelchair_accessible);
            }
        }

        let mut nodes: BTreeMap<u8, BTreeSet<u16>> = BTreeMap::new();
        nodes.entry(ps).or_default().insert(tau);
        for sched in &trips {
            for &(s, a, d, _, _) in sched {
                let entry = nodes.entry(s).or_default();
                entry.insert(a);
                entry.insert(d);
            }
        }

        // One walk hop suffices because the footpath matrix is already
        // transitively closed.
        let initial: Vec<(u8, u16)> = nodes
            .iter()
            .flat_map(|(&s, ts)| ts.iter().map(move |&t| (s, t)))
            .collect();
        for (from, t) in initial {
            for to in 0..spec.n_stops {
                if to == from {
                    continue;
                }
                if let Some(walk) = footpaths[from as usize][to as usize]
                    && let Some(arr) = t.checked_add(walk)
                {
                    nodes.entry(to).or_default().insert(arr);
                }
            }
        }

        Prep {
            nodes,
            trips,
            trip_wheelchair,
            footpaths,
            n_stops: spec.n_stops,
        }
    }
}

/// `no_pickup_at` may be empty (treated as all-false) or full-length.
fn pickup_flag(flags: &[bool], i: usize) -> bool {
    flags.get(i).copied().unwrap_or(false)
}

/// Same convention for `no_drop_off_at`.
fn dropoff_flag(flags: &[bool], i: usize) -> bool {
    flags.get(i).copied().unwrap_or(false)
}

fn relax(
    min_trips: &mut BTreeMap<(u8, u16), u8>,
    heap: &mut BinaryHeap<Reverse<(u16, u8, u8)>>,
    stop: u8,
    t: u16,
    trips: u8,
) {
    let entry = min_trips.entry((stop, t)).or_insert(u8::MAX);
    if trips < *entry {
        *entry = trips;
        heap.push(Reverse((t, trips, stop)));
    }
}

/// Brute-force ground-truth solver for the Pareto front of
/// `(arrival, trip_count)` from any of `origins` to any of `targets`,
/// departing at `tau`, capped at `max_trips` total trips.
///
/// `origins` and `targets` are `(stop, walk_offset_seconds)` slices —
/// each origin seeds the search at `tau + walk_offset`, and each
/// target's effective arrival is `arrival_at_stop + walk_offset`.
/// The Pareto filter is applied across the union of effective
/// arrivals at every target.
///
/// The state is `(stop, time)`; the cost stored at each state is `trips_used`.
/// Since the time component is encoded into the node, we keep min trips per
/// node – sufficient for the Pareto-front semantics: any state reachable
/// from `(s, t, k)` is also reachable from `(s, t, k')` with `k' < k` via
/// the same sequence of edges.
///
/// `require_wheelchair_accessible` mirrors the query option: when true,
/// trips with `wheelchair_accessible = false` and stops in
/// `inaccessible_stops` are skipped (alighting only — boarding from an
/// inaccessible stop is allowed, matching the algorithm's gating).
/// `no_pickup_at` / `no_drop_off_at` are enforced unconditionally.
pub fn reference_solve(
    spec: &NetworkSpec,
    origins: &[(u8, u16)],
    targets: &[(u8, u16)],
    tau: u16,
    max_trips: u8,
    require_wheelchair_accessible: bool,
) -> BTreeSet<(u16, u8)> {
    // Use the first origin as Prep's nominal start; Prep's job is to
    // enumerate candidate timepoints, and adding more sources only
    // grows that set — Prep handles all reachable starts uniformly.
    let nominal_start = origins.first().map(|&(s, _)| s).unwrap_or(0);
    let prep = Prep::build(spec, nominal_start, tau);
    let inaccessible_stops = &spec.inaccessible_stops;

    let mut min_trips: BTreeMap<(u8, u16), u8> = BTreeMap::new();
    let mut heap: BinaryHeap<Reverse<(u16, u8, u8)>> = BinaryHeap::new();

    // Seed every origin at `tau + walk_offset`, saturating on overflow
    // so degenerate walks don't pollute the search horizon.
    for &(origin, walk) in origins {
        let start_t = tau.saturating_add(walk);
        let key = (origin, start_t);
        let entry = min_trips.entry(key).or_insert(u8::MAX);
        if 0 < *entry {
            *entry = 0;
            heap.push(Reverse((start_t, 0, origin)));
        }
    }

    while let Some(Reverse((t, trips, stop))) = heap.pop() {
        if min_trips.get(&(stop, t)).copied() != Some(trips) {
            continue;
        }

        // Walk edges via transitively-closed footpaths. Free in trip count.
        for to in 0..prep.n_stops {
            if to == stop {
                continue;
            }
            if let Some(walk) = prep.footpaths[stop as usize][to as usize]
                && let Some(new_t) = t.checked_add(walk)
            {
                relax(&mut min_trips, &mut heap, to, new_t, trips);
            }
        }

        // Atomic board+ride-segment: pay +1 trip to ride a trip from any
        // stop on its sequence (where dep ≥ t) to any later stop on the
        // same sequence. This avoids the "free re-ride" bug from a separate
        // ride edge that doesn't track which trip you're on.
        if trips < max_trips {
            for (trip_idx, sched) in prep.trips.iter().enumerate() {
                // Wheelchair gate (trip-level) — only when the query
                // opts into it. Skips the trip entirely.
                if require_wheelchair_accessible && !prep.trip_wheelchair[trip_idx] {
                    continue;
                }
                for i in 0..sched.len() {
                    let (board_stop, _, board_dep, no_pickup, _) = sched[i];
                    if board_stop != stop || board_dep < t {
                        continue;
                    }
                    // GTFS pickup_type = 1: boarding forbidden at this position.
                    if no_pickup {
                        continue;
                    }
                    for &(alight_stop, alight_arr, _, _, no_drop_off) in &sched[i + 1..] {
                        // GTFS drop_off_type = 1: alighting forbidden at this position.
                        if no_drop_off {
                            continue;
                        }
                        // Wheelchair gate (stop-level) — alight only.
                        if require_wheelchair_accessible
                            && inaccessible_stops.contains(&alight_stop)
                        {
                            continue;
                        }
                        relax(
                            &mut min_trips,
                            &mut heap,
                            alight_stop,
                            alight_arr,
                            trips + 1,
                        );
                    }
                }
            }
        }
    }

    // Build `best_t_at[stop][k]` mirroring vulture's per-`(round, stop)`
    // label bag with single-criterion ArrivalTime: the best arrival at
    // `stop` reachable in *at most* `k` trips. Carry-forward semantics
    // (best in ≤k trips is at most best in ≤k-1 trips) collapses any
    // loop-back-with-more-trips that arrives later than a fewer-trips
    // path to the same stop, matching vulture's bag domination —
    // labels[k][stop] inherits from labels[k-1][stop], so a worse
    // arrival at higher k is dropped at insert time.
    let n_stops = prep.n_stops as usize;
    let max_k = usize::from(max_trips);
    let mut best_t_at: Vec<Vec<u16>> = vec![vec![u16::MAX; max_k + 1]; n_stops];
    for (&(stop, t), &k) in min_trips.iter() {
        if k as usize > max_k {
            continue;
        }
        let cell = &mut best_t_at[stop as usize][k as usize];
        if t < *cell {
            *cell = t;
        }
    }
    for row in best_t_at.iter_mut() {
        for k in 1..=max_k {
            if row[k - 1] < row[k] {
                row[k] = row[k - 1];
            }
        }
    }

    // RAPTOR's pt_threshold = min over targets of (walk-only arrival at
    // target_stop + target_walk). Mirrors the algorithm's local/target
    // pruning, which uses strict inequality on raw arrival vs threshold:
    // a trip-based journey is dropped if its raw arrival at the target
    // stop is ≥ τ*. Computed in raw-arrival space (not effective).
    let walk_only_tau_star: u16 = targets
        .iter()
        .map(|&(target_stop, target_walk)| {
            best_t_at[target_stop as usize][0].saturating_add(target_walk)
        })
        .min()
        .unwrap_or(u16::MAX);

    // For each target × k, emit a journey iff:
    // 1. `best_t_at[target][k]` strictly improves on
    //    `best_t_at[target][k-1]` — vulture's bag carries forward, so
    //    no improvement at this k means no boarding-tree step at
    //    (k, target, raw_arr) and reconstruction yields nothing.
    // 2. The raw arrival is strictly less than the walk-only τ*, the
    //    algorithm's local-pruning threshold.
    // Walk-only journeys (k = 0) are never emitted: RAPTOR cannot
    // produce empty plans.
    let mut out: BTreeSet<(u16, u8)> = BTreeSet::new();
    for &(target_stop, target_walk) in targets {
        let row = &best_t_at[target_stop as usize];
        for k in 1..=max_k {
            if row[k] < row[k - 1] && row[k] < walk_only_tau_star {
                let eff = row[k].saturating_add(target_walk);
                out.insert((eff, k as u8));
            }
        }
    }

    // Pareto-filter across the union of (target, k) emissions: sort by
    // (k, t), keep strictly-decreasing arrival.
    let mut entries: Vec<(u16, u8)> = out.into_iter().collect();
    entries.sort_by_key(|&(t, k)| (k, t));
    let mut best = u16::MAX;
    let mut pareto: BTreeSet<(u16, u8)> = BTreeSet::new();
    for (t, k) in entries {
        if t < best {
            best = t;
            pareto.insert((t, k));
        }
    }
    pareto
}

/// Brute-force reference for the `ArrivalAndWalk` two-criterion label.
/// Returns the Pareto front of `(effective_arrival, walk_time, trip_count)`
/// reachable from any origin to any target, departing at `tau`, capped at
/// `max_trips`.
///
/// State space is `(stop, raw_arrival)` keyed Pareto bags of
/// `(walk_time, trips)`. Walk edges add `walk_dur` to both `raw_arrival`
/// and `walk_time`; ride edges advance `raw_arrival` to the trip's
/// alighting time and leave `walk_time` unchanged. Pareto dominance for
/// bag pruning is component-wise on `(walk_time, trips)` at fixed
/// `(stop, raw_arrival)`.
///
/// Pure brute force: no `pt_threshold` pruning is applied. Every reachable
/// `(stop, raw_arrival, walk_time, trips)` tuple at a target stop becomes
/// a candidate; the algorithm's `pt_threshold` bag is a pruning
/// optimisation that does not change the final per-target Pareto front,
/// so brute force can ignore it and Pareto-filter at output.
///
/// Walk-only journeys (`k = 0`) are not emitted, matching RAPTOR's
/// empty-plan filter.
pub fn reference_solve_arrival_and_walk(
    spec: &NetworkSpec,
    origins: &[(u8, u16)],
    targets: &[(u8, u16)],
    tau: u16,
    max_trips: u8,
    require_wheelchair_accessible: bool,
) -> BTreeSet<(u16, u16, u8)> {
    let nominal_start = origins.first().map(|&(s, _)| s).unwrap_or(0);
    let prep = Prep::build(spec, nominal_start, tau);
    let inaccessible_stops = &spec.inaccessible_stops;

    // Per-(stop, raw_arrival) Pareto bag of (walk_time, trips). An insert
    // succeeds when the new pair is not weakly dominated; previously-bagged
    // pairs weakly dominated by the new one are evicted.
    let mut bags: BTreeMap<(u8, u16), Vec<(u16, u8)>> = BTreeMap::new();

    fn try_insert(bag: &mut Vec<(u16, u8)>, new: (u16, u8)) -> bool {
        for &existing in bag.iter() {
            if existing.0 <= new.0 && existing.1 <= new.1 {
                return false;
            }
        }
        bag.retain(|&existing| !(new.0 <= existing.0 && new.1 <= existing.1));
        bag.push(new);
        true
    }

    let mut queue: Vec<(u8, u16, u16, u8)> = Vec::new();
    for &(origin, walk) in origins {
        let start_t = tau.saturating_add(walk);
        let bag = bags.entry((origin, start_t)).or_default();
        if try_insert(bag, (0, 0)) {
            queue.push((origin, start_t, 0, 0));
        }
    }

    while let Some((stop, t, w, k)) = queue.pop() {
        // Validate label is still present in its bag (may have been evicted
        // by a later dominating insert).
        let still_present = bags
            .get(&(stop, t))
            .map(|b| b.contains(&(w, k)))
            .unwrap_or(false);
        if !still_present {
            continue;
        }

        // Walk edges (transitively-closed footpath matrix).
        for to in 0..prep.n_stops {
            if to == stop {
                continue;
            }
            if let Some(walk_dur) = prep.footpaths[stop as usize][to as usize]
                && let Some(new_t) = t.checked_add(walk_dur)
            {
                let new_w = w.saturating_add(walk_dur);
                let bag = bags.entry((to, new_t)).or_default();
                if try_insert(bag, (new_w, k)) {
                    queue.push((to, new_t, new_w, k));
                }
            }
        }

        // Ride edges: atomic board+ride-segment, +1 trip.
        if k < max_trips {
            for (trip_idx, sched) in prep.trips.iter().enumerate() {
                if require_wheelchair_accessible && !prep.trip_wheelchair[trip_idx] {
                    continue;
                }
                for i in 0..sched.len() {
                    let (board_stop, _, board_dep, no_pickup, _) = sched[i];
                    if board_stop != stop || board_dep < t {
                        continue;
                    }
                    if no_pickup {
                        continue;
                    }
                    for &(alight_stop, alight_arr, _, _, no_drop_off) in &sched[i + 1..] {
                        if no_drop_off {
                            continue;
                        }
                        if require_wheelchair_accessible
                            && inaccessible_stops.contains(&alight_stop)
                        {
                            continue;
                        }
                        let bag = bags.entry((alight_stop, alight_arr)).or_default();
                        if try_insert(bag, (w, k + 1)) {
                            queue.push((alight_stop, alight_arr, w, k + 1));
                        }
                    }
                }
            }
        }
    }

    // Build a `pt_threshold` Pareto bag from walk-only labels (k = 0) at
    // each target, extended by that target's walk-offset. Mirrors the
    // algorithm's cross-target pt_threshold mechanism (boarding.rs's
    // best_to_any_target): trip-based labels at any stop whose RAW (arr,
    // walk_time) is weakly dominated by any entry in this bag are filtered
    // out, matching the algorithm's raw-vs-effective conservative check.
    let mut pt_threshold_bag: Vec<(u16, u16)> = Vec::new();
    for &(target_stop, target_walk) in targets {
        for (&(s, t), bag) in &bags {
            if s != target_stop {
                continue;
            }
            for &(w, k) in bag {
                if k != 0 {
                    continue;
                }
                let eff_arr = t.saturating_add(target_walk);
                let eff_walk = w.saturating_add(target_walk);
                // Pareto insert.
                let dominated = pt_threshold_bag
                    .iter()
                    .any(|&(a, v)| a <= eff_arr && v <= eff_walk);
                if dominated {
                    continue;
                }
                pt_threshold_bag.retain(|&(a, v)| !(eff_arr <= a && eff_walk <= v));
                pt_threshold_bag.push((eff_arr, eff_walk));
            }
        }
    }

    // Per-target Pareto filter. Each target is a different destination from
    // the user's perspective; trip-based journeys to different targets are
    // not directly compared. Within each target, walk-only labels at the
    // same target participate in dominance so a trip-based journey that
    // ends up worse than just walking to *this* target gets dropped. Drop
    // k = 0 from the output to match RAPTOR's empty-plan emission rule.
    let mut front: BTreeSet<(u16, u16, u8)> = BTreeSet::new();
    for &(target_stop, target_walk) in targets {
        let mut per_target: Vec<(u16, u16, u8)> = Vec::new();
        for (&(s, t), bag) in &bags {
            if s != target_stop {
                continue;
            }
            for &(w, k) in bag {
                if k > 0 {
                    // Algorithm-equivalent cross-target pt_threshold filter:
                    // raw (t, w) is dropped if any pt_threshold bag entry
                    // (effective form) weakly dominates it.
                    let dominated_by_threshold =
                        pt_threshold_bag.iter().any(|&(a, v)| a <= t && v <= w);
                    if dominated_by_threshold {
                        continue;
                    }
                }
                let eff_arrival = t.saturating_add(target_walk);
                let eff_walk_time = w.saturating_add(target_walk);
                per_target.push((eff_arrival, eff_walk_time, k));
            }
        }

        // 3-D Pareto filter within this target's candidates: keep `c` iff
        // no other `c'` weakly dominates `c` on every component AND
        // strictly dominates on at least one.
        'outer: for &c in &per_target {
            for &other in &per_target {
                if other == c {
                    continue;
                }
                if other.0 <= c.0 && other.1 <= c.1 && other.2 <= c.2 {
                    continue 'outer;
                }
            }
            if c.2 == 0 {
                continue;
            }
            front.insert(c);
        }
    }
    front
}

/// Range-query reference: runs `reference_solve` once per departure
/// and applies the same 3-D Pareto filter vulture's
/// `filter_range_pareto_front` uses — drop any `(depart, arrival, k)`
/// triple weakly dominated by another on `(later depart, fewer
/// transfers, earlier arrival)`.
///
/// Output is a `BTreeSet<(depart, arrival, k)>` so the proptest can
/// compare order-independently against vulture's `Vec<RangeJourney>`.
pub fn reference_range_solve(
    spec: &NetworkSpec,
    origins: &[(u8, u16)],
    targets: &[(u8, u16)],
    departures: &[u16],
    max_trips: u8,
    require_wheelchair_accessible: bool,
) -> BTreeSet<(u16, u16, u8)> {
    let mut all: Vec<(u16, u16, u8)> = Vec::new();
    for &tau in departures {
        let front = reference_solve(
            spec,
            origins,
            targets,
            tau,
            max_trips,
            require_wheelchair_accessible,
        );
        for (arrival, k) in front {
            all.push((tau, arrival, k));
        }
    }

    // Sort matching vulture's filter_range_pareto_front: descending
    // depart, ascending plan_len (= k), ascending arrival. Earlier
    // entries are preferred so equal-on-all-three duplicates resolve
    // to the first one in.
    all.sort_by(|a, b| b.0.cmp(&a.0).then(a.2.cmp(&b.2)).then(a.1.cmp(&b.1)));

    let mut front: Vec<(u16, u16, u8)> = Vec::with_capacity(all.len());
    'outer: for r in all {
        let (rd, ra, rk) = r;
        for &(fd, fa, fk) in &front {
            if fd >= rd && fk <= rk && fa <= ra {
                continue 'outer;
            }
        }
        front.retain(|&(fd, fa, fk)| !(rd >= fd && rk <= fk && ra <= fa));
        front.push(r);
    }

    front.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::*;

    fn front(items: &[(u16, u8)]) -> BTreeSet<(u16, u8)> {
        items.iter().copied().collect()
    }

    #[test]
    fn ps_eq_pt_returns_empty_to_match_algorithm() {
        let spec = NetworkSpec {
            n_stops: 2,
            routes: vec![],
            footpaths: vec![],
            inaccessible_stops: BTreeSet::new(),
            query: QuerySpec {
                origins: vec![(0, 0)],
                targets: vec![(0, 0)],
                tau: 42,
                max_transfers: 3,
                require_wheelchair_accessible: false,
            },
        };
        let r = reference_solve(&spec, &[(0, 0)], &[(0, 0)], 42, 3, false);
        assert!(r.is_empty(), "ps == pt is not modelled as a journey");
    }

    #[test]
    fn disconnected_returns_empty() {
        let spec = NetworkSpec {
            n_stops: 2,
            routes: vec![],
            footpaths: vec![],
            inaccessible_stops: BTreeSet::new(),
            query: QuerySpec {
                origins: vec![(0, 0)],
                targets: vec![(1, 0)],
                tau: 0,
                max_transfers: 3,
                require_wheelchair_accessible: false,
            },
        };
        let r = reference_solve(&spec, &[(0, 0)], &[(1, 0)], 0, 3, false);
        assert!(r.is_empty());
    }

    #[test]
    fn single_trip_one_stop_hop() {
        let spec = NetworkSpec {
            n_stops: 2,
            routes: vec![RouteSpec {
                stop_sequence: vec![0, 1],
                trips: vec![TripSpec {
                    first_dep: 10,
                    leg_durations: vec![20],
                    dwell_times: vec![0, 0],
                    wheelchair_accessible: true,
                    no_pickup_at: vec![],
                    no_drop_off_at: vec![],
                }],
                fare: 0,
            }],
            footpaths: vec![],
            inaccessible_stops: BTreeSet::new(),
            query: QuerySpec {
                origins: vec![(0, 0)],
                targets: vec![(1, 0)],
                tau: 0,
                max_transfers: 3,
                require_wheelchair_accessible: false,
            },
        };
        let r = reference_solve(&spec, &[(0, 0)], &[(1, 0)], 0, 3, false);
        assert_eq!(r, front(&[(30, 1)]));
    }

    #[test]
    fn walk_only_journey_is_dropped() {
        // RAPTOR's `reconstruct_journey` cannot emit a walk-only journey
        // (no boarding events → empty plan → filtered). The reference
        // solver matches that convention by dropping `k == 0` journeys.
        let spec = NetworkSpec {
            n_stops: 2,
            routes: vec![],
            footpaths: vec![FootpathSpec {
                from: 0,
                to: 1,
                walk_time: 5,
            }],
            inaccessible_stops: BTreeSet::new(),
            query: QuerySpec {
                origins: vec![(0, 0)],
                targets: vec![(1, 0)],
                tau: 100,
                max_transfers: 3,
                require_wheelchair_accessible: false,
            },
        };
        let r = reference_solve(&spec, &[(0, 0)], &[(1, 0)], 100, 3, false);
        assert!(r.is_empty(), "walk-only journey should be filtered out");
    }

    #[test]
    fn walk_then_board_journey() {
        // A--walk-->B, then trip B->C. ps=A, pt=C.
        let spec = NetworkSpec {
            n_stops: 3,
            routes: vec![RouteSpec {
                stop_sequence: vec![1, 2],
                trips: vec![TripSpec {
                    first_dep: 50,
                    leg_durations: vec![20],
                    dwell_times: vec![0, 0],
                    wheelchair_accessible: true,
                    no_pickup_at: vec![],
                    no_drop_off_at: vec![],
                }],
                fare: 0,
            }],
            footpaths: vec![FootpathSpec {
                from: 0,
                to: 1,
                walk_time: 5,
            }],
            inaccessible_stops: BTreeSet::new(),
            query: QuerySpec {
                origins: vec![(0, 0)],
                targets: vec![(2, 0)],
                tau: 0,
                max_transfers: 3,
                require_wheelchair_accessible: false,
            },
        };
        let r = reference_solve(&spec, &[(0, 0)], &[(2, 0)], 0, 3, false);
        assert_eq!(r, front(&[(70, 1)]));
    }

    #[test]
    fn pareto_front_two_options_one_dominated() {
        // Two parallel routes ps=0 -> pt=1 with same trip count; only fastest
        // survives the Pareto filter.
        let spec = NetworkSpec {
            n_stops: 2,
            routes: vec![
                RouteSpec {
                    stop_sequence: vec![0, 1],
                    trips: vec![TripSpec {
                        first_dep: 0,
                        leg_durations: vec![100],
                        dwell_times: vec![0, 0],
                        wheelchair_accessible: true,
                        no_pickup_at: vec![],
                        no_drop_off_at: vec![],
                    }],
                    fare: 0,
                },
                RouteSpec {
                    stop_sequence: vec![0, 1],
                    trips: vec![TripSpec {
                        first_dep: 0,
                        leg_durations: vec![80],
                        dwell_times: vec![0, 0],
                        wheelchair_accessible: true,
                        no_pickup_at: vec![],
                        no_drop_off_at: vec![],
                    }],
                    fare: 0,
                },
            ],
            footpaths: vec![],
            inaccessible_stops: BTreeSet::new(),
            query: QuerySpec {
                origins: vec![(0, 0)],
                targets: vec![(1, 0)],
                tau: 0,
                max_transfers: 3,
                require_wheelchair_accessible: false,
            },
        };
        let r = reference_solve(&spec, &[(0, 0)], &[(1, 0)], 0, 3, false);
        assert_eq!(r, front(&[(80, 1)]));
    }

    #[test]
    fn node_set_includes_trip_arrivals_departures_and_walk_targets() {
        let spec = NetworkSpec {
            n_stops: 3,
            routes: vec![RouteSpec {
                stop_sequence: vec![0, 1],
                trips: vec![TripSpec {
                    first_dep: 100,
                    leg_durations: vec![20],
                    dwell_times: vec![5, 0],
                    wheelchair_accessible: true,
                    no_pickup_at: vec![],
                    no_drop_off_at: vec![],
                }],
                fare: 0,
            }],
            footpaths: vec![FootpathSpec {
                from: 1,
                to: 2,
                walk_time: 7,
            }],
            inaccessible_stops: BTreeSet::new(),
            query: QuerySpec {
                origins: vec![(0, 0)],
                targets: vec![(2, 0)],
                tau: 0,
                max_transfers: 2,
                require_wheelchair_accessible: false,
            },
        };
        let prep = Prep::build(&spec, 0u8, 0u16);
        // Stop 0: tau=0, trip arr=100, trip dep=105
        assert!(prep.nodes[&0].contains(&0));
        assert!(prep.nodes[&0].contains(&100));
        assert!(prep.nodes[&0].contains(&105));
        // Stop 1: trip arr=125, trip dep=125
        assert!(prep.nodes[&1].contains(&125));
        // Stop 2: walk-arrivals from stop 1 timepoints (125 -> 132)
        assert!(prep.nodes[&2].contains(&132));
    }
}
