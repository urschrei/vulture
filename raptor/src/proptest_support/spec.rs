//! Spec data model, per-layer Hegel composite generators, and the renderer
//! that turns a `NetworkSpec` into a `SimpleTimetable<u8, u8, u16>`.
//!
//! See `mod.rs` for the trip-count convention banner.

use crate::simple::SimpleTimetable;

/// Top-level spec for one randomly-generated test case.
#[derive(Debug, Clone)]
pub struct NetworkSpec {
    pub n_stops: u8,
    pub routes: Vec<RouteSpec>,
    pub footpaths: Vec<FootpathSpec>,
    pub query: QuerySpec,
}

/// One route: an ordered sequence of stops served by 1+ trips that share
/// a leg/dwell pattern (so overtaking is structurally impossible).
#[derive(Debug, Clone)]
pub struct RouteSpec {
    /// Distinct stop indices, all `< spec.n_stops`. `len() ∈ [2, 4]`.
    pub stop_sequence: Vec<u8>,
    /// Trips ordered by `first_dep`. `len() ∈ [1, 3]`.
    pub trips: Vec<TripSpec>,
}

/// One trip on a route. The renderer reconstructs `(arrival, departure)`
/// pairs by prefix-summing dwell and leg durations onto `first_dep`.
#[derive(Debug, Clone)]
pub struct TripSpec {
    pub first_dep: u16,
    /// `len() == stop_sequence.len() - 1`. Each `≥ 1`.
    pub leg_durations: Vec<u16>,
    /// `len() == stop_sequence.len()`. Each `≥ 0`.
    pub dwell_times: Vec<u16>,
}

/// One sparse footpath. The renderer transitively closes the footpath graph.
#[derive(Debug, Clone)]
pub struct FootpathSpec {
    pub from: u8,
    pub to: u8,
    pub walk_time: u16,
}

/// The query parameters: source, target, departure, max trip count.
#[derive(Debug, Clone)]
pub struct QuerySpec {
    pub ps: u8,
    pub pt: u8,
    pub tau: u16,
    pub max_transfers: u8,
}

/// Render a `NetworkSpec` into an executable `SimpleTimetable`.
///
/// Total and panic-free on every spec the layer generators produce.
/// Deterministic: same spec always renders byte-identically. The renderer
/// does not silently drop or normalize anything that violates the spec
/// contract — generator bugs surface as panics in debug rather than as
/// hidden mis-tests.
pub fn render(spec: &NetworkSpec) -> SimpleTimetable<u8, u8, u16> {
    let mut tt: SimpleTimetable<u8, u8, u16> = SimpleTimetable::new();
    let mut next_trip_id: u16 = 0;

    for (route_idx, route) in spec.routes.iter().enumerate() {
        let route_id = u8::try_from(route_idx).expect("route count exceeds u8");
        let stops = &route.stop_sequence;
        assert!(stops.len() >= 2, "route must have >= 2 stops");
        assert_eq!(
            route.trips.first().map(|t| t.leg_durations.len()),
            Some(stops.len() - 1),
            "leg_durations length mismatch",
        );

        let mut trip_owned: Vec<(u16, Vec<(crate::Tau, crate::Tau)>)> =
            Vec::with_capacity(route.trips.len());
        for trip in &route.trips {
            assert_eq!(trip.leg_durations.len(), stops.len() - 1);
            assert_eq!(trip.dwell_times.len(), stops.len());

            let mut times: Vec<(crate::Tau, crate::Tau)> = Vec::with_capacity(stops.len());
            let arr0 = trip.first_dep as crate::Tau;
            let dep0 = arr0 + trip.dwell_times[0] as crate::Tau;
            times.push((arr0, dep0));
            for i in 1..stops.len() {
                let prev_dep = times[i - 1].1;
                let arr = prev_dep + trip.leg_durations[i - 1] as crate::Tau;
                let dep = arr + trip.dwell_times[i] as crate::Tau;
                times.push((arr, dep));
            }

            trip_owned.push((next_trip_id, times));
            next_trip_id = next_trip_id.checked_add(1).expect("trip id overflow");
        }

        let trip_refs: Vec<(u16, &[(crate::Tau, crate::Tau)])> = trip_owned
            .iter()
            .map(|(id, times)| (*id, times.as_slice()))
            .collect();
        tt = tt.route(route_id, stops, &trip_refs);
    }

    let closed = close_footpaths(spec);
    for from in 0..spec.n_stops {
        for to in 0..spec.n_stops {
            if from == to {
                continue;
            }
            if let Some(walk) = closed[from as usize][to as usize] {
                tt = tt.footpath(from, to);
                tt = tt.transfer_time(from, to, walk as crate::Tau);
            }
        }
    }

    tt
}

/// Floyd–Warshall transitive closure of the sparse footpath list under
/// min-plus. Returns an `n × n` matrix; `m[i][j]` is the shortest walk
/// time from `i` to `j` (saturating on overflow), or `None` if `i == j`
/// or no walk path exists.
pub fn close_footpaths(spec: &NetworkSpec) -> Vec<Vec<Option<u16>>> {
    let n = spec.n_stops as usize;
    let mut dist: Vec<Vec<Option<u16>>> = vec![vec![None; n]; n];

    for fp in &spec.footpaths {
        let i = fp.from as usize;
        let j = fp.to as usize;
        if i == j || i >= n || j >= n {
            continue;
        }
        dist[i][j] = Some(match dist[i][j] {
            Some(d) => d.min(fp.walk_time),
            None => fp.walk_time,
        });
    }

    for k in 0..n {
        for i in 0..n {
            for j in 0..n {
                if i == j {
                    continue;
                }
                if let (Some(ik), Some(kj)) = (dist[i][k], dist[k][j]) {
                    let via_k = ik.saturating_add(kj);
                    dist[i][j] = Some(match dist[i][j] {
                        Some(d) => d.min(via_k),
                        None => via_k,
                    });
                }
            }
        }
    }

    dist
}

#[test]
fn close_footpaths_two_hop_chain() {
    let spec = NetworkSpec {
        n_stops: 3,
        routes: vec![],
        footpaths: vec![
            FootpathSpec {
                from: 0,
                to: 1,
                walk_time: 5,
            },
            FootpathSpec {
                from: 1,
                to: 2,
                walk_time: 7,
            },
        ],
        query: QuerySpec {
            ps: 0,
            pt: 2,
            tau: 0,
            max_transfers: 1,
        },
    };
    let closed = close_footpaths(&spec);
    assert_eq!(closed[0][1], Some(5));
    assert_eq!(closed[1][2], Some(7));
    assert_eq!(closed[0][2], Some(12), "two-hop walk should be closed");
    assert_eq!(closed[2][0], None, "directed: no return edge added");
    assert_eq!(closed[0][0], None, "no self-loop");
}

#[test]
fn close_footpaths_picks_min_when_duplicate() {
    let spec = NetworkSpec {
        n_stops: 2,
        routes: vec![],
        footpaths: vec![
            FootpathSpec {
                from: 0,
                to: 1,
                walk_time: 10,
            },
            FootpathSpec {
                from: 0,
                to: 1,
                walk_time: 4,
            },
        ],
        query: QuerySpec {
            ps: 0,
            pt: 1,
            tau: 0,
            max_transfers: 1,
        },
    };
    let closed = close_footpaths(&spec);
    assert_eq!(closed[0][1], Some(4));
}

#[test]
fn render_single_route_two_stops_one_trip() {
    use crate::Timetable;
    let spec = NetworkSpec {
        n_stops: 2,
        routes: vec![RouteSpec {
            stop_sequence: vec![0, 1],
            trips: vec![TripSpec {
                first_dep: 100,
                leg_durations: vec![20],
                dwell_times: vec![5, 0],
            }],
        }],
        footpaths: vec![],
        query: QuerySpec {
            ps: 0,
            pt: 1,
            tau: 0,
            max_transfers: 1,
        },
    };
    let tt = render(&spec);

    let routes_at_0 = tt.get_routes_serving_stop(0);
    assert_eq!(routes_at_0.as_ref(), &[0u8]);

    let trip = tt.get_earliest_trip(0u8, 0, 0u8).expect("trip exists");
    assert_eq!(tt.get_arrival_time(trip, 0u8), 100);
    assert_eq!(tt.get_departure_time(trip, 0u8), 105);
    assert_eq!(tt.get_arrival_time(trip, 1u8), 125);
    assert_eq!(tt.get_departure_time(trip, 1u8), 125);
}

#[test]
fn render_emits_transitively_closed_footpaths() {
    use crate::Timetable;
    let spec = NetworkSpec {
        n_stops: 3,
        routes: vec![],
        footpaths: vec![
            FootpathSpec {
                from: 0,
                to: 1,
                walk_time: 3,
            },
            FootpathSpec {
                from: 1,
                to: 2,
                walk_time: 4,
            },
        ],
        query: QuerySpec {
            ps: 0,
            pt: 2,
            tau: 0,
            max_transfers: 1,
        },
    };
    let tt = render(&spec);

    let from_0: Vec<u8> = tt.get_footpaths_from(0u8).into_owned();
    assert!(from_0.contains(&1u8), "direct A->B");
    assert!(from_0.contains(&2u8), "transitive A->C must be present");
    assert_eq!(tt.get_transfer_time(0u8, 2u8), 7);
}
