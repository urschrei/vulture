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
//! identified by [`RouteIdx`]. Trips on a synthetic route are
//! additionally split into non-overtaking sub-groups so that the
//! algorithm's binary-search-by-departure assumption holds.
//!
//! Use [`GtfsTimetable::route_id`] to recover the original GTFS
//! `route_id` for display, and [`GtfsTimetable::routes_for_gtfs_id`] to
//! enumerate every synthetic derived from a given GTFS route.

use std::collections::HashMap;

use gtfs_structures::Gtfs;
use smallvec::SmallVec;

use crate::{RouteIdx, StopIdx, Tau, Timetable, TripIdx};

const TYPICAL_ROUTES_PER_STOP: usize = 8;
const TYPICAL_TRANSFERS_PER_STOP: usize = 4;
const DEFAULT_TRANSFER_TIME_SECONDS: usize = 300;

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
    /// A trip has a stop_time without a departure time, which the algorithm
    /// needs for binary-search ordering.
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
/// interns stops/routes/trips to dense `u32` indices, splits each GTFS
/// `route_id` into one or more [`RouteIdx`]s by stop pattern and
/// overtaking, and builds the lookup indices the algorithm requires.
pub struct GtfsTimetable<'gtfs> {
    gtfs: &'gtfs Gtfs,

    // Forward tables: idx -> &'gtfs str (original GTFS IDs).
    stop_ids: Vec<&'gtfs str>,
    route_ids: Vec<&'gtfs str>,
    trip_ids: Vec<&'gtfs str>,

    // Reverse tables.
    stop_by_id: HashMap<&'gtfs str, StopIdx>,
    route_by_id: HashMap<&'gtfs str, RouteIdx>,
    routes_by_gtfs_id: HashMap<&'gtfs str, SmallVec<[RouteIdx; 2]>>,
    trip_by_id: HashMap<&'gtfs str, TripIdx>,

    // Per-route tables.
    routes_for_stop: Vec<SmallVec<[RouteIdx; TYPICAL_ROUTES_PER_STOP]>>,
    stops_for_route: Vec<Vec<StopIdx>>,
    trips_for_route: Vec<Vec<TripIdx>>,

    footpaths_for_stops: Vec<SmallVec<[StopIdx; TYPICAL_TRANSFERS_PER_STOP]>>,
    transfer_times: HashMap<(StopIdx, StopIdx), Tau>,
}

impl<'gtfs> GtfsTimetable<'gtfs> {
    /// Creates a new timetable from a parsed GTFS feed.
    ///
    /// Validates that every trip references existing stops and has
    /// stop_times with departure times, then interns identifiers to dense
    /// `u32` indices and splits each GTFS `route_id` into synthetic
    /// [`RouteIdx`]s as described in the module docs.
    ///
    /// # Footpath assumptions
    ///
    /// The adapter passes `transfers.txt` entries through to the
    /// [`Timetable::get_footpaths_from`] return as-is, without computing
    /// the transitive closure. The [`Timetable`] trait requires the
    /// footpath relation to be transitively closed (see the trait-level
    /// docs).
    pub fn new(gtfs: &'gtfs Gtfs) -> GtfsResult<Self> {
        // 1. Intern stops in iteration order.
        let mut stop_ids: Vec<&'gtfs str> = Vec::with_capacity(gtfs.stops.len());
        let mut stop_by_id: HashMap<&'gtfs str, StopIdx> = HashMap::with_capacity(gtfs.stops.len());
        for stop_id in gtfs.stops.keys() {
            let idx = StopIdx::new(stop_ids.len() as u32);
            stop_ids.push(stop_id.as_str());
            stop_by_id.insert(stop_id.as_str(), idx);
        }

        // 2. Validate trips and group by (route_id, stop_sequence) using
        //    interned stop indices.
        let mut groups: std::collections::BTreeMap<(&'gtfs str, Vec<StopIdx>), Vec<&'gtfs str>> =
            std::collections::BTreeMap::new();
        for (trip_id, trip) in &gtfs.trips {
            if trip.stop_times.is_empty() {
                return Err(GtfsError::MissingStopTimes(trip_id.clone()));
            }
            let mut stop_seq: Vec<StopIdx> = Vec::with_capacity(trip.stop_times.len());
            for st in &trip.stop_times {
                let raw_id = st.stop.id.as_str();
                let stop_idx = *stop_by_id
                    .get(raw_id)
                    .ok_or_else(|| GtfsError::MissingStop(raw_id.to_owned()))?;
                if st.departure_time.is_none() {
                    return Err(GtfsError::MissingDepartureTime {
                        trip: trip_id.clone(),
                        stop: raw_id.to_owned(),
                    });
                }
                stop_seq.push(stop_idx);
            }
            groups
                .entry((trip.route_id.as_str(), stop_seq))
                .or_default()
                .push(trip_id.as_str());
        }

        // 3. For each (route_id, stop_seq) group, sort trips by first-stop
        //    departure and split into non-overtaking sub-groups. Each
        //    sub-group becomes a synthetic RouteIdx; trips become TripIdxs
        //    in synthetic-route order.
        let mut route_ids: Vec<&'gtfs str> = Vec::new();
        let mut stops_for_route: Vec<Vec<StopIdx>> = Vec::new();
        let mut trips_for_route: Vec<Vec<TripIdx>> = Vec::new();
        let mut trip_ids: Vec<&'gtfs str> = Vec::new();
        let mut trip_by_id: HashMap<&'gtfs str, TripIdx> = HashMap::new();
        let mut route_by_id: HashMap<&'gtfs str, RouteIdx> = HashMap::new();
        let mut routes_by_gtfs_id: HashMap<&'gtfs str, SmallVec<[RouteIdx; 2]>> = HashMap::new();
        let mut routes_for_stop: Vec<SmallVec<[RouteIdx; TYPICAL_ROUTES_PER_STOP]>> =
            vec![SmallVec::new(); stop_ids.len()];

        for ((gtfs_route_id, stop_seq), trips) in groups {
            let mut trips_with_schedules: Vec<(&'gtfs str, &'gtfs [gtfs_structures::StopTime])> =
                trips
                    .into_iter()
                    .map(|trip_id| {
                        let trip = gtfs.get_trip(trip_id).expect("just inserted");
                        (trip_id, trip.stop_times.as_slice())
                    })
                    .collect();
            trips_with_schedules
                .sort_by_key(|(_, st)| st[0].departure_time.expect("validated above"));

            for sub_group in split_non_overtaking(&trips_with_schedules) {
                let route_idx = RouteIdx::new(route_ids.len() as u32);
                route_ids.push(gtfs_route_id);
                stops_for_route.push(stop_seq.clone());

                let mut sub_trip_idxs: Vec<TripIdx> = Vec::with_capacity(sub_group.len());
                for trip_id in &sub_group {
                    let trip_idx = TripIdx::new(trip_ids.len() as u32);
                    trip_ids.push(trip_id);
                    trip_by_id.insert(trip_id, trip_idx);
                    sub_trip_idxs.push(trip_idx);
                }
                trips_for_route.push(sub_trip_idxs);

                route_by_id.entry(gtfs_route_id).or_insert(route_idx);
                routes_by_gtfs_id
                    .entry(gtfs_route_id)
                    .or_default()
                    .push(route_idx);

                for &stop_idx in &stop_seq {
                    routes_for_stop[stop_idx.idx()].push(route_idx);
                }
            }
        }

        // 4. Footpaths and transfer times.
        let mut footpaths_for_stops: Vec<SmallVec<[StopIdx; TYPICAL_TRANSFERS_PER_STOP]>> =
            vec![SmallVec::new(); stop_ids.len()];
        let mut transfer_times: HashMap<(StopIdx, StopIdx), Tau> = HashMap::new();
        for (stop_id, stop) in &gtfs.stops {
            if stop.transfers.is_empty() {
                continue;
            }
            let from_idx = *stop_by_id.get(stop_id.as_str()).expect("stop interned");
            for t in &stop.transfers {
                let Some(&to_idx) = stop_by_id.get(t.to_stop_id.as_str()) else {
                    continue;
                };
                footpaths_for_stops[from_idx.idx()].push(to_idx);
                if let Some(min) = t.min_transfer_time {
                    transfer_times.insert((from_idx, to_idx), min as Tau);
                }
            }
        }

        Ok(Self {
            gtfs,
            stop_ids,
            route_ids,
            trip_ids,
            stop_by_id,
            route_by_id,
            routes_by_gtfs_id,
            trip_by_id,
            routes_for_stop,
            stops_for_route,
            trips_for_route,
            footpaths_for_stops,
            transfer_times,
        })
    }

    /// Returns the original GTFS `stop_id` for the given index.
    pub fn stop_id(&self, stop: StopIdx) -> &'gtfs str {
        self.stop_ids[stop.idx()]
    }

    /// Returns the original GTFS `route_id` for the given synthetic route.
    /// Several `RouteIdx`s may map to the same GTFS `route_id`.
    pub fn route_id(&self, route: RouteIdx) -> &'gtfs str {
        self.route_ids[route.idx()]
    }

    /// Returns the original GTFS `trip_id` for the given index.
    pub fn trip_id(&self, trip: TripIdx) -> &'gtfs str {
        self.trip_ids[trip.idx()]
    }

    /// Looks up the index of a stop by its GTFS `stop_id`.
    pub fn stop_idx(&self, id: &str) -> Option<StopIdx> {
        self.stop_by_id.get(id).copied()
    }

    /// Looks up the *first* synthetic route derived from a GTFS
    /// `route_id`. Use [`routes_for_gtfs_id`](Self::routes_for_gtfs_id) to
    /// enumerate every synthetic.
    pub fn route_idx(&self, id: &str) -> Option<RouteIdx> {
        self.route_by_id.get(id).copied()
    }

    /// Returns every synthetic route derived from a given GTFS `route_id`.
    pub fn routes_for_gtfs_id(&self, id: &str) -> &[RouteIdx] {
        self.routes_by_gtfs_id
            .get(id)
            .map(|sv| sv.as_slice())
            .unwrap_or(&[])
    }

    /// Looks up the index of a trip by its GTFS `trip_id`.
    pub fn trip_idx(&self, id: &str) -> Option<TripIdx> {
        self.trip_by_id.get(id).copied()
    }
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
    fn n_stops(&self) -> usize {
        self.stop_ids.len()
    }

    fn n_routes(&self) -> usize {
        self.route_ids.len()
    }

    fn get_routes_serving_stop(&self, stop: StopIdx) -> &[RouteIdx] {
        self.routes_for_stop[stop.idx()].as_slice()
    }

    fn get_earlier_stop(&self, route: RouteIdx, left: StopIdx, right: StopIdx) -> StopIdx {
        let stops = &self.stops_for_route[route.idx()];
        let left_pos = stops.iter().position(|&s| s == left);
        let right_pos = stops.iter().position(|&s| s == right);
        match (left_pos, right_pos) {
            (Some(l), Some(r)) if l <= r => left,
            (Some(_), Some(_)) => right,
            _ => panic!("both stops should exist on route"),
        }
    }

    fn get_stops_after(&self, route: RouteIdx, stop: StopIdx) -> &[StopIdx] {
        let stops = &self.stops_for_route[route.idx()];
        let pos = stops
            .iter()
            .position(|&s| s == stop)
            .expect("stop should exist on route");
        &stops[pos..]
    }

    fn get_earliest_trip(&self, route: RouteIdx, at: Tau, stop: StopIdx) -> Option<TripIdx> {
        let trips = &self.trips_for_route[route.idx()];
        let stop_pos = self.stops_for_route[route.idx()]
            .iter()
            .position(|&s| s == stop)?;

        let departure_at_stop = |trip_idx: TripIdx| -> Tau {
            let raw = self.trip_ids[trip_idx.idx()];
            let trip = self
                .gtfs
                .get_trip(raw)
                .expect("validated during construction");
            trip.stop_times[stop_pos]
                .departure_time
                .expect("validated during construction") as Tau
        };

        let idx = trips.partition_point(|&trip_idx| departure_at_stop(trip_idx) < at);
        trips.get(idx).copied()
    }

    fn get_arrival_time(&self, trip: TripIdx, stop: StopIdx) -> Tau {
        let raw_trip = self.trip_ids[trip.idx()];
        let raw_stop = self.stop_ids[stop.idx()];
        find_stop_time(self.gtfs, raw_trip, raw_stop)
            .arrival_time
            .expect("valid inputs") as Tau
    }

    fn get_departure_time(&self, trip: TripIdx, stop: StopIdx) -> Tau {
        let raw_trip = self.trip_ids[trip.idx()];
        let raw_stop = self.stop_ids[stop.idx()];
        find_stop_time(self.gtfs, raw_trip, raw_stop)
            .departure_time
            .expect("valid inputs") as Tau
    }

    fn get_footpaths_from(&self, stop: StopIdx) -> &[StopIdx] {
        self.footpaths_for_stops[stop.idx()].as_slice()
    }

    fn get_transfer_time(&self, from: StopIdx, to: StopIdx) -> Tau {
        self.transfer_times
            .get(&(from, to))
            .copied()
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
