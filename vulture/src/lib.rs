#![deny(missing_docs)]

//! Rust implementation of [RAPTOR][paper] (Delling, Pajor, Werneck): given a
//! transit network, find all Pareto-optimal journeys between two stops,
//! trading fewer transfers against earlier arrival.
//!
//! [paper]: https://www.microsoft.com/en-us/research/publication/round-based-public-transit-routing/
//!
//! # Quick start
//!
//! Most users start with the bundled [`gtfs`] adapter, which wraps a parsed
//! GTFS feed and implements [`Timetable`] for it.
//!
//! ```no_run
//! use gtfs_structures::Gtfs;
//! use jiff::civil::date;
//! use vulture::{SecondOfDay, Timetable};
//! use vulture::gtfs::GtfsTimetable;
//!
//! # fn main() -> anyhow::Result<()> {
//! let gtfs = Gtfs::new("path/to/gtfs.zip")?;
//! // Pin the timetable to one service date; inactive trips are filtered out.
//! let tt = GtfsTimetable::new(&gtfs, date(2026, 5, 4))?;
//!
//! // The algorithm takes dense u32 indices, not GTFS string IDs – resolve first.
//! let start = tt.stop_idx("dilshad_garden").expect("unknown stop");
//! let target = tt.stop_idx("vishwavidyalaya").expect("unknown stop");
//!
//! let journeys = tt
//!     .query()
//!     .from(start)
//!     .to(target)
//!     .max_transfers(10)
//!     .depart_at(SecondOfDay::hms(9, 0, 0))
//!     .run();
//!
//! for j in &journeys {
//!     println!("{} trip(s), arrives {}", j.plan.len(), j.arrival());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Concepts
//!
//! RAPTOR proceeds in *rounds*. Round k holds the earliest known arrival time
//! at every stop reachable using at most k trips. Each round scans the routes
//! through stops improved in the previous round and then relaxes walking
//! footpaths to a fixed point. The output is a Pareto front: one [`Journey`]
//! per trip count between zero and `max_transfers`, with strictly earlier
//! arrivals as you allow more trips. There is no Dijkstra-style priority queue
//! and no shortest-path tree – just successive rounds of array updates.
//!
//! The default routing is *single-criterion*: minimise arrival time, then
//! report one journey per trip count. The criterion under optimisation is
//! abstracted by the [`Label`] trait; the default [`ArrivalTime`] just carries
//! a [`SecondOfDay`]. Multi-criterion routing (e.g. trade-offs between arrival
//! time and accumulated walking time) plugs in via [`Timetable::query_with_label`]
//! with one of the [`labels`] impls or your own.
//!
//! # Common patterns
//!
//! - **Single query.** [`Timetable::query`] returns a typestate builder.
//!   Chain `.from(...).to(...).max_transfers(...).depart_at(...).run()`.
//! - **Multi-platform stations.** [`gtfs::GtfsTimetable::station_stops`]
//!   expands a parent station to its child platforms; pass it to `.from(...)` /
//!   `.to(...)`.
//! - **Range queries.** [`Query::depart_in_window`] returns a
//!   [`Vec<RangeJourney>`](RangeJourney) Pareto profile across departure times.
//!   Serial `.run()` is rRAPTOR; `.run_par()` / `.run_with_pool(&pool)` fan
//!   per-departure work across Rayon (default-on `parallel` feature).
//! - **Server reuse.** Allocate a [`RaptorCache`] once via
//!   [`RaptorCache::for_timetable`] and finish each chain with
//!   `.run_with_cache(&mut cache)` – amortises scratch-buffer allocation
//!   across queries.
//! - **Multi-threaded reuse.** [`RaptorCachePool`] is the `Sync` variant.
//!   Each worker calls [`RaptorCachePool::checkout`] to get a cache for one
//!   query; the cache returns to the pool on drop.
//! - **Multi-criterion routing.** [`Timetable::query_with_label`] takes a
//!   custom [`Label`]. The bundled [`labels::ArrivalAndWalk`] trades arrival
//!   time against accumulated walking time.
//! - **Per-leg trip IDs and timing.** [`Journey::with_timing`] reconstructs
//!   per-leg `(boarding stop, trip, depart, alight stop, arrive)` tuples.
//!   `Journey.plan` on its own is just topology.
//! - **Sparse `transfers.txt`.** [`gtfs::GtfsTimetable::with_walking_footpaths`]
//!   builds bidirectional walking edges from stop coordinates using an R-tree.
//!
//! # Custom backends
//!
//! Implement the [`Timetable`] trait directly if your data is not in GTFS
//! form. Identifiers are dense `u32` newtypes ([`StopIdx`], [`RouteIdx`],
//! [`TripIdx`]); your adapter interns external IDs to dense indices at
//! construction. The trait carries one mandatory soundness contract —
//! no-overtaking within a route – documented on [`Timetable`]. Footpaths
//! returned by [`Timetable::get_footpaths_from`] describe direct walks only;
//! the algorithm chains them within a round, so transitive closure of the
//! footpath relation is not required (see the [`Timetable`] trait's
//! Footpaths section for what closure means and when it matters).
//! Adapters whose relation *is* closed can opt into a faster single-pass
//! relaxation via [`Timetable::footpaths_are_transitively_closed`].
//!
//! [`manual::SimpleTimetable`] is a hand-rolled in-memory adapter useful for
//! tests and small fixtures.
//!
//! For an issue-by-issue walk through the algorithmic correctness of the
//! implementation against the paper, see
//! [`docs/soundness.md`](https://github.com/urschrei/vulture/blob/main/docs/soundness.md)
//! in the repository.
//!
//! # Errors
//!
//! The library uses three patterns, picked per-call to match what the
//! caller can usefully do:
//!
//! - **`Result` for construction.** Building a [`gtfs::GtfsTimetable`]
//!   from a parsed feed validates many invariants and returns
//!   [`Result<_, gtfs::GtfsError>`](gtfs::GtfsError). Custom adapters
//!   should follow the same pattern with their own typed error enum.
//! - **`Option` for lookups.** Resolving an external ID
//!   ([`gtfs::GtfsTimetable::stop_idx`], [`gtfs::GtfsTimetable::route_idx`])
//!   returns `Option` – `None` simply means "not in this timetable".
//!   Same for any "find me an `X` matching `Y`" accessor; there's no
//!   useful structured error.
//! - **`Result` for plan reconstruction.** [`Journey::with_timing`]
//!   returns [`Result<_, TimingError>`] – failure modes (no boarding
//!   stop reachable, no catchable trip, alighting stop not on route)
//!   are surfaced as variants because they carry useful debug context.
//! - **Panics for programmer-violated invariants.** Passing a
//!   [`RaptorCache`] sized for one timetable to a query against a
//!   differently-sized timetable panics – the mistake is unrecoverable
//!   and the assertion message is the diagnostic. Custom [`Timetable`]
//!   adapters that violate the no-overtaking contract documented on
//!   the trait will likely produce wrong answers rather than panic.
//!
//! There is no all-encompassing `vulture::Error` enum – each failure
//! lives at a clear boundary, so unifying them would lose information
//! rather than add it.
//!
//! # Cargo features
//!
//! - `parallel` (default-on) – pulls in `rayon`, enables [`Query::run_par`] /
//!   [`Query::run_with_pool`]. Opt out with `default-features = false` for
//!   wasm or minimal builds; [`RaptorCachePool`] itself stays available.
//! - `gtfs-bench` – enables the `gtfs` criterion benchmark.
//! - `internal` – enables the `raptor` criterion benchmark over [`manual`].

mod algorithm;
mod cache;
mod endpoints;
mod ids;
mod journey;
mod label;
mod query;
mod time;
mod timetable;

pub mod gtfs;
pub mod labels;
/// In-memory `Timetable` adapter you build by hand with `.route(...)` /
/// `.footpath(...)` calls. Useful when your data isn't from a parsed
/// GTFS feed – for tests, custom data sources, and toy examples.
pub mod manual;

// Central items: the trait, the builder, the result.
pub use timetable::Timetable;

pub use query::Query;
pub use query::RangeJourney;

pub use journey::Journey;
pub use journey::TimedLeg;
pub use journey::TimingError;

// Cache reuse: scratch buffers for repeated queries.
pub use cache::PooledCache;
pub use cache::RaptorCache;
pub use cache::RaptorCachePool;

// Label trait + the default single-criterion impl.
pub use label::ArrivalTime;
pub use label::Label;

// Query builder typestate markers.
pub use query::NeedsDeparture;
pub use query::RangeDeparture;
pub use query::SingleDeparture;

// Endpoints – query input.
pub use endpoints::Endpoints;
pub use endpoints::IntoEndpoints;

// Newtypes for indices and time quantities.
pub use ids::RouteIdx;
pub use ids::StopIdx;
pub use ids::TripIdx;
pub use time::Duration;
pub use time::SecondOfDay;
pub use time::Transfers;

#[cfg(test)]
mod test;

/// Internal round-counter type, used only for indexing label arrays.
/// User-facing transfer caps are passed as [`Transfers`].
pub(crate) type K = usize;
