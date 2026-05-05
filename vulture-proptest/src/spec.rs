//! Spec data model, per-layer Hegel composite generators, and the renderer
//! that turns a `NetworkSpec` into a `SimpleTimetable<u8, u8, u16>`.
//!
//! See `lib.rs` for the trip-count convention banner.

use vulture::manual::SimpleTimetable;

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
/// contract – generator bugs surface as panics in debug rather than as
/// hidden mis-tests.
pub fn render(spec: &NetworkSpec) -> SimpleTimetable<u8, u8, u16> {
    let mut tt: SimpleTimetable<u8, u8, u16> = SimpleTimetable::new();
    let mut next_trip_id: u16 = 0;

    // Ensure every stop in `0..n_stops` is interned so query stops are
    // looked up cleanly via `stop_idx_of` even when they aren't named by
    // any route or footpath (e.g. an isolated `ps`/`pt`, where the
    // reference solver returns an empty Pareto front).
    for s in 0..spec.n_stops {
        tt = tt.register_stop(s);
    }

    for (route_idx, route) in spec.routes.iter().enumerate() {
        let route_id = u8::try_from(route_idx).expect("route count exceeds u8");
        let stops = &route.stop_sequence;
        assert!(stops.len() >= 2, "route must have >= 2 stops");
        assert_eq!(
            route.trips.first().map(|t| t.leg_durations.len()),
            Some(stops.len() - 1),
            "leg_durations length mismatch",
        );

        let mut trip_owned: Vec<(u16, Vec<(vulture::SecondOfDay, vulture::SecondOfDay)>)> =
            Vec::with_capacity(route.trips.len());
        for trip in &route.trips {
            assert_eq!(trip.leg_durations.len(), stops.len() - 1);
            assert_eq!(trip.dwell_times.len(), stops.len());

            let mut times: Vec<(vulture::SecondOfDay, vulture::SecondOfDay)> =
                Vec::with_capacity(stops.len());
            let arr0 = vulture::SecondOfDay(trip.first_dep as u32);
            let dep0 = arr0 + vulture::Duration(trip.dwell_times[0] as u32);
            times.push((arr0, dep0));
            for i in 1..stops.len() {
                let prev_dep = times[i - 1].1;
                let arr = prev_dep + vulture::Duration(trip.leg_durations[i - 1] as u32);
                let dep = arr + vulture::Duration(trip.dwell_times[i] as u32);
                times.push((arr, dep));
            }

            trip_owned.push((next_trip_id, times));
            next_trip_id = next_trip_id.checked_add(1).expect("trip id overflow");
        }

        let trip_refs: Vec<(u16, &[(vulture::SecondOfDay, vulture::SecondOfDay)])> = trip_owned
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
                tt = tt.transfer_time(from, to, vulture::Duration(walk as u32));
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
    use vulture::Timetable;
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

    let _s0 = tt.stop_idx_of(&0u8);
    let _s1 = tt.stop_idx_of(&1u8);
    let r0 = tt.route_idx_of(&0u8);

    let routes_at_0 = tt.get_routes_serving_stop(_s0);
    assert_eq!(routes_at_0, &[(r0, 0)]);

    // Position 0 of route r0 corresponds to stop 0; position 1 to stop 1.
    let trip = tt
        .get_earliest_trip(r0, vulture::SecondOfDay::ZERO, 0)
        .expect("trip exists");
    assert_eq!(tt.get_arrival_time(trip, 0), vulture::SecondOfDay(100));
    assert_eq!(tt.get_departure_time(trip, 0), vulture::SecondOfDay(105));
    assert_eq!(tt.get_arrival_time(trip, 1), vulture::SecondOfDay(125));
    assert_eq!(tt.get_departure_time(trip, 1), vulture::SecondOfDay(125));
}

#[test]
fn render_emits_transitively_closed_footpaths() {
    use vulture::Timetable;
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

    let s0 = tt.stop_idx_of(&0u8);
    let s1 = tt.stop_idx_of(&1u8);
    let s2 = tt.stop_idx_of(&2u8);

    let from_0: Vec<vulture::StopIdx> = tt.get_footpaths_from(s0).to_vec();
    assert!(from_0.contains(&s1), "direct A->B");
    assert!(from_0.contains(&s2), "transitive A->C must be present");
    assert_eq!(tt.get_transfer_time(s0, s2), vulture::Duration(7));
}

// -------- Hegel generators ---------------------------------------------

use hegel::generators;

/// Per-layer envelope sizes. Used to parameterize the shared
/// `network_spec` generator.
#[derive(Debug, Clone, Copy)]
pub struct LayerBounds {
    pub n_stops_min: u8,
    pub n_stops_max: u8,
    pub routes_max: u8,
    pub trips_max: u8,
    pub footpaths_max: u8,
    pub stop_seq_max: u8,
    pub allow_footpaths: bool,
    /// When true, a route's stop_sequence may contain duplicate stop ids
    /// (a "loop route" – the trip revisits the same stop). When false,
    /// every stop in a route's sequence is distinct.
    pub allow_loops: bool,
}

pub fn layer1_bounds() -> LayerBounds {
    LayerBounds {
        n_stops_min: 2,
        n_stops_max: 4,
        routes_max: 2,
        trips_max: 2,
        footpaths_max: 0,
        stop_seq_max: 4,
        allow_footpaths: false,
        allow_loops: false,
    }
}

pub fn layer2_bounds() -> LayerBounds {
    LayerBounds {
        n_stops_min: 2,
        n_stops_max: 5,
        routes_max: 3,
        trips_max: 2,
        footpaths_max: 4,
        stop_seq_max: 4,
        allow_footpaths: true,
        allow_loops: false,
    }
}

pub fn layer3_bounds() -> LayerBounds {
    LayerBounds {
        n_stops_min: 2,
        n_stops_max: 6,
        routes_max: 4,
        trips_max: 3,
        footpaths_max: 6,
        stop_seq_max: 4,
        allow_footpaths: true,
        allow_loops: true,
    }
}

#[hegel::composite]
fn route_spec(tc: hegel::TestCase, n_stops: u8, bounds: LayerBounds) -> RouteSpec {
    let stop_seq_min: u8 = 2;
    let stop_seq_cap = bounds.stop_seq_max.min(n_stops);
    let stop_count = tc.draw(
        generators::integers::<u8>()
            .min_value(stop_seq_min)
            .max_value(stop_seq_cap),
    );
    let stop_sequence: Vec<u8> = if bounds.allow_loops {
        tc.draw(
            generators::vecs(
                generators::integers::<u8>()
                    .min_value(0)
                    .max_value(n_stops - 1),
            )
            .min_size(stop_count as usize)
            .max_size(stop_count as usize),
        )
    } else {
        tc.draw(
            generators::vecs(
                generators::integers::<u8>()
                    .min_value(0)
                    .max_value(n_stops - 1),
            )
            .min_size(stop_count as usize)
            .max_size(stop_count as usize)
            .unique(true),
        )
    };

    let leg_count = stop_sequence.len() - 1;
    let leg_durations: Vec<u16> = tc.draw(
        generators::vecs(generators::integers::<u16>().min_value(1).max_value(80))
            .min_size(leg_count)
            .max_size(leg_count),
    );

    let dwell_times: Vec<u16> = tc.draw(
        generators::vecs(generators::integers::<u16>().min_value(0).max_value(20))
            .min_size(stop_sequence.len())
            .max_size(stop_sequence.len()),
    );

    let trip_count = tc.draw(
        generators::integers::<u8>()
            .min_value(1)
            .max_value(bounds.trips_max),
    );
    let mut trips: Vec<TripSpec> = Vec::with_capacity(trip_count as usize);
    let mut last_dep: u16 = 0;
    for _ in 0..trip_count {
        let max_first_dep: u16 = 400;
        let next_dep = tc.draw(
            generators::integers::<u16>()
                .min_value(last_dep)
                .max_value(max_first_dep),
        );
        trips.push(TripSpec {
            first_dep: next_dep,
            leg_durations: leg_durations.clone(),
            dwell_times: dwell_times.clone(),
        });
        last_dep = next_dep;
    }

    RouteSpec {
        stop_sequence,
        trips,
    }
}

#[hegel::composite]
fn footpath_spec(tc: hegel::TestCase, n_stops: u8) -> FootpathSpec {
    let from = tc.draw(
        generators::integers::<u8>()
            .min_value(0)
            .max_value(n_stops - 1),
    );
    // Generate `to` distinct from `from` without rejection sampling.
    let raw = tc.draw(
        generators::integers::<u8>()
            .min_value(0)
            .max_value(n_stops.saturating_sub(2)),
    );
    let to = if raw >= from { raw + 1 } else { raw };
    let walk_time = tc.draw(generators::integers::<u16>().min_value(1).max_value(300));
    FootpathSpec {
        from,
        to,
        walk_time,
    }
}

#[hegel::composite]
pub fn network_spec(tc: hegel::TestCase, bounds: LayerBounds) -> NetworkSpec {
    let n_stops = tc.draw(
        generators::integers::<u8>()
            .min_value(bounds.n_stops_min)
            .max_value(bounds.n_stops_max),
    );

    let route_count = tc.draw(
        generators::integers::<u8>()
            .min_value(1)
            .max_value(bounds.routes_max),
    );
    let mut routes: Vec<RouteSpec> = Vec::with_capacity(route_count as usize);
    for _ in 0..route_count {
        routes.push(tc.draw(route_spec(n_stops, bounds)));
    }

    let footpaths: Vec<FootpathSpec> = if bounds.allow_footpaths && n_stops >= 2 {
        let fp_count = tc.draw(
            generators::integers::<u8>()
                .min_value(0)
                .max_value(bounds.footpaths_max),
        );
        let mut v = Vec::with_capacity(fp_count as usize);
        for _ in 0..fp_count {
            v.push(tc.draw(footpath_spec(n_stops)));
        }
        v
    } else {
        Vec::new()
    };

    let ps = tc.draw(
        generators::integers::<u8>()
            .min_value(0)
            .max_value(n_stops - 1),
    );
    let pt = tc.draw(
        generators::integers::<u8>()
            .min_value(0)
            .max_value(n_stops - 1),
    );
    let tau = tc.draw(generators::integers::<u16>().min_value(0).max_value(500));
    let max_transfers = tc.draw(generators::integers::<u8>().min_value(1).max_value(5));

    NetworkSpec {
        n_stops,
        routes,
        footpaths,
        query: QuerySpec {
            ps,
            pt,
            tau,
            max_transfers,
        },
    }
}
