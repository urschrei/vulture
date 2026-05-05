//! Canned [`Label`] implementations.
//!
//! Single-criterion routing uses [`crate::ArrivalTime`] (re-exported at
//! the crate root). For multi-criterion routing, this module ships a
//! small set of vetted impls users can drive the algorithm with via
//! [`Timetable::query_with_label`](crate::Timetable::query_with_label).
//!
//! Custom impls live in user code – see the [`Label`]
//! trait docs for the requirements.

use crate::{Duration, Label, SecondOfDay};

/// Two-criterion label tracking arrival time *and* accumulated walking
/// time. Trip rides preserve the boarding label's walking time;
/// footpath relaxations add the walk's duration to both arrival and
/// the running walking-time component.
///
/// Pareto dominance is the obvious component-wise relation: `self`
/// dominates `other` iff its arrival is `≤` and its walking time is
/// `≤`. The algorithm maintains a Pareto front per stop, so a query
/// with this label can return multiple journeys at the same target —
/// for example, a faster one with more walking and a slower one with
/// less.
///
/// ```no_run
/// use vulture::{Journey, StopIdx, SecondOfDay, Timetable};
/// use vulture::labels::ArrivalAndWalk;
///
/// # fn run<T: Timetable>(tt: &T, start: StopIdx, target: StopIdx) {
/// let journeys: Vec<Journey<ArrivalAndWalk>> = tt
///     .query_with_label::<ArrivalAndWalk>()
///     .from(start)
///     .to(target)
///     .max_transfers(10)
///     .depart_at(SecondOfDay::hms(9, 0, 0))
///     .run();
/// for j in &journeys {
///     println!(
///         "arrives {}s, walked {}s",
///         j.label.arrival, j.label.walk_time,
///     );
/// }
/// # }
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ArrivalAndWalk {
    /// Effective arrival time at the labelled stop, in seconds since
    /// midnight.
    pub arrival: SecondOfDay,
    /// Total walking time accumulated along the path that produced
    /// this label, in seconds.
    pub walk_time: Duration,
}

impl Label for ArrivalAndWalk {
    const UNREACHED: Self = ArrivalAndWalk {
        arrival: SecondOfDay::MAX,
        walk_time: Duration::ZERO,
    };

    #[inline]
    fn from_departure(at: SecondOfDay) -> Self {
        ArrivalAndWalk {
            arrival: at,
            walk_time: Duration::ZERO,
        }
    }

    #[inline]
    fn extend_by_trip(self, arrival: SecondOfDay) -> Self {
        ArrivalAndWalk {
            arrival,
            walk_time: self.walk_time,
        }
    }

    #[inline]
    fn extend_by_footpath(self, walk: Duration) -> Self {
        ArrivalAndWalk {
            arrival: self.arrival + walk,
            walk_time: self.walk_time + walk,
        }
    }

    #[inline]
    fn dominates(&self, other: &Self) -> bool {
        self.arrival <= other.arrival && self.walk_time <= other.walk_time
    }

    #[inline]
    fn arrival(&self) -> SecondOfDay {
        self.arrival
    }
}
