//! Concrete-typed entry points for FFI bindings (PyO3, wasm-bindgen,
//! and similar) that can't easily monomorphise the generic
//! [`Query<'tt, T, L, M>`](crate::Query) builder across the language
//! boundary.
//!
//! Each function:
//!
//! - Takes `tt: &T` with `T: Timetable + ?Sized`, so it accepts both
//!   concrete adapters (`&GtfsTimetable`, `&SimpleTimetable`) and
//!   trait objects (`&dyn Timetable`). FFI bindings typically wrap
//!   `Box<dyn Timetable>` and pass `&*tt`.
//! - Hardcodes a single bundled [`Label`](crate::Label) impl per
//!   function name (`run_arrival`, `run_walk`, `run_fare`) so the
//!   return type is concrete and the binding doesn't need to plumb a
//!   generic `L` through.
//! - Allocates a fresh [`RaptorCache`] per call.
//!   FFI workloads that hit the same timetable repeatedly should
//!   wrap a `RaptorCache<L>` on the binding side and call the lower-
//!   level builder API directly; this module's role is "one-shot,
//!   no fuss".
//!
//! Direct Rust use should prefer [`Timetable::query`]
//! / [`Timetable::query_with_label`] —
//! these wrappers exist for FFI consumers, not as the primary
//! single-criterion API.

use crate::algorithm::per_call::run_per_call_query;
use crate::cache::RaptorCache;
use crate::ids::StopIdx;
use crate::journey::Journey;
use crate::label::ArrivalTime;
use crate::labels::{ArrivalAndFare, ArrivalAndWalk, FareTable};
use crate::time::{Duration, SecondOfDay};
use crate::timetable::Timetable;

/// Single-criterion arrival-time query — the default RAPTOR shape.
/// Returns one [`Journey`] per `(arrival, transfer count)` Pareto
/// trade-off, sorted by transfer count ascending.
pub fn run_arrival<T: Timetable + ?Sized>(
    tt: &T,
    origins: &[(StopIdx, Duration)],
    targets: &[(StopIdx, Duration)],
    max_transfers: u8,
    depart: SecondOfDay,
    require_wheelchair_accessible: bool,
) -> Vec<Journey<ArrivalTime>> {
    let mut cache = RaptorCache::<ArrivalTime>::for_timetable(tt);
    run_per_call_query(
        tt,
        &(),
        &mut cache,
        usize::from(max_transfers),
        depart,
        origins,
        targets,
        require_wheelchair_accessible,
    )
}

/// Two-criterion `(arrival, walking time)` query. Returns the Pareto
/// front: a fastest-but-walkier journey alongside any slower-but-
/// less-walking alternatives.
pub fn run_walk<T: Timetable + ?Sized>(
    tt: &T,
    origins: &[(StopIdx, Duration)],
    targets: &[(StopIdx, Duration)],
    max_transfers: u8,
    depart: SecondOfDay,
    require_wheelchair_accessible: bool,
) -> Vec<Journey<ArrivalAndWalk>> {
    let mut cache = RaptorCache::<ArrivalAndWalk>::for_timetable(tt);
    run_per_call_query(
        tt,
        &(),
        &mut cache,
        usize::from(max_transfers),
        depart,
        origins,
        targets,
        require_wheelchair_accessible,
    )
}

/// Two-criterion `(arrival, fare)` query. The supplied [`FareTable`]
/// maps each [`RouteIdx`](crate::RouteIdx) to its fare in user-defined
/// units (typically cents). Returns the Pareto front of arrival-vs-
/// fare trade-offs.
pub fn run_fare<T: Timetable + ?Sized>(
    tt: &T,
    fares: &FareTable,
    origins: &[(StopIdx, Duration)],
    targets: &[(StopIdx, Duration)],
    max_transfers: u8,
    depart: SecondOfDay,
    require_wheelchair_accessible: bool,
) -> Vec<Journey<ArrivalAndFare>> {
    let mut cache = RaptorCache::<ArrivalAndFare>::for_timetable(tt);
    run_per_call_query(
        tt,
        fares,
        &mut cache,
        usize::from(max_transfers),
        depart,
        origins,
        targets,
        require_wheelchair_accessible,
    )
}
