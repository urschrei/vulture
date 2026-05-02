//! Spec data model, per-layer Hegel composite generators, and the renderer
//! that turns a `NetworkSpec` into a `SimpleTimetable<u8, u8, u16>`.
//!
//! See `mod.rs` for the trip-count convention banner.

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
