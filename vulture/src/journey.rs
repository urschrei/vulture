//! [`Journey`] – what the algorithm returns: a sequence of `(route, alight stop)`
//! steps plus a label at the target. [`Journey::with_timing`] reconstructs
//! per-leg [`TimedLeg`]s (trip IDs and depart/arrive times) given the
//! timetable; failures surface as [`TimingError`].

use crate::Timetable;
use crate::ids::RouteIdx;
use crate::ids::StopIdx;
use crate::ids::TripIdx;
use crate::label::ArrivalTime;
use crate::label::Label;
use crate::time::Duration;
use crate::time::SecondOfDay;

/// A journey found by the RAPTOR algorithm.
///
/// Each journey consists of a sequence of (route, alight stop) steps and a
/// final label. Multiple journeys may be returned for a single query,
/// representing pareto-optimal trade-offs between fewer transfers and earlier
/// arrival.
///
/// `origin` is whichever of the user-supplied origin stops this journey
/// actually started from – relevant for multi-source queries (e.g. "any
/// platform of this station") where the algorithm picks the best origin
/// internally. Similarly `target` is the target stop reached.
///
/// `L` defaults to [`ArrivalTime`] for single-criterion routing.
#[derive(Debug, Clone)]
pub struct Journey<L: Label = ArrivalTime> {
    /// The origin stop this journey starts from, picked from the
    /// user-supplied origin set.
    pub origin: StopIdx,
    /// The target stop this journey ends at, picked from the user-supplied
    /// target set.
    pub target: StopIdx,
    /// Sequence of steps, each a (route, alight stop) pair.
    ///
    /// The origin stop is implicit – it is not part of the plan. Each entry
    /// means "take this route until this stop". The first step boards at the
    /// origin stop, and each subsequent step boards at the stop where the
    /// previous step got off (possibly via an intermediate footpath).
    pub plan: Vec<(RouteIdx, StopIdx)>,
    /// The label at the target stop, with the target's walk-time offset
    /// already folded in. For [`ArrivalTime`], `label.arrival()` is the
    /// effective arrival time in seconds since midnight.
    pub label: L,
}

impl<L: Label> Journey<L> {
    /// Convenience accessor: `self.label.arrival()`. The effective
    /// arrival time at the chosen target, with the target's
    /// walk-time offset already applied.
    pub fn arrival(&self) -> SecondOfDay {
        self.label.arrival()
    }

    /// Walk the plan against `tt` to recover the specific trip ridden
    /// for each leg, plus per-leg departure and arrival times.
    /// `depart` is the original query departure time and `origin_walk`
    /// is the walk-time offset for `self.origin` from the original
    /// origins slice (typically [`Duration::ZERO`] for single-stop
    /// queries).
    ///
    /// Each returned [`TimedLeg`] reports the route, boarding stop,
    /// alighting stop, the specific [`TripIdx`] caught, and the
    /// boarding / alighting times. If the previous leg's alight
    /// stop is not directly served by the next leg's route, this
    /// scans the previous alight stop's direct footpath neighbours
    /// for one that *is* served, walks there, and uses that as the
    /// next leg's `board`. The walking time advances `depart`
    /// without producing a separate leg – callers detect a walking
    /// transfer by comparing `legs[n].alight` with `legs[n+1].board`.
    ///
    /// # Errors
    ///
    /// Returns a [`TimingError`] when the plan can't be matched
    /// against `tt`. For a `Journey` produced by the same `tt` and
    /// `depart`, the only realistic failure is
    /// [`TimingError::NoBoardingStop`] when a transfer would need a
    /// walk chain longer than one direct footpath hop (multi-hop
    /// walk reconstruction is not implemented). The other variants
    /// are soundness escape hatches that indicate a programmer
    /// error or a `Journey` that was matched against a different
    /// timetable.
    ///
    /// **Loop routes:** if `route` revisits the boarding stop on
    /// its sequence, this picks the *earliest* qualifying position
    /// (matching what [`Timetable::get_routes_serving_stop`]
    /// reports). For non-loop routes (the common case) this is
    /// unambiguous; for loop-heavy networks the reconstructed trip
    /// matches the algorithm's choice in practice.
    pub fn with_timing<T: Timetable>(
        &self,
        tt: &T,
        depart: SecondOfDay,
        origin_walk: Duration,
    ) -> Result<Vec<TimedLeg>, TimingError> {
        let mut legs = Vec::with_capacity(self.plan.len());
        let mut current_time = depart + origin_walk;
        let mut current_stop = self.origin;

        for (leg, &(route, alight)) in self.plan.iter().enumerate() {
            // Find a stop on `route` reachable from current_stop —
            // either current_stop itself or, failing that, a one-hop
            // footpath neighbour that the route serves. Pick the
            // first matching neighbour in iteration order; for the
            // typical case of one neighbour on the route this is
            // unambiguous.
            let serving_here = tt.get_routes_serving_stop(current_stop);
            let (board, board_pos, walk_time) =
                if let Some(&(_, pos)) = serving_here.iter().find(|(r, _)| *r == route) {
                    (current_stop, pos, Duration::ZERO)
                } else {
                    let mut found = None;
                    for &neighbour in tt.get_footpaths_from(current_stop) {
                        if let Some(&(_, pos)) = tt
                            .get_routes_serving_stop(neighbour)
                            .iter()
                            .find(|(r, _)| *r == route)
                        {
                            let walk = tt.get_transfer_time(current_stop, neighbour);
                            found = Some((neighbour, pos, walk));
                            break;
                        }
                    }
                    found.ok_or(TimingError::NoBoardingStop {
                        leg,
                        route,
                        from_stop: current_stop,
                    })?
                };

            current_time = current_time + walk_time;

            let trip = tt.get_earliest_trip(route, current_time, board_pos).ok_or(
                TimingError::NoCatchableTrip {
                    leg,
                    route,
                    board_pos,
                    at: current_time,
                },
            )?;
            let depart = tt.get_departure_time(trip, board_pos);

            // Find the alight position by scanning forward from board_pos.
            let stops_ahead = tt.get_stops_after(route, board_pos);
            let alight_offset = stops_ahead.iter().position(|&s| s == alight).ok_or(
                TimingError::UnreachableAlight {
                    leg,
                    route,
                    board_pos,
                    alight,
                },
            )?;
            let alight_pos = board_pos + alight_offset as u32;
            let arrive = tt.get_arrival_time(trip, alight_pos);

            legs.push(TimedLeg {
                route,
                board,
                alight,
                trip,
                depart,
                arrive,
            });

            current_time = arrive;
            current_stop = alight;
        }

        Ok(legs)
    }
}

/// One transit leg of a [`Journey`] with reconstructed timing.
/// Produced by [`Journey::with_timing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimedLeg {
    /// The route boarded.
    pub route: RouteIdx,
    /// Stop where the rider boards.
    pub board: StopIdx,
    /// Stop where the rider alights.
    pub alight: StopIdx,
    /// The specific trip ridden – the earliest one departing at or
    /// after the rider's available time at `board`.
    pub trip: TripIdx,
    /// Departure time at `board`, in seconds since midnight.
    pub depart: SecondOfDay,
    /// Arrival time at `alight`, in seconds since midnight.
    pub arrive: SecondOfDay,
}

/// Failure modes for [`Journey::with_timing`].
///
/// Each variant carries the leg index where reconstruction stopped (zero-based)
/// and enough context to identify what went wrong. For a `Journey` produced by
/// the same timetable the algorithm ran against, the only variant a caller
/// realistically hits is [`TimingError::NoBoardingStop`] – and only when a
/// transfer needs a walk chain longer than one direct footpath hop. The other
/// two variants surface programmer errors (a `Journey` matched against a
/// different timetable, a custom adapter violating the no-overtaking contract).
#[non_exhaustive]
#[derive(Debug, Clone, thiserror::Error)]
pub enum TimingError {
    /// No stop in the plan-reconstruction frontier (the previous leg's
    /// alight stop or any of its one-hop footpath neighbours) is served
    /// by the route this leg wants to board. Multi-hop walk chains are
    /// not reconstructed – if the original journey actually walked
    /// through more than one intermediate stop, that walk can't be
    /// recovered from `Journey.plan` alone today.
    #[error(
        "leg {leg}: no boarding stop for route {route} reachable from {from_stop} \
         (or any one-hop walk neighbour)"
    )]
    NoBoardingStop {
        /// Zero-based plan index where reconstruction stopped.
        leg: usize,
        /// The route this leg wanted to board.
        route: RouteIdx,
        /// The stop the rider was at when reconstruction stopped (the
        /// previous leg's alight stop, or `self.origin` for `leg == 0`).
        from_stop: StopIdx,
    },
    /// No trip on the boarded route departs at or after the rider's
    /// available time. For a `Journey` produced by the same timetable
    /// this should not happen – surface as a programmer-error escape
    /// hatch.
    #[error(
        "leg {leg}: no trip on route {route} departs at or after {at} from position {board_pos}"
    )]
    NoCatchableTrip {
        /// Zero-based plan index where reconstruction stopped.
        leg: usize,
        /// The route this leg boarded.
        route: RouteIdx,
        /// The position within `route` where the rider boarded.
        board_pos: u32,
        /// The time at which the rider was ready to board.
        at: SecondOfDay,
    },
    /// The boarded route's stop sequence (from the boarding position
    /// onwards) does not include the claimed alighting stop. For a
    /// `Journey` produced by the same timetable this should not
    /// happen – surface as a programmer-error escape hatch.
    #[error("leg {leg}: route {route} does not reach stop {alight} from position {board_pos}")]
    UnreachableAlight {
        /// Zero-based plan index where reconstruction stopped.
        leg: usize,
        /// The route this leg boarded.
        route: RouteIdx,
        /// The position within `route` where the rider boarded.
        board_pos: u32,
        /// The stop the plan claimed the rider alighted at.
        alight: StopIdx,
    },
}
