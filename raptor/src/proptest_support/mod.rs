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
