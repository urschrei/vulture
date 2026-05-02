#![deny(missing_docs)]

//! Rust implementation of the RAPTOR (Round-bAsed Public Transit Routing) algorithm.
//!
//! RAPTOR computes pareto-optimal journeys in a public transit network, trading off
//! between arrival time and number of transfers. Implement the [`Timetable`] trait
//! for your transit data, then call [`Timetable::raptor`] to query for journeys.
//!
//! A ready-made implementation for GTFS feeds is available in the [`gtfs`] module.
//!
//! # Example
//!
//! ```no_run
//! use raptor::Timetable;
//!
//! // implement Timetable for your transit data, then:
//! // let journeys = timetable.raptor(max_transfers, departure_time, source, target);
//! ```
//!
//! Based on the paper:
//! *Round-Based Public Transit Routing* by Daniel Delling, Thomas Pajor, and Renato F. Werneck.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;

pub mod gtfs;
/// In-memory timetable for testing and simple use cases.
pub mod simple;

#[cfg(test)]
mod test;

#[cfg(test)]
mod proptest_support;

/// The number of transfers (round number in the RAPTOR algorithm).
pub type K = usize;

/// Time value in seconds since midnight.
pub type Tau = usize;

/// A journey found by the RAPTOR algorithm.
///
/// Each journey consists of a sequence of steps (route, arrival stop) and a final arrival time.
/// Multiple journeys may be returned for a single query, representing pareto-optimal trade-offs
/// between fewer transfers and earlier arrival.
#[derive(Debug, Clone)]
pub struct Journey<Route, Stop> {
    /// Sequence of steps, each a (route, stop to get off at) pair.
    ///
    /// The source stop is implicit — it is not part of the plan. Each entry means
    /// "take this route until this stop". The first step boards at the source stop
    /// passed to [`Timetable::raptor`], and each subsequent step boards at the stop
    /// where the previous step got off.
    ///
    /// For example, going from stop `"A"` to stop `"D"` with two transfers, the plan
    /// would look like:
    ///
    /// ```json
    /// [("R1", "B"), ("R2", "C"), ("R3", "D")]
    /// ```
    ///
    /// Read as: board `R1` at `A`, get off at `B`, board `R2` at `B`, get off at `C`,
    /// board `R3` at `C`, get off at `D`.
    ///
    /// See the [`gtfs-timetable`](https://github.com/keogami/raptor-rs/blob/main/examples/gtfs-timetable.rs)
    /// example for how to interpret and display a plan.
    pub plan: Vec<(Route, Stop)>,
    /// Arrival time at the target stop, in seconds since midnight.
    pub arrival: Tau,
}

type BoardingTree<Route, Stop> = BTreeMap<(K, Stop), (Stop, Route)>;

fn reconstruct_journey<R, S>(
    tree: &BoardingTree<R, S>,
    ps: S,
    pt: S,
    transfers: K,
) -> Vec<Vec<(R, S)>>
where
    S: Ord + Copy + Debug,
    R: Copy + Debug,
{
    if tree.is_empty() {
        // Either no trips were taken, or we never reached target. The latter is
        // possible if ps and pt are nodes of a disjoint graph
        return Default::default();
    }

    let mut plans = Vec::new();

    for k in 1..=transfers {
        let mut plan = Vec::with_capacity(k);
        let mut parent = pt;

        log::debug!("outer_k = {k} | parent = {parent:?} | plans = {plans:?}");

        for inner_k in (1..=k).rev() {
            log::debug!("inner_k = {inner_k} | parent = {parent:?} | plan = {plan:?}");
            if parent == ps {
                log::debug!("stopping because parent is ps");
                break;
            }

            let Some((stop, route)) = tree.get(&(inner_k, parent)).copied() else {
                log::debug!("stopping because tree has no entry for current (inner_k, parent)");
                break;
            };

            plan.push((route, parent));
            parent = stop;
        }

        if !plan.is_empty() && parent == ps {
            plan.reverse();
            plans.push(plan)
        }
    }

    plans
}

/// Models a route-based transit network for the RAPTOR algorithm.
///
/// Implement this trait to describe your transit network's topology and schedule.
/// The algorithm itself is provided as a default method ([`Timetable::raptor`]).
pub trait Timetable {
    /// Identifier for a transit stop.
    type Stop: Ord + Copy + Debug;
    /// Identifier for a transit route.
    type Route: Ord + Copy + Debug;
    /// Identifier for a specific trip (a single run of a route).
    type Trip: Copy + Debug;

    /// Returns all routes that serve the given stop.
    fn get_routes_serving_stop(&self, stop: Self::Stop) -> Cow<'_, [Self::Route]>;

    /// Given two stops on a route, returns whichever appears earlier in the route's sequence.
    fn get_earlier_stop(
        &self,
        route: Self::Route,
        left: Self::Stop,
        right: Self::Stop,
    ) -> Self::Stop;

    /// Returns all stops on a route from the given stop onwards (inclusive), in sequence order.
    fn get_stops_after(&self, route: Self::Route, stop: Self::Stop) -> Cow<'_, [Self::Stop]>;

    /// Finds the earliest trip on a route departing at or after `at` from `stop`.
    ///
    /// Returns `None` if no such trip exists.
    fn get_earliest_trip(
        &self,
        route: Self::Route,
        at: Tau,
        stop: Self::Stop,
    ) -> Option<Self::Trip>;

    /// Returns the arrival time of a trip at a stop.
    fn get_arrival_time(&self, trip: Self::Trip, stop: Self::Stop) -> Tau;

    /// Returns the departure time of a trip at a stop.
    fn get_departure_time(&self, trip: Self::Trip, stop: Self::Stop) -> Tau;

    /// Returns all stops reachable from the given stop via walking (footpaths).
    fn get_footpaths_from(&self, stop: Self::Stop) -> Cow<'_, [Self::Stop]>;

    /// Returns the walking transfer time between two stops, in seconds.
    ///
    /// The default implementation returns `1`. Override this for realistic transfer times.
    fn get_transfer_time(&self, from: Self::Stop, to: Self::Stop) -> Tau {
        let (_, _) = (from, to);
        1
    }

    /// Runs the RAPTOR algorithm and returns all pareto-optimal journeys.
    ///
    /// Finds journeys from `ps` (source) to `pt` (target) departing at or after `tau`,
    /// using at most `transfers` steps. Returns a set of pareto-optimal journeys trading
    /// off between fewer transfers and earlier arrival.
    ///
    /// Returns an empty `Vec` if no journey exists.
    fn raptor(
        &self,
        transfers: usize,
        tau: usize,
        ps: Self::Stop,
        pt: Self::Stop,
    ) -> Vec<Journey<Self::Route, Self::Stop>> {
        // for (i, stop) earliest known arrival time at `stop` with at most `i` transfers
        let mut best_arrival_per_k = BTreeMap::<(K, Self::Stop), Tau>::new();
        let mut best_arrival = BTreeMap::<Self::Stop, Tau>::new();

        best_arrival_per_k.insert((0, ps), tau);
        let mut board_detail_per_k: BoardingTree<Self::Route, Self::Stop> = BTreeMap::new();

        let mut marked_stops = BTreeSet::<Self::Stop>::from([ps]);

        #[allow(non_snake_case)]
        // allowing weird naming to match with the paper
        let mut Q = BTreeMap::<Self::Route, Self::Stop>::new();

        for k in 1..=transfers {
            Q.clear();
            // find all routes that serve the marked stops, for evaluation in this round
            for &marked_stop in &marked_stops {
                for &route in self.get_routes_serving_stop(marked_stop).iter() {
                    let p_dash = Q.entry(route).or_insert(marked_stop);

                    *p_dash = self.get_earlier_stop(route, marked_stop, *p_dash);
                }
            }

            marked_stops.clear();

            // scanning each route
            for (&route, &p) in Q.iter() {
                let mut current_trip: Option<Self::Trip> = None;
                let mut boarding_stop = p;

                for &pi in self.get_stops_after(route, p).iter() {
                    if let Some(arr) = current_trip.map(|trip| self.get_arrival_time(trip, pi)) {
                        let best_arrival_to_target = best_arrival.get(&pt).unwrap_or(&Tau::MAX);
                        let best_arrival_to_pi = best_arrival.get(&pi).unwrap_or(&Tau::MAX);
                        let time_to_beat = *best_arrival_to_pi.min(best_arrival_to_target);

                        if arr < time_to_beat {
                            board_detail_per_k.insert((k, pi), (boarding_stop, route));
                            best_arrival_per_k.insert((k, pi), arr);
                            best_arrival.insert(pi, arr);
                            marked_stops.insert(pi);
                        }
                    }

                    let t_prev_pi = *best_arrival_per_k.get(&(k - 1, pi)).unwrap_or(&Tau::MAX);
                    if t_prev_pi
                        <= current_trip
                            .map(|trip| self.get_departure_time(trip, pi))
                            .unwrap_or(Tau::MAX)
                    {
                        current_trip = self.get_earliest_trip(route, t_prev_pi, pi);
                        boarding_stop = pi;
                    }
                }
            }

            // look at footpaths, and mark the stops reachable
            let mut more_marked_stops = Vec::new();
            for &stop in &marked_stops {
                for &p_dash in self.get_footpaths_from(stop).iter() {
                    let tau = best_arrival_per_k
                        .get(&(k, p_dash))
                        .copied()
                        .unwrap_or(Tau::MAX)
                        .min(
                            best_arrival_per_k
                                .get(&(k, stop))
                                .copied()
                                .unwrap_or(Tau::MAX)
                                + self.get_transfer_time(stop, p_dash),
                        );
                    best_arrival_per_k.insert((k, p_dash), tau);
                    more_marked_stops.push(p_dash);
                }
            }

            marked_stops.extend(&more_marked_stops);

            if marked_stops.is_empty() {
                break;
            }
        }

        let plans = reconstruct_journey(&board_detail_per_k, ps, pt, transfers);

        plans
            .into_iter()
            .map(|plan| {
                let arrival = *best_arrival_per_k.get(&(plan.len(), pt)).unwrap();

                Journey { plan, arrival }
            })
            .collect()
    }
}
