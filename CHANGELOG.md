# Changelog

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
- Hegel-based property test harness in `raptor/src/proptest_support/`
  that checks the algorithm against a brute-force multi-criterion
  Dijkstra reference solver. Three generator layers (no-footpath,
  small-footpath, full-network) all green on this release.
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
