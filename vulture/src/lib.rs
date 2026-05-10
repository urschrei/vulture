#![deny(missing_docs)]

//! An implementation of [RAPTOR][paper] (Delling, Pajor, Werneck): given a
//! public transit network, find all Pareto-optimal journeys between two stops,
//! trading fewer transfers against earlier arrival.
//!
//! [paper]: https://www.microsoft.com/en-us/research/publication/round-based-public-transit-routing/
//!
//! # Quick start
//!
//! The [`gtfs`] adapter implements [`Timetable`] over a parsed GTFS feed.
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
//! arrivals as trip count rises.
//!
//! The default routing is *single-criterion*: minimise arrival time, one
//! journey per trip count. The criterion is abstracted by the [`Label`] trait;
//! the default [`ArrivalTime`] carries a [`SecondOfDay`]. Multi-criterion
//! routing plugs in via [`Timetable::query_with_label`] with one of the
//! [`labels`] impls or a custom [`Label`].
//!
//! # Common patterns
//!
//! - **Single query.** [`Timetable::query`] returns a typestate builder.
//!   Chain `.from(...).to(...).max_transfers(...).depart_at(...).run()`.
//! - **Multi-platform stations.** [`gtfs::GtfsTimetable::station_stops`]
//!   expands a parent station to its child platforms for `.from(...)` /
//!   `.to(...)`.
//! - **Range queries.** [`Query::depart_in_window`] returns a
//!   [`Vec<RangeJourney>`](RangeJourney) Pareto profile across departure times.
//!   Serial `.run()` is rRAPTOR; `.run_par()` / `.run_with_pool(&pool)` fan
//!   per-departure work across Rayon (default-on `parallel` feature).
//! - **Server reuse.** Allocate a [`RaptorCache`] once via
//!   [`RaptorCache::for_timetable`]; finish each chain with
//!   `.run_with_cache(&mut cache)`.
//! - **Multi-threaded reuse.** [`RaptorCachePool`] is the `Sync` variant.
//!   Each worker calls [`RaptorCachePool::checkout`] for one query; the cache
//!   returns to the pool on drop.
//! - **Multi-criterion routing.** [`Timetable::query_with_label`] takes a
//!   custom [`Label`]. [`labels::ArrivalAndWalk`] trades arrival against
//!   accumulated walking time; [`labels::ArrivalAndFare`] trades arrival
//!   against accumulated fare via a context supplied through
//!   [`Query::with_context`].
//! - **Per-leg trip IDs and timing.** [`Journey::with_timing`] reconstructs
//!   per-leg `(boarding stop, trip, depart, alight stop, arrive)` tuples.
//!   `Journey.plan` carries the topology only.
//! - **Wheelchair-accessibility filter.**
//!   [`Query::require_wheelchair_accessible`] skips trips and stops flagged
//!   `NotAvailable` in GTFS `wheelchair_accessible` /
//!   `wheelchair_boarding`.
//! - **Multi-day search.**
//!   [`gtfs::GtfsTimetable::new_with_overnight_days`] loads
//!   [`gtfs::OvernightDays`] additional service days after the base date,
//!   shifting day-`d` trips by `d × 86 400` seconds onto one time axis.
//! - **Sparse `transfers.txt`.**
//!   [`gtfs::GtfsTimetable::with_walking_footpaths`] builds bidirectional
//!   R-tree walking edges from stop coordinates.
//! - **Feed introspection.** [`gtfs::GtfsTimetable::features`] returns a
//!   [`gtfs::FeedFeatures`] snapshot (counts, transfer-type breakdown,
//!   parent stations, shaped trips, wheelchair flags, footpath state).
//!   [`gtfs::FeedFeatures::suggestions`] maps it to advisory strings.
//!
//! # Custom backends
//!
//! Implement the [`Timetable`] trait directly for non-GTFS data. Identifiers
//! are dense `u32` newtypes ([`StopIdx`], [`RouteIdx`], [`TripIdx`]); the
//! adapter interns external IDs at construction. The trait carries one
//! mandatory soundness contract, no-overtaking within a route, documented
//! on [`Timetable`]. Footpaths returned by [`Timetable::get_footpaths_from`]
//! are direct walks only; the algorithm chains them within a round, so
//! transitive closure is not required. Adapters whose relation *is* closed
//! can opt into single-pass relaxation via
//! [`Timetable::footpaths_are_transitively_closed`].
//!
//! [`manual::SimpleTimetable`] is a hand-rolled in-memory adapter for tests
//! and small fixtures.
//!
//! Issue-by-issue audit against the paper:
//! [`docs/soundness.md`](https://github.com/urschrei/vulture/blob/main/docs/soundness.md).
//!
//! # Errors
//!
//! Four patterns, picked per call:
//!
//! - **`Result` for construction.** [`gtfs::GtfsTimetable`] returns
//!   [`Result<_, gtfs::GtfsError>`](gtfs::GtfsError). Custom adapters
//!   should follow the same pattern with their own error enum.
//! - **`Option` for lookups.** [`gtfs::GtfsTimetable::stop_idx`],
//!   [`gtfs::GtfsTimetable::route_idx`], and similar "find an `X` matching
//!   `Y`" accessors return `Option`; `None` means "not in this timetable".
//! - **`Result` for plan reconstruction.** [`Journey::with_timing`]
//!   returns [`Result<_, TimingError>`]; failure modes (no boarding
//!   stop reachable, no catchable trip, alighting stop not on route)
//!   are typed variants for debug context.
//! - **Panics for programmer-violated invariants.** A [`RaptorCache`]
//!   sized for one timetable, used against a differently-sized one,
//!   panics with an assertion message. Custom [`Timetable`] adapters
//!   that violate the no-overtaking contract typically produce wrong
//!   answers rather than panic.
//!
//! # Cargo features
//!
//! - `parallel` (default-on) – enables [`Query::run_par`] /
//!   [`Query::run_with_pool`] via `rayon`. `default-features = false` drops
//!   the dependency; [`RaptorCachePool`] stays available.
//! - `gtfs-bench` – the `gtfs` criterion benchmark.
//! - `internal` – the `raptor` criterion benchmark over [`manual`].

mod algorithm;
mod cache;
mod endpoints;
mod ids;
mod journey;
mod label;
mod query;
mod time;
mod timetable;

pub mod ffi;
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
pub use label::TripContext;

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
pub use time::ParseSecondOfDayError;
pub use time::SecondOfDay;
pub use time::Transfers;
pub use time::TryFromSignedDurationError;

#[cfg(test)]
mod test;

/// Internal round-counter type, used only for indexing label arrays.
/// User-facing transfer caps are passed as [`Transfers`].
pub(crate) type K = usize;
