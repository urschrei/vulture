//! Time-expanded multi-criterion Dijkstra reference solver.
//!
//! Optimise nothing. If we ever debug this, we have gone wrong somewhere.
//!
//! See `mod.rs` for the trip-count convention banner.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

use crate::proptest_support::spec::{NetworkSpec, close_footpaths};

/// Pre-computed per-trip stop schedules: `(stop, arrival, departure)` per
/// stop in the trip's stop sequence.
pub(super) type TripSchedule = Vec<(u8, u16, u16)>;

/// Pre-computation: per-stop relevant timepoints, per-trip schedules,
/// transitively-closed footpath matrix.
pub(super) struct Prep {
    pub nodes: BTreeMap<u8, BTreeSet<u16>>,
    pub trips: Vec<TripSchedule>,
    pub footpaths: Vec<Vec<Option<u16>>>,
    pub n_stops: u8,
}

impl Prep {
    pub fn build(spec: &NetworkSpec, ps: u8, tau: u16) -> Self {
        let footpaths = close_footpaths(spec);

        let mut trips: Vec<TripSchedule> = Vec::new();
        for route in &spec.routes {
            for trip in &route.trips {
                let mut schedule: TripSchedule = Vec::with_capacity(route.stop_sequence.len());
                let mut arr = trip.first_dep;
                let mut dep = arr.saturating_add(trip.dwell_times[0]);
                schedule.push((route.stop_sequence[0], arr, dep));
                for i in 1..route.stop_sequence.len() {
                    arr = dep.saturating_add(trip.leg_durations[i - 1]);
                    dep = arr.saturating_add(trip.dwell_times[i]);
                    schedule.push((route.stop_sequence[i], arr, dep));
                }
                trips.push(schedule);
            }
        }

        let mut nodes: BTreeMap<u8, BTreeSet<u16>> = BTreeMap::new();
        nodes.entry(ps).or_default().insert(tau);
        for sched in &trips {
            for &(s, a, d) in sched {
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
            footpaths,
            n_stops: spec.n_stops,
        }
    }
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
/// `(arrival, trip_count)` from `ps` to `pt` departing at `tau`, capped at
/// `max_trips` total trips.
///
/// The state is `(stop, time)`; the cost stored at each state is `trips_used`.
/// Since the time component is encoded into the node, we keep min trips per
/// node — sufficient for the Pareto-front semantics: any state reachable
/// from `(s, t, k)` is also reachable from `(s, t, k')` with `k' < k` via
/// the same sequence of edges.
pub fn reference_solve(
    spec: &NetworkSpec,
    ps: u8,
    pt: u8,
    tau: u16,
    max_trips: u8,
) -> BTreeSet<(u16, u8)> {
    if ps == pt {
        // `reconstruct_journey` only returns journeys with ≥ 1 trip, so
        // "stay put at the source" is not modelled as a journey. The
        // sibling case for `ps != pt` (a walk-only journey with 0 trips)
        // is filtered below.
        return BTreeSet::new();
    }

    let prep = Prep::build(spec, ps, tau);

    let mut min_trips: BTreeMap<(u8, u16), u8> = BTreeMap::new();
    let mut heap: BinaryHeap<Reverse<(u16, u8, u8)>> = BinaryHeap::new();

    let start = (ps, tau);
    min_trips.insert(start, 0);
    heap.push(Reverse((tau, 0, ps)));

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
            for sched in &prep.trips {
                for i in 0..sched.len() {
                    let (board_stop, _, board_dep) = sched[i];
                    if board_stop != stop || board_dep < t {
                        continue;
                    }
                    for &(alight_stop, alight_arr, _) in &sched[i + 1..] {
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

    // Drop walk-only journeys (k == 0): RAPTOR's `reconstruct_journey`
    // filters out empty plans, so the algorithm cannot emit a
    // walk-from-ps-to-pt journey. Match that convention.
    let mut at_pt: Vec<(u16, u8)> = min_trips
        .iter()
        .filter(|((s, _), _)| *s == pt)
        .filter(|&(_, &k)| k > 0 && k <= max_trips)
        .map(|(&(_, t), &k)| (t, k))
        .collect();

    // Pareto filter: sort by trip count ascending, keep strictly-decreasing arrival.
    at_pt.sort_by_key(|&(t, k)| (k, t));
    let mut best = u16::MAX;
    let mut out = BTreeSet::new();
    for (t, k) in at_pt {
        if t < best {
            best = t;
            out.insert((t, k));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proptest_support::spec::*;

    fn front(items: &[(u16, u8)]) -> BTreeSet<(u16, u8)> {
        items.iter().copied().collect()
    }

    #[test]
    fn ps_eq_pt_returns_empty_to_match_algorithm() {
        let spec = NetworkSpec {
            n_stops: 2,
            routes: vec![],
            footpaths: vec![],
            query: QuerySpec {
                ps: 0,
                pt: 0,
                tau: 42,
                max_transfers: 3,
            },
        };
        let r = reference_solve(&spec, 0, 0, 42, 3);
        assert!(r.is_empty(), "ps == pt is not modelled as a journey");
    }

    #[test]
    fn disconnected_returns_empty() {
        let spec = NetworkSpec {
            n_stops: 2,
            routes: vec![],
            footpaths: vec![],
            query: QuerySpec {
                ps: 0,
                pt: 1,
                tau: 0,
                max_transfers: 3,
            },
        };
        let r = reference_solve(&spec, 0, 1, 0, 3);
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
                }],
            }],
            footpaths: vec![],
            query: QuerySpec {
                ps: 0,
                pt: 1,
                tau: 0,
                max_transfers: 3,
            },
        };
        let r = reference_solve(&spec, 0, 1, 0, 3);
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
            query: QuerySpec {
                ps: 0,
                pt: 1,
                tau: 100,
                max_transfers: 3,
            },
        };
        let r = reference_solve(&spec, 0, 1, 100, 3);
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
                }],
            }],
            footpaths: vec![FootpathSpec {
                from: 0,
                to: 1,
                walk_time: 5,
            }],
            query: QuerySpec {
                ps: 0,
                pt: 2,
                tau: 0,
                max_transfers: 3,
            },
        };
        let r = reference_solve(&spec, 0, 2, 0, 3);
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
                    }],
                },
                RouteSpec {
                    stop_sequence: vec![0, 1],
                    trips: vec![TripSpec {
                        first_dep: 0,
                        leg_durations: vec![80],
                        dwell_times: vec![0, 0],
                    }],
                },
            ],
            footpaths: vec![],
            query: QuerySpec {
                ps: 0,
                pt: 1,
                tau: 0,
                max_transfers: 3,
            },
        };
        let r = reference_solve(&spec, 0, 1, 0, 3);
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
                }],
            }],
            footpaths: vec![FootpathSpec {
                from: 1,
                to: 2,
                walk_time: 7,
            }],
            query: QuerySpec {
                ps: 0,
                pt: 2,
                tau: 0,
                max_transfers: 2,
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
