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
