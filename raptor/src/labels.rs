//! Canned [`Label`](crate::Label) implementations.
//!
//! Single-criterion routing uses [`crate::ArrivalTime`] (re-exported at
//! the crate root). For multi-criterion routing, this module ships a
//! small set of vetted impls users can drive the algorithm with via
//! [`Timetable::raptor_with_label`](crate::Timetable::raptor_with_label).
//!
//! Custom impls live in user code — see the [`Label`](crate::Label)
//! trait docs for the requirements.

use crate::{Label, Tau};

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
/// use raptor::{Journey, StopIdx, Tau, Timetable};
/// use raptor::labels::ArrivalAndWalk;
///
/// # fn run<T: Timetable>(tt: &T, start: StopIdx, target: StopIdx) {
/// let journeys: Vec<Journey<ArrivalAndWalk>> =
///     tt.raptor_with_label::<ArrivalAndWalk>(10, 32400, &[(start, 0)], &[(target, 0)]);
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
    pub arrival: Tau,
    /// Total walking time accumulated along the path that produced
    /// this label, in seconds.
    pub walk_time: Tau,
}

impl Label for ArrivalAndWalk {
    const UNREACHED: Self = ArrivalAndWalk {
        arrival: Tau::MAX,
        walk_time: 0,
    };

    #[inline]
    fn from_departure(tau: Tau) -> Self {
        ArrivalAndWalk {
            arrival: tau,
            walk_time: 0,
        }
    }

    #[inline]
    fn extend_by_trip(self, arrival_tau: Tau) -> Self {
        ArrivalAndWalk {
            arrival: arrival_tau,
            walk_time: self.walk_time,
        }
    }

    #[inline]
    fn extend_by_footpath(self, walk: Tau) -> Self {
        ArrivalAndWalk {
            arrival: self.arrival.saturating_add(walk),
            walk_time: self.walk_time.saturating_add(walk),
        }
    }

    #[inline]
    fn dominates(&self, other: &Self) -> bool {
        self.arrival <= other.arrival && self.walk_time <= other.walk_time
    }

    #[inline]
    fn arrival(&self) -> Tau {
        self.arrival
    }
}
