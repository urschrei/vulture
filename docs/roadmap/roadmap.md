# vulture roadmap

What's planned next, ordered by clarity-of-scope rather than priority.

For what's already landed, see [`CHANGELOG.md`](../../CHANGELOG.md).

## Near-term

### GTFS-RT (real-time updates)

Real-time delays, cancellations, and added trips. Standard pattern: keep the static [`GtfsTimetable`] immutable and overlay a delta structure that adjusts arrival/departure lookups per query. The Connection Scan Algorithm paper (Strasser & Wagner, 2014) sketches the delta-overlay shape; for RAPTOR the same idea works.

Open questions: how to expose the overlay in the public API (a separate `Timetable` impl that wraps a `GtfsTimetable` + a delta? a builder method?), whether the overlay needs its own `Cargo` feature, and how to source updates (the `gtfs-rt` crate can decode protobuf feeds – wiring is straightforward, the design is the work).

### Pickup / drop-off type flags

GTFS `stop_times.pickup_type` and `drop_off_type` allow a feed to mark a scheduled stop as not-boardable (`pickup_type = 1`) or not-alightable (`drop_off_type = 1`) – common on long-distance rail with explicit "set down only" stops, or on airport shuttles that pick up at one terminal but don't drop there on the return. Vulture currently ignores both flags and treats every scheduled stop as boardable and alightable; on metros this is invisible (all flags are 0/0), but a long-distance feed could see vulture produce journeys the operator does not actually allow.

Fix: gate the boarding scan in `Timetable::get_earliest_trip` on `pickup_type ≠ 1` and the alighting candidacy check on `drop_off_type ≠ 1`. The flags are per `(trip, stop_sequence)` so they fit naturally into the existing per-position trait accessors. Surfaced by [`docs/cross-impl-comparison.md`](../cross-impl-comparison.md) – `raptor-journey-planner` honours these.

### Multi-day journey search

A 23:00 query that needs an after-midnight or next-morning trip currently returns no journey because the `GtfsTimetable` is calendar-filtered to a single service date at construction. RAPTOR itself extends naturally across days; the `Timetable` just needs to know about tomorrow's services. Two shapes worth considering:

- **Eager:** load N consecutive service days at construction. Simpler; uses more memory; arithmetic over `SecondOfDay` already permits values past 86400.
- **Lazy:** when the algorithm exhausts the current day's service without finding a journey, roll forward into the next day's calendar and retry. Closer to what `raptor-journey-planner`'s `maxSearchDays = 3` does. More complex but pays only on overnight queries.

Either way it's a `GtfsTimetable` builder option (`.with_overnight_days(usize)` or similar) plus algorithm-side handling of cross-midnight times. Real value for "last train home" and red-eye flight queries; nuisance otherwise.

### Accessibility flags

GTFS exposes `wheelchair_boarding` on stops and `wheelchair_accessible` on trips. Add a query-level filter that drops trips/stops failing the requirement. Likely shape: a builder method `.require_wheelchair_accessible()` on the `Query` typestate that adds a per-trip predicate to `get_earliest_trip`. Should compose cleanly with the existing `Label` machinery – a wheelchair-aware journey is still arrival-time-optimal within the filtered subnetwork.

### Better error context for GTFS construction

`GtfsError` covers the failure modes but each variant currently carries only the offending ID. For real feeds the useful question is "where in the feed did this trip come from" – adding the `agency_id` / `route_id` chain in error messages would make diagnosing bad feeds substantially less painful. Pure ergonomic improvement, no algorithm changes.

### CI on real feeds

Current CI presumably runs unit + proptests. Add:
- A nightly job that loads the bundled Delhi feed and runs the `gtfs_query` / `gtfs_range` benches; fails on regressions beyond a threshold.
- A weekly job that pulls Helsinki HSL / Berlin VBB / Paris IDFM (the three large feeds the cross-city benchmarks use), constructs the timetable, runs a small fixed query set, and asserts no panics. Catches breakage from upstream feed format changes.

### Fuzzing target

`cargo-fuzz` target that constructs random `Timetable` impls (reusing the proptest spec generator) and runs queries, asserting no panics, no infinite loops, and well-formed output. The proptest harness already covers correctness against a brute-force reference; fuzz adds robustness coverage for the panic surface in algorithm hot loops and the GTFS adapter.

### MSRV policy

Pin a minimum supported Rust version in `Cargo.toml` (`rust-version = "1.85"` would be a reasonable lower bound – Rust 2024 edition stable). Add a CI matrix entry that builds against MSRV alongside latest stable.

## Longer-term

### Workspace split

Currently one crate (`vulture`) with feature flags gating GTFS, parallel, and dotgraph. A future split:

- `vulture-core` – algorithm, `Timetable` trait, `Label` trait, no I/O dependencies.
- `vulture-gtfs` – the GTFS adapter. Pulls `gtfs-structures`, `chrono`, `jiff`, `rstar`.
- `vulture-gtfsrt` – real-time overlay (when it lands).
- `vulture-cli` – a CLI binary for ad-hoc queries against a GTFS feed.
- `vulture` – facade crate that re-exports the core + adapters via feature flags. Keeps `cargo add vulture` working.

Worth doing if the dependency footprint of the GTFS adapter becomes a problem (e.g., for embedded or wasm users who only want the algorithm). Premature otherwise – the feature flags currently do the same job at zero cost.

### Transfer Patterns

Bast et al. (2010): for a given network and a representative query distribution, precompute the small set of stop-sequence patterns capable of producing optimal journeys between each origin–destination pair (or proxy thereof). Queries become bag-of-pattern lookups plus a few small RAPTOR scans, cutting query latency by an order of magnitude or more – at the cost of a pattern-database build that takes hours to days for a real network.

`raptor-journey-planner` ships a `TransferPatternQuery` and a `TransferPatternRepository` backed by MySQL. For vulture this would be a separate algorithm path rather than a refinement of the RAPTOR core: at minimum a precomputation tool, a storage format, and a runtime query that reuses RAPTOR for the per-query local scans. Substantial – comparable in scope to the GTFS adapter or the proptest harness, not a mod of the existing code.

Worth doing only when the network is stable and the query distribution is biased (a public route planner for the same city, run for years – yes; a one-shot analysis against an unfamiliar feed – no). Defer until there is a concrete user.

### McProfileRAPTOR for range queries

The serial range path is rRAPTOR specialised for `L = ArrivalTime`. Multi-criterion `Label`s (e.g., `ArrivalAndWalk`) currently fall back to the parallel naïve batch even on the serial path. McProfileRAPTOR – the multi-criterion + range generalisation from §5 of the paper – would close that gap, with a Pareto profile of (depart, label) per cell. Substantial new algorithm; rough scope is similar to the rRAPTOR rewrite that just landed.

### Multi-hop walk reconstruction in `with_timing`

`Journey::with_timing` reconstructs single-hop walking transfers between transit legs but returns `TimingError::NoBoardingStop` for multi-hop walks. Fixing it requires either (a) storing the boarding stop per leg in the `Journey` plan or (b) running a per-call walk-graph relaxation during reconstruction. Option (a) bloats the plan; option (b) is a non-trivial extra step at output time. Worth doing only if a real user trips over it.

### CLI binary

A `vulture` (or `vulture-cli`) binary for ad-hoc queries: `vulture query --feed berlin.zip --from de:11000:900003201 --to de:11000:900100003 --depart 09:00`. Useful for exploration without writing Rust; mostly a packaging/UX exercise on top of the existing API. Part of the workspace split if that lands.

### Bindings

Python or wasm bindings via `pyo3` / `wasm-bindgen`. No specific demand yet; would be driven by user request.

## Out of scope

- **`no_std`.** RAPTOR uses heap allocation throughout (the cache, the bag-of-labels, the Pareto fronts) and the target audience (transit routing servers) all have `std`. Skip unless a concrete embedded use case appears.
- **Path expansion ALT / contraction hierarchies.** RAPTOR's whole appeal is no preprocessing; once you start preprocessing you might as well use a different algorithm family.
- **Driving / cycling routing.** Different problem family – use [`pathfinding`](https://crates.io/crates/pathfinding) or a road-router crate.
