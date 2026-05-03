//! [`Timetable`] implementation backed by a GTFS feed.
//!
//! Wraps a parsed [`Gtfs`] object and pre-computes lookup indices for
//! efficient route, stop, and trip queries.
//!
//! ## Synthetic routes
//!
//! A "RAPTOR route" is an equivalence class of trips with identical stop
//! sequences (the paper, §3.1). A GTFS `route_id` is *not* a RAPTOR route
//! — it routinely groups trips with different stop patterns
//! (short-turns, branching, deadheads). At construction time, this
//! adapter splits each `route_id` into one or more synthetic routes,
//! identified by [`RouteId`]. Trips on a synthetic route are
//! additionally split into non-overtaking sub-groups so that the
//! algorithm's binary-search-by-departure assumption holds. Use
//! [`GtfsTimetable::route_name`] to recover the original `route_id` for
//! display.

use std::{borrow::Cow, collections::BTreeMap};

use gtfs_structures::Gtfs;
use smallvec::SmallVec;

use crate::Timetable;

const TYPICAL_ROUTES_PER_STOP: usize = 8;
const TYPICAL_TRANSFERS_PER_STOP: usize = 4;
const DEFAULT_TRANSFER_TIME_SECONDS: usize = 300;

/// Synthetic RAPTOR route identifier.
///
/// One per equivalence class of trips with identical, non-overtaking
/// stop sequences. A single GTFS `route_id` may map to multiple
/// `RouteId`s when its trips have differing stop patterns or contain
/// overtaking pairs. Use [`GtfsTimetable::route_name`] to recover the
/// original GTFS `route_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RouteId(u32);

impl RouteId {
    fn idx(self) -> usize {
        self.0 as usize
    }
}

type RoutesForStops<'gtfs> = BTreeMap<&'gtfs str, SmallVec<[RouteId; TYPICAL_ROUTES_PER_STOP]>>;
type FootpathsForStops<'gtfs> =
    BTreeMap<&'gtfs str, SmallVec<[&'gtfs str; TYPICAL_TRANSFERS_PER_STOP]>>;

/// Errors that can occur when constructing a [`GtfsTimetable`].
#[derive(thiserror::Error, Debug)]
pub enum GtfsError {
    /// A trip referenced in the feed was not found.
    #[error("trip not found: {0}")]
    MissingTrip(String),
    /// A stop referenced by a trip was not found.
    #[error("stop not found: {0}")]
    MissingStop(String),
    /// A trip has no stop times defined.
    #[error("trip has no stop_times: {0}")]
    MissingStopTimes(String),
    /// A trip has a stop_time without a departure time, which the
    /// algorithm needs for binary-search ordering.
    #[error("stop_time has no departure_time: trip {trip}, stop {stop}")]
    MissingDepartureTime {
        /// The trip the stop_time belongs to.
        trip: String,
        /// The stop the stop_time refers to.
        stop: String,
    },
}

type GtfsResult<T> = std::result::Result<T, GtfsError>;

/// A [`Timetable`] implementation that wraps a parsed GTFS feed.
///
/// Constructed via [`GtfsTimetable::new`], which validates the feed,
/// splits each GTFS `route_id` into one or more [`RouteId`]s by stop
/// pattern and overtaking, and builds the lookup indices the algorithm
/// requires.
pub struct GtfsTimetable<'gtfs> {
    gtfs: &'gtfs Gtfs,

    routes_for_stops: RoutesForStops<'gtfs>,
    /// For each synthetic route, the original GTFS `route_id`.
    gtfs_route_for_route: Vec<&'gtfs str>,
    /// For each synthetic route, the stop sequence (shared across every
    /// trip on the route).
    stops_for_route: Vec<Vec<&'gtfs str>>,
    /// For each synthetic route, trips sorted by first-stop departure.
    /// All trips share `stops_for_route[r]` and pairwise do not overtake,
    /// so departure ordering at every stop matches the first-stop order.
    trips_for_route: Vec<Vec<&'gtfs str>>,
    footpaths_for_stops: FootpathsForStops<'gtfs>,
}

impl<'a> GtfsTimetable<'a> {
    /// Creates a new timetable from a parsed GTFS feed.
    ///
    /// Validates that every trip references existing stops and has
    /// stop_times with departure times, then splits each GTFS `route_id`
    /// into synthetic [`RouteId`]s as described in the module docs.
    ///
    /// # Footpath assumptions
    ///
    /// The adapter passes `transfers.txt` entries through to the
    /// [`Timetable::get_footpaths_from`] return as-is, without computing
    /// the transitive closure. The [`Timetable`] trait requires the
    /// footpath relation to be transitively closed (see the trait-level
    /// docs). For typical feeds whose `transfers.txt` consists of
    /// explicit station-pair entries this holds; for feeds that derive
    /// transfers from coordinates with a max-radius rule it may not, in
    /// which case the caller is responsible for pre-closing the data
    /// before constructing the [`Gtfs`] passed in here. (A built-in
    /// closure pass is on the roadmap but is opt-in because it is
    /// expensive on large feeds.)
    ///
    /// [`Timetable`]: crate::Timetable
    /// [`Timetable::get_footpaths_from`]: crate::Timetable::get_footpaths_from
    pub fn new(gtfs: &'a Gtfs) -> GtfsResult<Self> {
        let RouteIndex {
            routes_for_stops,
            gtfs_route_for_route,
            stops_for_route,
            trips_for_route,
        } = build_route_index(gtfs)?;
        let footpaths_for_stops = Self::cache_footpaths_for_stops(gtfs);

        Ok(Self {
            gtfs,
            routes_for_stops,
            gtfs_route_for_route,
            stops_for_route,
            trips_for_route,
            footpaths_for_stops,
        })
    }

    /// Returns the original GTFS `route_id` that this synthetic route
    /// was derived from. Use this to look up route metadata
    /// (`short_name`, `long_name`, etc.) on `gtfs.routes` when displaying
    /// a journey.
    pub fn route_name(&self, route: RouteId) -> &'a str {
        self.gtfs_route_for_route[route.idx()]
    }

    fn cache_footpaths_for_stops(gtfs: &'a Gtfs) -> FootpathsForStops<'a> {
        let mut footpaths_for_stops = FootpathsForStops::default();

        for (stop_id, stop) in &gtfs.stops {
            if stop.transfers.is_empty() {
                continue;
            }

            let targets: SmallVec<_> = stop
                .transfers
                .iter()
                .map(|t| t.to_stop_id.as_str())
                .collect();
            footpaths_for_stops.insert(stop_id.as_str(), targets);
        }

        footpaths_for_stops
    }
}

struct RouteIndex<'gtfs> {
    routes_for_stops: RoutesForStops<'gtfs>,
    gtfs_route_for_route: Vec<&'gtfs str>,
    stops_for_route: Vec<Vec<&'gtfs str>>,
    trips_for_route: Vec<Vec<&'gtfs str>>,
}

fn build_route_index<'gtfs>(gtfs: &'gtfs Gtfs) -> GtfsResult<RouteIndex<'gtfs>> {
    // 1. Validate trips and group by (route_id, stop_sequence).
    let mut groups: BTreeMap<(&'gtfs str, Vec<&'gtfs str>), Vec<&'gtfs str>> = BTreeMap::new();

    for (trip_id, trip) in &gtfs.trips {
        if trip.stop_times.is_empty() {
            return Err(GtfsError::MissingStopTimes(trip_id.clone()));
        }

        let mut stop_seq: Vec<&'gtfs str> = Vec::with_capacity(trip.stop_times.len());
        for st in &trip.stop_times {
            let stop_id = st.stop.id.as_str();
            gtfs.get_stop(stop_id)
                .map_err(|_| GtfsError::MissingStop(stop_id.to_owned()))?;
            if st.departure_time.is_none() {
                return Err(GtfsError::MissingDepartureTime {
                    trip: trip_id.clone(),
                    stop: stop_id.to_owned(),
                });
            }
            stop_seq.push(stop_id);
        }

        let route_id = trip.route_id.as_str();
        groups
            .entry((route_id, stop_seq))
            .or_default()
            .push(trip_id.as_str());
    }

    // 2. For each (route_id, stop_seq) group, sort trips by first-stop
    //    departure and split into non-overtaking sub-groups. Each
    //    sub-group becomes a synthetic RouteId.
    let mut gtfs_route_for_route: Vec<&'gtfs str> = Vec::new();
    let mut stops_for_route: Vec<Vec<&'gtfs str>> = Vec::new();
    let mut trips_for_route: Vec<Vec<&'gtfs str>> = Vec::new();
    let mut routes_for_stops: RoutesForStops<'gtfs> = BTreeMap::new();

    for ((gtfs_route_id, stop_seq), trips) in groups {
        let mut trips_with_schedules: Vec<(&'gtfs str, &'gtfs [gtfs_structures::StopTime])> = trips
            .into_iter()
            .map(|trip_id| {
                let trip = gtfs.get_trip(trip_id).expect("just inserted");
                (trip_id, trip.stop_times.as_slice())
            })
            .collect();
        trips_with_schedules.sort_by_key(|(_, st)| st[0].departure_time.expect("validated above"));

        for sub_group in split_non_overtaking(&trips_with_schedules) {
            let route = RouteId(gtfs_route_for_route.len() as u32);
            gtfs_route_for_route.push(gtfs_route_id);
            stops_for_route.push(stop_seq.clone());
            trips_for_route.push(sub_group);
            for &stop in &stop_seq {
                routes_for_stops.entry(stop).or_default().push(route);
            }
        }
    }

    Ok(RouteIndex {
        routes_for_stops,
        gtfs_route_for_route,
        stops_for_route,
        trips_for_route,
    })
}

/// Greedily split a departure-sorted list of trips on a shared stop
/// sequence into sub-groups within which no trip overtakes any earlier
/// trip in the same sub-group.
///
/// Insertion order: each trip is appended to the first sub-group whose
/// last trip it does not overtake; otherwise a new sub-group is opened.
/// Within a sub-group "doesn't overtake the last trip" extends to the
/// whole sub-group by transitivity (all members are pairwise
/// non-overtaking and times are monotone over the sequence).
fn split_non_overtaking<'gtfs>(
    trips: &[(&'gtfs str, &'gtfs [gtfs_structures::StopTime])],
) -> Vec<Vec<&'gtfs str>> {
    let mut sub_groups: Vec<Vec<(&'gtfs str, &'gtfs [gtfs_structures::StopTime])>> = Vec::new();
    'outer: for &entry in trips {
        for sub_group in &mut sub_groups {
            let (_, last_st) = *sub_group.last().expect("non-empty by construction");
            if !overtakes(last_st, entry.1) {
                sub_group.push(entry);
                continue 'outer;
            }
        }
        sub_groups.push(vec![entry]);
    }
    sub_groups
        .into_iter()
        .map(|g| g.into_iter().map(|(id, _)| id).collect())
        .collect()
}

/// Returns true if `later` overtakes `earlier` at any stop. Both
/// schedules are assumed to share a stop sequence and to have departure
/// times at every stop (validated at construction).
fn overtakes(earlier: &[gtfs_structures::StopTime], later: &[gtfs_structures::StopTime]) -> bool {
    earlier.iter().zip(later).any(|(es, ls)| {
        let e_dep = es.departure_time.expect("validated at construction");
        let l_dep = ls.departure_time.expect("validated at construction");
        if l_dep < e_dep {
            return true;
        }
        // A later trip whose arrival at some stop is earlier than the
        // earlier trip's arrival is also overtaking.
        matches!(
            (es.arrival_time, ls.arrival_time),
            (Some(e_arr), Some(l_arr)) if l_arr < e_arr
        )
    })
}

fn find_stop_time<'a>(gtfs: &'a Gtfs, trip: &str, stop: &str) -> &'a gtfs_structures::StopTime {
    let trip = gtfs.get_trip(trip).expect("validated during construction");
    trip.stop_times
        .iter()
        .find(|st| st.stop.id == stop)
        .expect("valid inputs")
}

impl<'gtfs> Timetable for GtfsTimetable<'gtfs> {
    type Stop = &'gtfs str;
    type Route = RouteId;
    type Trip = &'gtfs str;

    fn get_routes_serving_stop(&self, stop: Self::Stop) -> Cow<'_, [RouteId]> {
        self.routes_for_stops
            .get(&stop)
            .map(|sv| sv.as_slice())
            .unwrap_or(&[])
            .into()
    }

    fn get_earlier_stop(
        &self,
        route: Self::Route,
        left: Self::Stop,
        right: Self::Stop,
    ) -> Self::Stop {
        let stops = &self.stops_for_route[route.idx()];
        let left_pos = stops.iter().position(|&s| s == left);
        let right_pos = stops.iter().position(|&s| s == right);

        match (left_pos, right_pos) {
            (Some(l), Some(r)) if l <= r => left,
            (Some(_), Some(_)) => right,
            _ => panic!("both stops should exist on route"),
        }
    }

    fn get_stops_after(&self, route: Self::Route, stop: Self::Stop) -> Cow<'_, [&'gtfs str]> {
        let stops = &self.stops_for_route[route.idx()];
        let pos = stops
            .iter()
            .position(|&s| s == stop)
            .expect("stop should exist on route");
        Cow::Borrowed(&stops[pos..])
    }

    fn get_earliest_trip(
        &self,
        route: Self::Route,
        at: crate::Tau,
        stop: Self::Stop,
    ) -> Option<Self::Trip> {
        let trips = &self.trips_for_route[route.idx()];
        let stop_pos = self.stops_for_route[route.idx()]
            .iter()
            .position(|&s| s == stop)?;

        let departure_at_stop = |trip_id: &str| -> crate::Tau {
            let trip = self
                .gtfs
                .get_trip(trip_id)
                .expect("validated during construction");
            trip.stop_times[stop_pos]
                .departure_time
                .expect("validated during construction") as crate::Tau
        };

        // All trips share the stop sequence and are sorted by first-stop
        // departure. Because no pair overtakes (enforced at construction),
        // the trip order is also sorted by departure at every other stop.
        let idx = trips.partition_point(|&trip_id| departure_at_stop(trip_id) < at);
        trips.get(idx).copied()
    }

    fn get_arrival_time(&self, trip: Self::Trip, stop: Self::Stop) -> crate::Tau {
        find_stop_time(self.gtfs, trip, stop)
            .arrival_time
            .expect("valid inputs") as crate::Tau
    }

    fn get_departure_time(&self, trip: Self::Trip, stop: Self::Stop) -> crate::Tau {
        find_stop_time(self.gtfs, trip, stop)
            .departure_time
            .expect("valid inputs") as crate::Tau
    }

    fn get_footpaths_from(&self, stop: Self::Stop) -> Cow<'_, [Self::Stop]> {
        self.footpaths_for_stops
            .get(&stop)
            .map(|sv| sv.as_slice())
            .unwrap_or(&[])
            .into()
    }

    // TODO: handle TransferType to distinguish between timed transfers and walking
    fn get_transfer_time(&self, from: Self::Stop, to: Self::Stop) -> crate::Tau {
        self.gtfs
            .get_stop(from)
            .expect("validated during construction")
            .transfers
            .iter()
            .find(|t| t.to_stop_id == to)
            .and_then(|t| t.min_transfer_time)
            .map(|t| t as crate::Tau)
            .unwrap_or(DEFAULT_TRANSFER_TIME_SECONDS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtfs_structures::StopTime;

    fn st(arr: u32, dep: u32) -> StopTime {
        StopTime {
            arrival_time: Some(arr),
            departure_time: Some(dep),
            ..Default::default()
        }
    }

    #[test]
    fn overtakes_detects_arrival_inversion() {
        // Two stops; later trip arrives before earlier trip at the second stop.
        let earlier = vec![st(0, 0), st(20, 20)];
        let later = vec![st(5, 5), st(15, 15)];
        assert!(overtakes(&earlier, &later));
    }

    #[test]
    fn overtakes_detects_departure_inversion() {
        // Later trip's departure precedes earlier's at the second stop.
        let earlier = vec![st(0, 0), st(10, 30)];
        let later = vec![st(5, 5), st(10, 20)];
        assert!(overtakes(&earlier, &later));
    }

    #[test]
    fn non_overtaking_pair_is_clean() {
        let earlier = vec![st(0, 0), st(10, 10)];
        let later = vec![st(5, 5), st(15, 15)];
        assert!(!overtakes(&earlier, &later));
    }

    #[test]
    fn equal_schedules_do_not_overtake() {
        // Two trips with identical schedules don't overtake each other.
        let a = vec![st(0, 0), st(10, 10)];
        let b = vec![st(0, 0), st(10, 10)];
        assert!(!overtakes(&a, &b));
    }

    #[test]
    fn split_keeps_non_overtaking_trips_in_one_group() {
        let t1 = vec![st(0, 0), st(10, 10)];
        let t2 = vec![st(5, 5), st(15, 15)];
        let t3 = vec![st(10, 10), st(20, 20)];
        let trips = vec![
            ("t1", t1.as_slice()),
            ("t2", t2.as_slice()),
            ("t3", t3.as_slice()),
        ];
        let groups = split_non_overtaking(&trips);
        assert_eq!(groups, vec![vec!["t1", "t2", "t3"]]);
    }

    #[test]
    fn split_separates_overtaking_trips() {
        // t1 departs first but is overtaken by t2 (the express).
        let t1_local = vec![st(0, 0), st(60, 60)];
        let t2_express = vec![st(10, 10), st(20, 20)];
        let t3_local = vec![st(70, 70), st(130, 130)];
        let trips = vec![
            ("t1", t1_local.as_slice()),
            ("t2", t2_express.as_slice()),
            ("t3", t3_local.as_slice()),
        ];
        let groups = split_non_overtaking(&trips);
        // Two non-overtaking sub-groups: {t1, t3} (locals) and {t2} (express).
        // Greedy insertion places t2 in a new sub-group when it overtakes t1,
        // then t3 lands in the t1 sub-group since it doesn't overtake t1.
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0], vec!["t1", "t3"]);
        assert_eq!(groups[1], vec!["t2"]);
    }
}
