# Property-Based Test Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Hegel-based property-test harness in `raptor/src/proptest_support/` that compares `Timetable::raptor` output against a brute-force reference solver, designed to expose v0.2.0 soundness issues A, B, C, D and prevent regressions as Phase 0 fixes land.

**Architecture:** A `NetworkSpec` data model + per-layer Hegel composite generators feed a deterministic `render()` that emits `SimpleTimetable<u8, u8, u16>`. A self-contained time-expanded multi-criterion Dijkstra in `reference.rs` produces the ground-truth Pareto front of `(arrival, trips)`. Three `#[hegel::test]` functions (Layer 1 green on baseline; Layers 2 & 3 `#[ignore]`-flagged because they're expected to fail until Phase 0 lands) compare the algorithm's output (after a defensive `raptor_front` Pareto filter that masks issue F) against the reference.

**Tech Stack:** Rust 2024 edition, jujutsu for VCS, `cargo nextest r` for tests, Hegel (git dep) for property tests, std-only reference solver.

**Companion docs:** `docs/superpowers/specs/2026-05-02-proptest-harness-design.md` (design),
`docs/proptest.md` (handoff brief), `soundness.md` (issue catalogue).

---

## Conventions all tasks follow

- Each step that changes code also commits via `jj fix && jj commit -m "[WIP: claude] ..."`.
- After every code change, run `cargo fmt -p raptor` and `cargo clippy -p raptor --tests` and fix warnings before committing.
- Tests are run via `cargo nextest r`, not `cargo test`. Hegel-ignored tests run via `cargo nextest r --run-ignored all`.
- All UK spelling in docs/comments. No emoji.
- Property-test convention banner (the "transfers parameter is trip count" paragraph) is mandatory at the top of every file in `proptest_support/`.

---

## Task 1: Wire up Hegel dependency, gitignore, and module skeleton

**Files:**
- Modify: `raptor/Cargo.toml`
- Modify: `.gitignore`
- Modify: `raptor/src/lib.rs`
- Create: `raptor/src/proptest_support/mod.rs`
- Create: `raptor/src/proptest_support/spec.rs`
- Create: `raptor/src/proptest_support/reference.rs`

- [ ] **Step 1: Verify clean working copy**

```bash
jj st
```

Expected: `The working copy has no changes.` If not, stop and ask the user.

- [ ] **Step 2: Add Hegel to dev-dependencies**

In `raptor/Cargo.toml`, locate the `[dev-dependencies]` block (currently containing `anyhow`, `criterion`, `env_logger`, `humantime`) and add:

```toml
hegel = { git = "https://github.com/hegeldev/hegel-rust" }
```

- [ ] **Step 3: Add `.hegel/` to gitignore**

Replace the contents of `.gitignore` with:

```
/target
.hegel/
```

- [ ] **Step 4: Create the module skeleton with the convention banner**

Create `raptor/src/proptest_support/mod.rs` with content:

```rust
//! Property-based test harness comparing `Timetable::raptor` against a
//! brute-force reference solver on randomly generated networks.
//!
//! TRIP-COUNT CONVENTION
//!
//! `Timetable::raptor`'s `transfers` parameter is the trip count, not the
//! transfer count, despite its name. A journey "board R1 to B, board R2 to D"
//! has 2 trips and 1 transfer; the trait's `transfers=2` admits this journey.
//! Similarly, `Journey::plan.len()` is the trip count.
//!
//! Both this harness and the reference solver use trip counts everywhere.
//! The Pareto front compared in property tests is over `(arrival, trip_count)`.
//!
//! Layer-to-issue mapping is documented in `README.md` next to this file.

pub mod reference;
pub mod spec;
```

Create `raptor/src/proptest_support/spec.rs` with content:

```rust
//! Spec data model, per-layer Hegel composite generators, and the renderer
//! that turns a `NetworkSpec` into a `SimpleTimetable<u8, u8, u16>`.
//!
//! See `mod.rs` for the trip-count convention banner.
```

Create `raptor/src/proptest_support/reference.rs` with content:

```rust
//! Time-expanded multi-criterion Dijkstra reference solver.
//!
//! Optimise nothing. If we ever debug this, we have gone wrong somewhere.
//!
//! See `mod.rs` for the trip-count convention banner.
```

- [ ] **Step 5: Wire the module into `raptor/src/lib.rs`**

In `raptor/src/lib.rs`, locate the existing `#[cfg(test)] mod test;` line near the top of the file and add a new line directly below it:

```rust
#[cfg(test)]
mod proptest_support;
```

Result should look like:

```rust
#[cfg(test)]
mod test;

#[cfg(test)]
mod proptest_support;
```

- [ ] **Step 6: Verify compilation**

```bash
cargo nextest r -p raptor --no-run
```

Expected: build succeeds with no errors. Hegel may take a minute on first build.

- [ ] **Step 7: Commit**

```bash
jj fix
jj commit -m "[WIP: claude] Wire up hegel dep and proptest_support module skeleton"
```

---

## Task 2: Define `NetworkSpec` data types in `spec.rs`

**Files:**
- Modify: `raptor/src/proptest_support/spec.rs`

- [ ] **Step 1: Add the data types below the file-level doc comment**

Append to `raptor/src/proptest_support/spec.rs`:

```rust
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
```

- [ ] **Step 2: Verify compilation**

```bash
cargo nextest r -p raptor --no-run
```

Expected: build succeeds.

- [ ] **Step 3: Format, lint, commit**

```bash
cargo fmt -p raptor
cargo clippy -p raptor --tests
jj fix
jj commit -m "[WIP: claude] Add NetworkSpec data types for proptest harness"
```

---

## Task 3: Implement and unit-test `close_footpaths`

**Files:**
- Modify: `raptor/src/proptest_support/spec.rs`

`close_footpaths` runs Floyd–Warshall over min-plus to produce the all-pairs
transitive closure of the sparse footpath list. It is used by both `render`
(Task 4) and the reference solver (Task 5).

- [ ] **Step 1: Write the failing test**

Append to `raptor/src/proptest_support/spec.rs`:

```rust
#[test]
fn close_footpaths_two_hop_chain() {
    let spec = NetworkSpec {
        n_stops: 3,
        routes: vec![],
        footpaths: vec![
            FootpathSpec { from: 0, to: 1, walk_time: 5 },
            FootpathSpec { from: 1, to: 2, walk_time: 7 },
        ],
        query: QuerySpec { ps: 0, pt: 2, tau: 0, max_transfers: 1 },
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
            FootpathSpec { from: 0, to: 1, walk_time: 10 },
            FootpathSpec { from: 0, to: 1, walk_time: 4 },
        ],
        query: QuerySpec { ps: 0, pt: 1, tau: 0, max_transfers: 1 },
    };
    let closed = close_footpaths(&spec);
    assert_eq!(closed[0][1], Some(4));
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo nextest r -p raptor proptest_support::spec::close_footpaths
```

Expected: build error — `close_footpaths` is not defined.

- [ ] **Step 3: Implement `close_footpaths`**

Insert before the `#[test]` blocks added in Step 1:

```rust
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
```

- [ ] **Step 4: Run the tests**

```bash
cargo nextest r -p raptor proptest_support::spec::close_footpaths
```

Expected: both tests pass.

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt -p raptor
cargo clippy -p raptor --tests
jj fix
jj commit -m "[WIP: claude] Add close_footpaths min-plus Floyd-Warshall helper"
```

---

## Task 4: Implement and unit-test `render`

**Files:**
- Modify: `raptor/src/proptest_support/spec.rs`

`render` turns a `NetworkSpec` into a `SimpleTimetable<u8, u8, u16>` by
prefix-summing dwell/leg durations onto `first_dep` and emitting transitively
closed footpaths.

- [ ] **Step 1: Write the failing test**

Append to `raptor/src/proptest_support/spec.rs`:

```rust
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
        query: QuerySpec { ps: 0, pt: 1, tau: 0, max_transfers: 1 },
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
            FootpathSpec { from: 0, to: 1, walk_time: 3 },
            FootpathSpec { from: 1, to: 2, walk_time: 4 },
        ],
        query: QuerySpec { ps: 0, pt: 2, tau: 0, max_transfers: 1 },
    };
    let tt = render(&spec);

    let from_0: Vec<u8> = tt.get_footpaths_from(0u8).into_owned();
    assert!(from_0.contains(&1u8), "direct A->B");
    assert!(from_0.contains(&2u8), "transitive A->C must be present");
    assert_eq!(tt.get_transfer_time(0u8, 2u8), 7);
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo nextest r -p raptor proptest_support::spec::render
```

Expected: build error — `render` is not defined.

- [ ] **Step 3: Implement `render`**

Insert before the `#[test]` blocks:

```rust
use crate::simple::SimpleTimetable;

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
```

- [ ] **Step 4: Run the tests**

```bash
cargo nextest r -p raptor proptest_support::spec
```

Expected: all four spec tests pass.

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt -p raptor
cargo clippy -p raptor --tests
jj fix
jj commit -m "[WIP: claude] Add NetworkSpec renderer with closed footpaths"
```

---

## Task 5: Reference solver — node set construction

**Files:**
- Modify: `raptor/src/proptest_support/reference.rs`

The reference solver builds an explicit `(stop, time)` node set first, then
runs Dijkstra over it. This task adds the node-set construction and a small
struct holding precomputed trip schedules. Edge expansion lands in Task 6.

- [ ] **Step 1: Write the failing test**

Append to `raptor/src/proptest_support/reference.rs`:

```rust
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
            footpaths: vec![FootpathSpec { from: 1, to: 2, walk_time: 7 }],
            query: QuerySpec { ps: 0, pt: 2, tau: 0, max_transfers: 2 },
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
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo nextest r -p raptor proptest_support::reference
```

Expected: build error — `Prep` is not defined.

- [ ] **Step 3: Implement `Prep::build`**

Insert at the top of `raptor/src/proptest_support/reference.rs` (below the file-level doc comment):

```rust
use std::collections::{BTreeMap, BTreeSet};

use crate::proptest_support::spec::{NetworkSpec, close_footpaths};

/// Pre-computed per-trip stop schedules: `(stop, arrival, departure)` per
/// stop in the trip's stop sequence.
type TripSchedule = Vec<(u8, u16, u16)>;

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

        Prep { nodes, trips, footpaths, n_stops: spec.n_stops }
    }
}
```

- [ ] **Step 4: Run the tests**

```bash
cargo nextest r -p raptor proptest_support::reference
```

Expected: pass.

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt -p raptor
cargo clippy -p raptor --tests
jj fix
jj commit -m "[WIP: claude] Add reference solver Prep node-set construction"
```

---

## Task 6: Reference solver — Dijkstra and `reference_solve`

**Files:**
- Modify: `raptor/src/proptest_support/reference.rs`

This is the meat of the reference solver: multi-criterion Dijkstra over
`(stop, time)` nodes, criterion `trips_used`. Since the time component is
encoded into the node, we keep `min_trips` per node — sufficient for the
Pareto-front semantics (proof: any state reachable from `(s, t, k)` is also
reachable from `(s, t, k')` with `k' < k` via the same edges).

- [ ] **Step 1: Write the failing tests**

Append inside the existing `mod tests` block in
`raptor/src/proptest_support/reference.rs`:

```rust
    use std::collections::BTreeSet;

    fn front(items: &[(u16, u8)]) -> BTreeSet<(u16, u8)> {
        items.iter().copied().collect()
    }

    #[test]
    fn ps_eq_pt_returns_tau_zero() {
        let spec = NetworkSpec {
            n_stops: 2,
            routes: vec![],
            footpaths: vec![],
            query: QuerySpec { ps: 0, pt: 0, tau: 42, max_transfers: 3 },
        };
        let r = reference_solve(&spec, 0, 0, 42, 3);
        assert_eq!(r, front(&[(42, 0)]));
    }

    #[test]
    fn disconnected_returns_empty() {
        let spec = NetworkSpec {
            n_stops: 2,
            routes: vec![],
            footpaths: vec![],
            query: QuerySpec { ps: 0, pt: 1, tau: 0, max_transfers: 3 },
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
            query: QuerySpec { ps: 0, pt: 1, tau: 0, max_transfers: 3 },
        };
        let r = reference_solve(&spec, 0, 1, 0, 3);
        assert_eq!(r, front(&[(30, 1)]));
    }

    #[test]
    fn walk_only_journey() {
        let spec = NetworkSpec {
            n_stops: 2,
            routes: vec![],
            footpaths: vec![FootpathSpec { from: 0, to: 1, walk_time: 5 }],
            query: QuerySpec { ps: 0, pt: 1, tau: 100, max_transfers: 3 },
        };
        let r = reference_solve(&spec, 0, 1, 100, 3);
        assert_eq!(r, front(&[(105, 0)]));
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
            footpaths: vec![FootpathSpec { from: 0, to: 1, walk_time: 5 }],
            query: QuerySpec { ps: 0, pt: 2, tau: 0, max_transfers: 3 },
        };
        let r = reference_solve(&spec, 0, 2, 0, 3);
        assert_eq!(r, front(&[(70, 1)]));
    }

    #[test]
    fn pareto_front_two_options_one_dominated() {
        // R1 direct: ps=0 -> pt=1, depart 0, arrive 100, 1 trip.
        // R2/R3 via stop 2: also 1 trip via single faster route.
        // Use two routes with same trip count -> only fastest survives Pareto filter.
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
            query: QuerySpec { ps: 0, pt: 1, tau: 0, max_transfers: 3 },
        };
        let r = reference_solve(&spec, 0, 1, 0, 3);
        assert_eq!(r, front(&[(80, 1)]));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo nextest r -p raptor proptest_support::reference::tests
```

Expected: build error — `reference_solve` is not defined.

- [ ] **Step 3: Implement `reference_solve`**

Insert after the `Prep` impl in `raptor/src/proptest_support/reference.rs`:

```rust
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::ops::Bound;

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
        let mut out = BTreeSet::new();
        out.insert((tau, 0));
        return out;
    }

    let prep = Prep::build(spec, ps, tau);

    let mut min_trips: BTreeMap<(u8, u16), u8> = BTreeMap::new();
    let mut heap: BinaryHeap<Reverse<(u16, u8, u8)>> = BinaryHeap::new();

    let start = (ps, tau);
    min_trips.insert(start, 0);
    heap.push(Reverse((tau, 0, ps)));

    let mut relax = |min_trips: &mut BTreeMap<(u8, u16), u8>,
                     heap: &mut BinaryHeap<Reverse<(u16, u8, u8)>>,
                     stop: u8,
                     t: u16,
                     trips: u8| {
        let entry = min_trips.entry((stop, t)).or_insert(u8::MAX);
        if trips < *entry {
            *entry = trips;
            heap.push(Reverse((t, trips, stop)));
        }
    };

    while let Some(Reverse((t, trips, stop))) = heap.pop() {
        if min_trips.get(&(stop, t)).copied() != Some(trips) {
            continue;
        }

        // Wait edge: next adjacent timepoint at the same stop.
        if let Some(stop_times) = prep.nodes.get(&stop) {
            if let Some(&next_t) = stop_times
                .range((Bound::Excluded(t), Bound::Unbounded))
                .next()
            {
                relax(&mut min_trips, &mut heap, stop, next_t, trips);
            }
        }

        // Walk edges via transitively-closed footpaths.
        for to in 0..prep.n_stops {
            if to == stop {
                continue;
            }
            if let Some(walk) = prep.footpaths[stop as usize][to as usize] {
                if let Some(new_t) = t.checked_add(walk) {
                    relax(&mut min_trips, &mut heap, to, new_t, trips);
                }
            }
        }

        // Ride edges: if we are at a trip's exact departure timepoint.
        for sched in &prep.trips {
            for i in 0..sched.len().saturating_sub(1) {
                if sched[i].0 == stop && sched[i].2 == t {
                    let (next_stop, next_arr, _) = sched[i + 1];
                    relax(&mut min_trips, &mut heap, next_stop, next_arr, trips);
                }
            }
        }

        // Board edges: pay +1 trip to jump to a trip departure at this stop.
        if trips < max_trips {
            for sched in &prep.trips {
                for &(s, _, dep) in sched {
                    if s == stop && dep >= t {
                        relax(&mut min_trips, &mut heap, stop, dep, trips + 1);
                    }
                }
            }
        }
    }

    let mut at_pt: Vec<(u16, u8)> = min_trips
        .iter()
        .filter(|((s, _), _)| *s == pt)
        .filter(|(_, &k)| k <= max_trips)
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
```

Note the closure captures: Rust will complain about mutable-borrow conflicts
if `relax` is defined as a `FnMut` closure capturing `min_trips`/`heap`. Move
the closure inside the loop or rewrite as an inline `match` if borrow-checker
errors arise. The inline-relax path:

```rust
// (Replacement for the closure if borrow-check fights you.)
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
```

Then call sites become `relax(&mut min_trips, &mut heap, …)`. Use whichever
form clippy is happiest with.

- [ ] **Step 4: Run the tests**

```bash
cargo nextest r -p raptor proptest_support::reference::tests
```

Expected: all six reference tests pass.

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt -p raptor
cargo clippy -p raptor --tests
jj fix
jj commit -m "[WIP: claude] Add reference_solve multi-criterion Dijkstra"
```

---

## Task 7: `raptor_front` comparison helper

**Files:**
- Modify: `raptor/src/proptest_support/mod.rs`

`raptor_front` projects a `Vec<Journey>` to a `BTreeSet<(arrival, trips)>`
Pareto front, applying the output-side filter the algorithm itself ought to
be doing (issue F is masked here intentionally — see design doc).

- [ ] **Step 1: Write the failing test**

Append to `raptor/src/proptest_support/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Journey;
    use std::collections::BTreeSet;

    fn j(arrival: usize, plan: Vec<(u8, u8)>) -> Journey<u8, u8> {
        Journey { plan, arrival }
    }

    #[test]
    fn raptor_front_drops_dominated_higher_trip_journeys() {
        let journeys = vec![
            j(100, vec![(0, 1)]),         // 1 trip, arr=100
            j(100, vec![(0, 2), (1, 3)]), // 2 trips, arr=100 — dominated
            j(80, vec![(0, 2), (1, 3)]),  // 2 trips, arr=80 — non-dominated
        ];
        let f = raptor_front(&journeys);
        let expected: BTreeSet<(u16, u8)> = [(100u16, 1u8), (80u16, 2u8)].into_iter().collect();
        assert_eq!(f, expected);
    }

    #[test]
    fn raptor_front_empty_input_is_empty() {
        let f = raptor_front::<u8, u8>(&[]);
        assert!(f.is_empty());
    }

    #[test]
    fn raptor_front_strict_monotonicity_drops_ties() {
        // Same arrival, more trips: drop.
        let journeys = vec![
            j(50, vec![(0, 1)]),
            j(50, vec![(0, 1), (1, 2)]),
        ];
        let f = raptor_front(&journeys);
        let expected: BTreeSet<(u16, u8)> = [(50u16, 1u8)].into_iter().collect();
        assert_eq!(f, expected);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo nextest r -p raptor proptest_support::tests
```

Expected: build error — `raptor_front` is not defined.

- [ ] **Step 3: Implement `raptor_front`**

Insert before the `mod tests` block in `raptor/src/proptest_support/mod.rs`:

```rust
use std::collections::BTreeSet;

use crate::Journey;

/// Project the algorithm's `Vec<Journey>` to a Pareto front of
/// `(arrival, trip_count)`, sorted by trip count ascending, keeping only
/// points where arrival is *strictly* less than the best seen so far.
///
/// This applies the output-side Pareto filter the algorithm should be doing
/// itself (soundness issue F). Filtering on the harness side intentionally
/// masks F so the front-equality property isolates issues A, B, C, D.
pub fn raptor_front<R, S>(journeys: &[Journey<R, S>]) -> BTreeSet<(u16, u8)> {
    let mut points: Vec<(u16, u8)> = journeys
        .iter()
        .map(|j| {
            let arr = u16::try_from(j.arrival)
                .expect("arrival exceeds u16::MAX — generator range exceeded?");
            let k = u8::try_from(j.plan.len())
                .expect("plan length exceeds u8::MAX — should never happen");
            (arr, k)
        })
        .collect();
    points.sort_by_key(|&(t, k)| (k, t));
    let mut best = u16::MAX;
    let mut out = BTreeSet::new();
    for (arr, k) in points {
        if arr < best {
            best = arr;
            out.insert((arr, k));
        }
    }
    out
}
```

- [ ] **Step 4: Run the tests**

```bash
cargo nextest r -p raptor proptest_support::tests
```

Expected: all three tests pass.

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt -p raptor
cargo clippy -p raptor --tests
jj fix
jj commit -m "[WIP: claude] Add raptor_front Pareto-projection helper"
```

---

## Task 8: Layer 1 generator and property test

**Files:**
- Modify: `raptor/src/proptest_support/spec.rs`
- Modify: `raptor/src/proptest_support/mod.rs`

Layer 1 covers the regime the existing hand-written tests cover: small
networks, no footpaths. It must pass on the v0.2.0 baseline.

- [ ] **Step 1: Add `LayerBounds` and the layer-1 bounds in `spec.rs`**

Append to `raptor/src/proptest_support/spec.rs`:

```rust
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
    }
}
```

- [ ] **Step 2: Add the shared `network_spec` composite generator**

Append to `raptor/src/proptest_support/spec.rs`:

```rust
use hegel::generators::{self, Generator};

#[hegel::composite]
fn route_spec(
    tc: hegel::TestCase,
    n_stops: u8,
    bounds: LayerBounds,
) -> RouteSpec {
    let stop_seq_min: u8 = 2;
    let stop_seq_cap = bounds.stop_seq_max.min(n_stops);
    let stop_count = tc.draw(
        generators::integers::<u8>()
            .min_value(stop_seq_min)
            .max_value(stop_seq_cap),
    );
    let stop_sequence: Vec<u8> = tc.draw(
        generators::vecs(
            generators::integers::<u8>()
                .min_value(0)
                .max_value(n_stops - 1),
        )
        .min_size(stop_count as usize)
        .max_size(stop_count as usize)
        .unique(),
    );

    let leg_count = stop_sequence.len() - 1;
    let leg_durations: Vec<u16> = tc.draw(
        generators::vecs(
            generators::integers::<u16>()
                .min_value(1)
                .max_value(80),
        )
        .min_size(leg_count)
        .max_size(leg_count),
    );

    let dwell_times: Vec<u16> = tc.draw(
        generators::vecs(
            generators::integers::<u16>()
                .min_value(0)
                .max_value(20),
        )
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

    RouteSpec { stop_sequence, trips }
}

#[hegel::composite]
fn footpath_spec(tc: hegel::TestCase, n_stops: u8) -> FootpathSpec {
    let from = tc.draw(generators::integers::<u8>().min_value(0).max_value(n_stops - 1));
    // Generate `to` distinct from `from` without rejection sampling.
    let raw = tc.draw(
        generators::integers::<u8>()
            .min_value(0)
            .max_value(n_stops.saturating_sub(2)),
    );
    let to = if raw >= from { raw + 1 } else { raw };
    let walk_time = tc.draw(generators::integers::<u16>().min_value(1).max_value(300));
    FootpathSpec { from, to, walk_time }
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

    let ps = tc.draw(generators::integers::<u8>().min_value(0).max_value(n_stops - 1));
    let pt = tc.draw(generators::integers::<u8>().min_value(0).max_value(n_stops - 1));
    let tau = tc.draw(generators::integers::<u16>().min_value(0).max_value(500));
    let max_transfers = tc.draw(generators::integers::<u8>().min_value(1).max_value(5));

    NetworkSpec {
        n_stops,
        routes,
        footpaths,
        query: QuerySpec { ps, pt, tau, max_transfers },
    }
}
```

- [ ] **Step 3: Add the layer-1 property test in `mod.rs`**

Append to `raptor/src/proptest_support/mod.rs`:

```rust
use crate::Timetable;

fn run_property(tc: &hegel::TestCase, spec: &spec::NetworkSpec) {
    let timetable = spec::render(spec);
    let ours = timetable.raptor(
        spec.query.max_transfers as usize,
        spec.query.tau as usize,
        spec.query.ps,
        spec.query.pt,
    );
    let theirs = reference::reference_solve(
        spec,
        spec.query.ps,
        spec.query.pt,
        spec.query.tau,
        spec.query.max_transfers,
    );
    let our_front = raptor_front(&ours);
    if our_front != theirs {
        tc.note(&format!("spec: {:#?}", spec));
        tc.note(&format!("raptor:     {:?}", ours));
        tc.note(&format!("ours_front: {:?}", our_front));
        tc.note(&format!("theirs:     {:?}", theirs));
    }
    assert_eq!(our_front, theirs);
}

#[hegel::test]
fn layer1_matches_reference(tc: hegel::TestCase) {
    let spec = tc.draw(spec::network_spec(spec::layer1_bounds()));
    run_property(&tc, &spec);
}
```

- [ ] **Step 4: Run the layer-1 test (must pass on baseline)**

```bash
cargo nextest r -p raptor proptest_support::layer1_matches_reference
```

Expected: pass. If it fails, the harness has a bug — investigate before
proceeding. The most likely source of a layer-1 failure is the renderer or
the reference solver, not the algorithm.

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt -p raptor
cargo clippy -p raptor --tests
jj fix
jj commit -m "[WIP: claude] Add layer-1 hegel generator and property test"
```

---

## Task 9: Layer 2 generator and `#[ignore]`-flagged property test

**Files:**
- Modify: `raptor/src/proptest_support/spec.rs`
- Modify: `raptor/src/proptest_support/mod.rs`

Layer 2 adds footpaths. Targets soundness issues A, B, C, D. Expected to fail
on v0.2.0 — `#[ignore]` keeps `cargo nextest r` green for the project's jj
workflow.

- [ ] **Step 1: Add `layer2_bounds()` in `spec.rs`**

Append to `raptor/src/proptest_support/spec.rs`:

```rust
pub fn layer2_bounds() -> LayerBounds {
    LayerBounds {
        n_stops_min: 2,
        n_stops_max: 5,
        routes_max: 3,
        trips_max: 2,
        footpaths_max: 4,
        stop_seq_max: 4,
        allow_footpaths: true,
    }
}
```

- [ ] **Step 2: Add the layer-2 test in `mod.rs`**

Append to `raptor/src/proptest_support/mod.rs`:

```rust
#[ignore = "expected to fail on v0.2.0; targets soundness issues A, B, C, D"]
#[hegel::test]
fn layer2_matches_reference(tc: hegel::TestCase) {
    let spec = tc.draw(spec::network_spec(spec::layer2_bounds()));
    run_property(&tc, &spec);
}
```

- [ ] **Step 3: Verify the test is registered but ignored by default**

```bash
cargo nextest r -p raptor proptest_support::layer2_matches_reference
```

Expected: 1 test ignored, 0 passed, 0 failed.

- [ ] **Step 4: Verify the test fails when run with `--run-ignored`**

```bash
cargo nextest r -p raptor proptest_support::layer2_matches_reference --run-ignored all
```

Expected: failure with a shrunk `NetworkSpec` printed via `tc.note`. Read the
spec — it should be small (ideally 2–3 stops, 1 route, 1 footpath). Confirm
the failure mode looks like one of soundness issues A or B (e.g., the
optimal journey involves walking from the source to a stop and then boarding,
or walking after disembarking before the algorithm re-scans).

If the test passes when un-ignored, the harness has a bug — Layers 2 must
detect the known soundness problems on baseline. Investigate before
proceeding.

- [ ] **Step 5: Format, lint, commit**

```bash
cargo fmt -p raptor
cargo clippy -p raptor --tests
jj fix
jj commit -m "[WIP: claude] Add layer-2 hegel test (ignored; expected to fail on v0.2.0)"
```

---

## Task 10: Layer 3 generator and `#[ignore]`-flagged property test

**Files:**
- Modify: `raptor/src/proptest_support/spec.rs`
- Modify: `raptor/src/proptest_support/mod.rs`

Layer 3 is the catch-all with the largest input space and a higher
`test_cases` setting for fuller coverage in CI.

- [ ] **Step 1: Add `layer3_bounds()` in `spec.rs`**

Append to `raptor/src/proptest_support/spec.rs`:

```rust
pub fn layer3_bounds() -> LayerBounds {
    LayerBounds {
        n_stops_min: 2,
        n_stops_max: 6,
        routes_max: 4,
        trips_max: 3,
        footpaths_max: 6,
        stop_seq_max: 4,
        allow_footpaths: true,
    }
}
```

- [ ] **Step 2: Add the layer-3 test in `mod.rs`**

Append to `raptor/src/proptest_support/mod.rs`:

```rust
#[ignore = "expected to fail on v0.2.0; full-network coverage"]
#[hegel::test(test_cases = 500)]
fn layer3_matches_reference(tc: hegel::TestCase) {
    let spec = tc.draw(spec::network_spec(spec::layer3_bounds()));
    run_property(&tc, &spec);
}
```

- [ ] **Step 3: Verify the test runs with `--run-ignored` and stays under budget**

```bash
time cargo nextest r -p raptor proptest_support::layer3_matches_reference --run-ignored all
```

Expected: failure with a shrunk counterexample. Total wall-clock under 10s
on a developer laptop. If it blows the budget, drop `test_cases` to 200
in `mod.rs` and re-run; the budget cap is more important than the case count.

- [ ] **Step 4: Format, lint, commit**

```bash
cargo fmt -p raptor
cargo clippy -p raptor --tests
jj fix
jj commit -m "[WIP: claude] Add layer-3 hegel test (ignored; expected to fail on v0.2.0)"
```

---

## Task 11: Write the README

**Files:**
- Create: `raptor/src/proptest_support/README.md`

- [ ] **Step 1: Write the README**

Create `raptor/src/proptest_support/README.md` with content:

```markdown
# Property-based test harness

This module exists to detect and prevent soundness regressions in
`Timetable::raptor` by comparing its output against a brute-force reference
solver on randomly generated networks. Powered by Hegel.

## Trip-count convention

`Timetable::raptor`'s `transfers` parameter is the trip count, not the
transfer count, despite its name. A journey "board R1 to B, board R2 to D"
has 2 trips and 1 transfer; the trait's `transfers=2` admits this journey.
Similarly, `Journey::plan.len()` is the trip count.

The Pareto front compared in property tests is over `(arrival, trip_count)`.

## Generator layers

| Layer | Stops | Routes | Trips/route | Footpaths | Status on v0.2.0 |
|-------|-------|--------|-------------|-----------|-------------------|
| 1     | 2..=4 | 1..=2  | 1..=2       | 0         | passes            |
| 2     | 2..=5 | 1..=3  | 1..=2       | 1..=4     | fails (issues A, B, C, D) |
| 3     | 2..=6 | 1..=4  | 1..=3       | 0..=6     | fails             |

Layers 2 and 3 are `#[ignore]`-flagged so `cargo nextest r` stays green on
the v0.2.0 baseline. Run them with:

```
cargo nextest r -p raptor proptest_support --run-ignored all
```

When Phase 0 of `docs/roadmap/roadmap.md` lands, remove the `#[ignore]`
attributes — the deliverable for v0.3 includes "the previously-ignored
property tests now pass."

## Layer-to-soundness-issue map

See `soundness.md` at the repo root for the full issue catalogue.

| Soundness issue | Layer that detects it | Notes |
|-----------------|-----------------------|-------|
| A — labels not carried forward | 2 | Triggered by any journey reaching a stop in round k−1 then walking in round k. |
| B — no footpath relaxation from source | 2 | Triggered when optimal journey starts with a walk. |
| C — τ\* not updated in footpath stage | 2 (indirect) | Manifests as inflated arrivals via leaky pruning; most easily seen on multi-round journeys with footpaths. |
| D — no target pruning in footpath stage | 2 (indirect) | Wastes work but is correctness-neutral if F is also fixed; the harness may not directly demonstrate D in isolation. |
| E — GTFS adapter route-pattern conflation | (out of scope) | Needs a separate harness over `GtfsTimetable`. |
| F — output not Pareto-filtered | masked by `raptor_front` | Intentionally hidden in this harness so the front-equality property isolates A/B/C/D. A separate test once Phase 0 lands. |
| G — non-saturating Tau arithmetic | (out of scope) | Generator ranges keep `tau`, `walk_time` small enough to never trigger overflow; a targeted unit test is more appropriate. |
| H — footpath transitivity assumption | (out of scope) | Renderer transitively closes footpaths, so the harness can't observe non-closed inputs. |

## Reproducing a failure

Hegel persists failing seeds to `.hegel/` (gitignored). Re-running
`cargo nextest r -p raptor <test_name> --run-ignored all` deterministically
reproduces the most recent shrunk counterexample. To reproduce a specific
failure on a different machine, pass the seed explicitly:

```rust
#[hegel::test(seed = Some(0xdeadbeef))]
fn layer2_matches_reference(tc: hegel::TestCase) { /* ... */ }
```

## Internal simplification: trips on a route share leg/dwell durations

The generator constrains trips on the same route to share `leg_durations`
and `dwell_times`. This makes overtaking *structurally impossible* — every
spec the generator produces is a valid RAPTOR input.

This is stricter than the paper requires: the paper allows differing
durations as long as no trip overtakes another. We sample from a smaller
subset of the input space, not from outside the contract. Loosening this
(generating arbitrary per-trip durations and using rejection or
construction to avoid overtaking) is a reasonable future enhancement —
expect roughly 20–30 lines of changes in `route_spec` and an explicit
"no overtaking" check in `render`.

## Adding a new generator layer

To add a Layer 4 (e.g., for McRAPTOR in roadmap Phase 2):

1. Add a `LayerBounds` constant via `layerN_bounds()` in `spec.rs`.
2. Add a `#[hegel::test]` function in `mod.rs` that draws from
   `spec::network_spec(spec::layerN_bounds())` and calls `run_property`.
3. If the new layer requires multi-criterion comparison, the property
   helper (`run_property`) and `raptor_front` will need to be generalised.
   That's a Phase 2 concern.

## Wall-clock budget

Target: full property-test suite under 10 seconds on a developer laptop at
default case counts. If the budget blows out, the most likely culprit is
over-generation in the reference solver's node set; the adjacent-only
wait-edge model already caps this. Drop Layer 3's `test_cases` first.
```

- [ ] **Step 2: Commit**

```bash
jj fix
jj commit -m "[WIP: claude] Add proptest_support README"
```

---

## Task 12: Final verification

**Files:** none (verification-only).

- [ ] **Step 1: Confirm the full suite passes when ignored tests are skipped**

```bash
cargo nextest r -p raptor
```

Expected: all unit tests pass, all proptest_support tests pass except the
two ignored ones (layer 2 and layer 3) which show as ignored.

- [ ] **Step 2: Confirm Layer 2 and Layer 3 fail on v0.2.0 baseline when un-ignored**

```bash
cargo nextest r -p raptor proptest_support --run-ignored all
```

Expected: Layer 1 passes, Layer 2 fails with shrunk counterexample, Layer 3
fails with shrunk counterexample. Total wall-clock under 10 seconds.

- [ ] **Step 3: Read the Layer 2 counterexample and confirm it's small**

The shrunk spec should ideally be 2–3 stops, 1 route, 1 footpath — small
enough to read at a glance. Note the specific failure pattern in the commit
message of the next step. If the counterexample isn't shrinking well,
consider whether any generator is using rejection sampling that's tripping
the shrinker.

- [ ] **Step 4: Run clippy and fmt one last time across the workspace**

```bash
cargo fmt
cargo clippy --all-targets --all-features
```

Expected: no warnings.

- [ ] **Step 5: Final verification commit (empty if nothing changed)**

```bash
jj st
```

If clean, no further commit needed. Otherwise:

```bash
jj fix
jj commit -m "[WIP: claude] Format/clippy pass for proptest_support"
```

- [ ] **Step 6: Summarise the deliverable**

Print to the chat:

- The `cargo nextest r -p raptor` exit status (must be green).
- The `cargo nextest r -p raptor proptest_support --run-ignored all` exit
  status (must be red on layer 2 + 3).
- The shrunk Layer 2 counterexample.
- A one-line confirmation that the deliverable from
  `docs/proptest.md` is met: "A failing test on the unmodified v0.2.0
  baseline" — yes/no, with the specific failing test names.

This is the handoff to Phase 0. The next piece of work (separate plan) will
fix soundness issues A–D and remove the `#[ignore]` attributes.
