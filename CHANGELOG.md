# Changelog

## [0.4.0] — 2026-05-04

This release closes the data-representation half of Phase 1: the
algorithm's hot loop is now branch-friendly array indexing, the GTFS
adapter pre-computes per-route departure/arrival tables, and the
`Timetable` trait is non-generic.

### Breaking changes

- The `Timetable` trait no longer has associated `Stop`/`Route`/`Trip`
  types. All identifiers are now newtypes around `u32`: `StopIdx`,
  `RouteIdx`, `TripIdx`. Implementors must add `n_stops()` and
  `n_routes()` methods; slice-returning accessors return `&[T]` instead
  of `Cow<[T]>`.
- `Journey` is now non-generic:
  `Journey { plan: Vec<(RouteIdx, StopIdx)>, arrival: Tau }`.
- `RaptorCache` is non-generic and constructed via
  `RaptorCache::for_timetable` (or `RaptorCache::with_capacity` for the
  count-only path). Reusing a cache against a differently-sized timetable
  now panics on entry to `raptor_with_cache`.
- `GtfsTimetable`'s associated types are gone; look up string IDs via
  `GtfsTimetable::stop_id` / `GtfsTimetable::route_id` /
  `GtfsTimetable::trip_id` and inverse-lookup via
  `GtfsTimetable::stop_idx` / `GtfsTimetable::route_idx` /
  `GtfsTimetable::trip_idx`. The previously-public `gtfs: &'gtfs Gtfs`
  field has been removed (no accessor reads it after the rewrite); the
  `'gtfs` lifetime remains in use for interned `&str`s and the
  `GtfsTimetable::new(gtfs: &'gtfs Gtfs)` signature is unchanged.
- `gtfs::RouteId` is removed; the synthetic-route concept now lives on
  `RouteIdx` with the same semantics.
- `SimpleTimetable<S, R, T>` has new generic bounds (`Hash + Eq + Clone`)
  and now interns its keys to dense `u32` indices. Construction API is
  unchanged; tests asserting on plans use the new
  `SimpleTimetable::stop_idx_of` / `SimpleTimetable::route_idx_of`
  helpers (and the in-test `plan!` macro). The internal `trips` field
  is now `Vec<Option<...>>` rather than a placeholder-padded `Vec<...>`,
  so trip slots that have not yet been populated are explicit.

### Performance

- Round labels are now `Vec<Vec<Tau>>` indexed by `(round, stop_idx)`
  rather than `Vec<BTreeMap<Stop, Tau>>` — all label reads/writes in the
  hot loop are array indexing.
- Marked stops are a `fixedbitset::FixedBitSet` sized to `n_stops`;
  insertion is a single bit write, iteration walks set bits.
- The per-round route queue is a sparse-set pair (`Vec<Option<StopIdx>>`
  plus a dense `Vec<RouteIdx>`) instead of `BTreeMap<RouteIdx, StopIdx>`,
  giving `O(distinct routes touched)` reset cost per round.
- `GtfsTimetable` now holds `arrival_times[route][stop_pos][trip_pos]`
  and `departure_times[…]` arrays computed once at construction. The
  table read itself is `O(1)`; a `route_for_trip[trip]` reverse map
  resolves the route in `O(1)`. The `stop_pos` lookup remains an
  `O(stops_in_route)` linear scan, so `get_arrival_time` /
  `get_departure_time` are now `O(stops_in_route)` once per call rather
  than `O(stops_in_route)` per stop scanned inside a trip lookup. A
  `(route, stop) → stop_pos` reverse map would close this gap; deferred
  to a follow-up.

### Added

- New dependency: `fixedbitset = "0.5"` (used for the marked-stops
  bitset).
- `GtfsTimetable::routes_for_gtfs_id` to enumerate every synthetic
  `RouteIdx` derived from a given GTFS `route_id`.
- `SimpleTimetable::register_stop` to intern a stop into the timetable
  builder without otherwise modifying it (useful when you need a stop
  reachable only via footpaths, with no boarding events of its own).

## [0.3.0] — 2026-05-03

This release closes Phase 0 of the production roadmap: the algorithm now
produces correct results, and the GTFS adapter no longer silently
returns wrong answers on real-world feeds.

### Breaking changes

- `gtfs::GtfsTimetable`'s associated `Route` type is now
  `gtfs::RouteId` (a `u32` newtype) rather than `&str`. A single GTFS
  `route_id` is split at construction into one or more synthetic
  `RouteId`s — one per equivalence class of trips with identical,
  non-overtaking stop sequences. Recover the original `route_id` for
  display via `GtfsTimetable::route_name(RouteId)`.

  Migration: where you previously matched on a `&str` route in a
  `Journey::plan` entry, call `timetable.route_name(*route_id)` first
  to get the GTFS `route_id`, then look up further metadata as before.
  See the updated `examples/gtfs-timetable.rs`.

### Algorithm correctness fixes

The full Phase 0 list. Each was a soundness gap on `v0.2.0`; details
are in `soundness.md` (issues A–I, all moved to Resolved Issues).

- **Carry-forward round labels** (issue A): labels are now stored as
  `Vec<BTreeMap<Stop, Tau>>` indexed by round, with `labels[k] =
  labels[k-1].clone()` at the top of each round. Stops reached in
  earlier rounds remain usable as boarding points and footpath
  origins.
- **Footpath relaxation from the source in round 0** (issue B): the
  footpath stage now runs once at init so journeys that begin with a
  walk are discoverable in round 1.
- **τ\* updated in the footpath stage** (issue C): walk-derived label
  improvements are mirrored into `best_arrival` so local and target
  pruning see them.
- **Target pruning in the footpath stage** (issue D): walk-reached
  stops are only marked when their arrival improves on
  `best_arrival[pt]`.
- **GTFS route-pattern splitting** (issue E): synthetic `RouteId`s as
  described above; `get_earliest_trip` is now sound on real GTFS
  feeds.
- **Pareto-filtered output** (issue F): `Timetable::raptor` sorts the
  result by trip count ascending and drops journeys whose arrival is
  not strictly better than the best seen so far. Output is
  deterministic.
- **Walk-leg journey reconstruction** (issue I): the boarding tree
  records both `Boarded` and `Walked` steps; reconstruction chains
  through walks within a round so walk-then-board, board-walk-board,
  and board-then-walk-to-pt journeys all reach the user.
- **Saturating arithmetic** (issue G): `relax_footpaths_round` uses
  `saturating_add`; the rest of the algorithm performs no `Tau`
  arithmetic.
- **Documented invariants** (issue H): the `Timetable` trait spells
  out the footpath-transitivity and no-overtaking contracts.

### Added

- `RaptorCache<Route, Stop>` and `Timetable::raptor_with_cache` for
  reusing scratch buffers across queries — recommended for server use
  cases running many queries against the same timetable.
- Hegel-based property test harness in the new `raptor-proptest`
  workspace crate that checks the algorithm against a brute-force
  multi-criterion Dijkstra reference solver. Three generator layers
  (no-footpath, small-footpath, full-network) all green on this
  release.
- `gtfs::GtfsError::MissingDepartureTime` variant: `GtfsTimetable::new`
  now refuses to construct from a feed whose `stop_times.txt` is
  missing departure times the algorithm relies on for ordering.

### Documentation

- Top-level `Timetable` trait now documents the footpath-transitivity
  and no-overtaking contracts that implementors must uphold.
- `Timetable::raptor`'s doc spells out the Pareto-optimality contract
  (sorted by trip count ascending; arrival strictly decreasing).
- README has a new "Implementing `Timetable`" section, an updated
  GTFS example reflecting the `RouteId` change, and a "Performance:
  reusing a `RaptorCache`" section.

## [0.2.0] — 2026-03-01

### Added
- Benchmarks with different networks and a `dotgraph` visualization tool (#17)
- Test infrastructure with `TestTimetable` and property-based test cases (#9)
- CI/CD workflow for PR checks (#10)

### Changed
- Switched to `Cow`-based API for zero-copy support (#13)
- Improved GTFS implementation: named constants, upfront cache validation, default transfer time (#11)
- Removed code duplication in the routing algorithm (#12)
- Rewrote documentation and README (#14)
- Converted to workspace layout; made `TestTimetable` public (#17)

## [0.1.1]

Initial release.
