//! Time-expanded multi-criterion Dijkstra reference solver.
//!
//! Optimise nothing. If we ever debug this, we have gone wrong somewhere.
//!
//! See `mod.rs` for the trip-count convention banner.

use std::collections::{BTreeMap, BTreeSet};

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
                if let Some(walk) = footpaths[from as usize][to as usize] {
                    if let Some(arr) = t.checked_add(walk) {
                        nodes.entry(to).or_default().insert(arr);
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proptest_support::spec::*;

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
