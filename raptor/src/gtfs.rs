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

use chrono::{Datelike, NaiveDate, Weekday};
use gtfs_structures::{Exception, Gtfs};
use jiff::civil::Date;
use rstar::{AABB, PointDistance, RTree, RTreeObject};
use smallvec::SmallVec;

use crate::{RouteIdx, StopIdx, Tau, Timetable, TripIdx};

/// Mean Earth radius in metres, used to project stop coordinates to a
/// local Cartesian frame for the `with_walking_footpaths` spatial query.
const EARTH_RADIUS_M: f64 = 6_371_000.0;

/// A stop projected to local Cartesian coordinates (metres) via an
/// equirectangular projection anchored at a reference latitude. Used as
/// the leaf type of the R-tree built by `with_walking_footpaths`.
#[derive(Clone, Copy, Debug)]
struct ProjectedStop {
    pos: [f64; 2],
    idx: StopIdx,
}

impl RTreeObject for ProjectedStop {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_point(self.pos)
    }
}

impl PointDistance for ProjectedStop {
    fn distance_2(&self, point: &[f64; 2]) -> f64 {
        let dx = self.pos[0] - point[0];
        let dy = self.pos[1] - point[1];
        dx * dx + dy * dy
    }
}

/// Returns true iff `service_id` is active on `date` per the GTFS feed's
/// `calendar.txt` and `calendar_dates.txt` rules.
///
/// Resolution order: an entry in `calendar_dates.txt` for the exact
/// date trumps `calendar.txt`. If `calendar_dates.txt` has no entry,
/// `calendar.txt` decides via the day-of-week flags constrained by
/// the service's `start_date`/`end_date` window. A service that
/// appears in neither file is considered inactive.
fn is_service_active(gtfs: &Gtfs, service_id: &str, date: NaiveDate) -> bool {
    if let Some(cdates) = gtfs.calendar_dates.get(service_id)
        && let Some(cd) = cdates.iter().find(|cd| cd.date == date)
    {
        return matches!(cd.exception_type, Exception::Added);
    }
    if let Some(cal) = gtfs.calendar.get(service_id) {
        if date < cal.start_date || date > cal.end_date {
            return false;
        }
        return match date.weekday() {
            Weekday::Mon => cal.monday,
            Weekday::Tue => cal.tuesday,
            Weekday::Wed => cal.wednesday,
            Weekday::Thu => cal.thursday,
            Weekday::Fri => cal.friday,
            Weekday::Sat => cal.saturday,
            Weekday::Sun => cal.sunday,
        };
    }
    false
}

/// Convert a `jiff::civil::Date` to `chrono::NaiveDate` at the GTFS
/// boundary. `gtfs-structures` exposes calendar dates as `chrono`; the
/// rest of this crate uses `jiff` so users only see one date type.
fn jiff_to_chrono(d: Date) -> NaiveDate {
    NaiveDate::from_ymd_opt(d.year() as i32, d.month() as u32, d.day() as u32)
        .expect("jiff date is a valid civil date")
}

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
    // Forward tables: idx -> &'gtfs str (original GTFS IDs).
    stop_ids: Vec<&'gtfs str>,
    route_ids: Vec<&'gtfs str>,
    trip_ids: Vec<&'gtfs str>,

    // Reverse tables.
    stop_by_id: HashMap<&'gtfs str, StopIdx>,
    route_by_id: HashMap<&'gtfs str, RouteIdx>,
    routes_by_gtfs_id: HashMap<&'gtfs str, SmallVec<[RouteIdx; 2]>>,
    trip_by_id: HashMap<&'gtfs str, TripIdx>,

    // For each stop, the routes serving it paired with the *earliest*
    // position of the stop on that route. Each route appears at most once
    // per stop (loop routes that revisit the stop only get their earliest
    // position recorded).
    routes_for_stop: Vec<SmallVec<[(RouteIdx, u32); TYPICAL_ROUTES_PER_STOP]>>,
    stops_for_route: Vec<Vec<StopIdx>>,
    trips_for_route: Vec<Vec<TripIdx>>,
    /// arrival_times[route.idx()][stop_pos][trip_pos] = Tau
    arrival_times: Vec<Vec<Vec<Tau>>>,
    /// departure_times[route.idx()][stop_pos][trip_pos] = Tau
    departure_times: Vec<Vec<Vec<Tau>>>,
    /// route_for_trip[trip.idx()] = (route_idx, position-within-route)
    route_for_trip: Vec<(RouteIdx, usize)>,

    footpaths_for_stops: Vec<SmallVec<[StopIdx; TYPICAL_TRANSFERS_PER_STOP]>>,
    transfer_times: HashMap<(StopIdx, StopIdx), Tau>,

    /// User-asserted closure flag. Returned from
    /// [`Timetable::footpaths_are_transitively_closed`] so the algorithm
    /// can pick the single-pass relaxation. Set via
    /// [`GtfsTimetable::assert_footpaths_closed`]; reset to `false` by
    /// [`GtfsTimetable::with_walking_footpaths`] (which adds direct,
    /// non-closed edges).
    transfers_closed: bool,

    /// For each parent-station GTFS id, the child platform `StopIdx`es
    /// (each paired with a default zero walk time, ready to pass to
    /// [`Timetable::raptor`] as a multi-source/multi-target query).
    station_children: HashMap<&'gtfs str, Vec<(StopIdx, Tau)>>,
}

impl<'gtfs> GtfsTimetable<'gtfs> {
    /// Creates a new timetable from a parsed GTFS feed for a specific
    /// service date.
    ///
    /// Trips whose `service_id` is not active on `service_date` (per
    /// `calendar.txt` and `calendar_dates.txt`) are filtered out at
    /// construction. The returned timetable contains only trips that
    /// run on `service_date`.
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
    pub fn new(gtfs: &'gtfs Gtfs, service_date: Date) -> GtfsResult<Self> {
        let date_chrono = jiff_to_chrono(service_date);

        // 1. Intern stops in iteration order.
        let mut stop_ids: Vec<&'gtfs str> = Vec::with_capacity(gtfs.stops.len());
        let mut stop_by_id: HashMap<&'gtfs str, StopIdx> = HashMap::with_capacity(gtfs.stops.len());
        for stop_id in gtfs.stops.keys() {
            let idx = StopIdx::new(stop_ids.len() as u32);
            stop_ids.push(stop_id.as_str());
            stop_by_id.insert(stop_id.as_str(), idx);
        }

        // 2. Validate trips active on `service_date` and group by
        //    (route_id, stop_sequence) using interned stop indices. Trips
        //    on inactive services are skipped silently.
        let mut groups: std::collections::BTreeMap<(&'gtfs str, Vec<StopIdx>), Vec<&'gtfs str>> =
            std::collections::BTreeMap::new();
        for (trip_id, trip) in &gtfs.trips {
            if !is_service_active(gtfs, &trip.service_id, date_chrono) {
                continue;
            }
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
        let mut arrival_times: Vec<Vec<Vec<Tau>>> = Vec::new();
        let mut departure_times: Vec<Vec<Vec<Tau>>> = Vec::new();
        let mut route_for_trip: Vec<(RouteIdx, usize)> = Vec::with_capacity(gtfs.trips.len());
        let mut trip_ids: Vec<&'gtfs str> = Vec::new();
        let mut trip_by_id: HashMap<&'gtfs str, TripIdx> = HashMap::new();
        let mut route_by_id: HashMap<&'gtfs str, RouteIdx> = HashMap::new();
        let mut routes_by_gtfs_id: HashMap<&'gtfs str, SmallVec<[RouteIdx; 2]>> = HashMap::new();
        let mut routes_for_stop: Vec<SmallVec<[(RouteIdx, u32); TYPICAL_ROUTES_PER_STOP]>> =
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
                    debug_assert_eq!(route_for_trip.len(), trip_idx.idx());
                    route_for_trip.push((route_idx, sub_trip_idxs.len() - 1));
                }
                trips_for_route.push(sub_trip_idxs);

                // Per-route arrival/departure tables: shape [stop_pos][trip_pos].
                let n_stops_in_route = stop_seq.len();
                let n_trips_in_route = sub_group.len();
                let mut arr_table: Vec<Vec<Tau>> =
                    vec![vec![Tau::MAX; n_trips_in_route]; n_stops_in_route];
                let mut dep_table: Vec<Vec<Tau>> =
                    vec![vec![Tau::MAX; n_trips_in_route]; n_stops_in_route];
                for (trip_pos, trip_id) in sub_group.iter().enumerate() {
                    let trip = gtfs.get_trip(trip_id).expect("validated above");
                    for (stop_pos, st) in trip.stop_times.iter().enumerate() {
                        if let Some(a) = st.arrival_time {
                            arr_table[stop_pos][trip_pos] = a as Tau;
                        }
                        let d = st.departure_time.expect("validated at construction");
                        dep_table[stop_pos][trip_pos] = d as Tau;
                    }
                }
                arrival_times.push(arr_table);
                departure_times.push(dep_table);

                route_by_id.entry(gtfs_route_id).or_insert(route_idx);
                routes_by_gtfs_id
                    .entry(gtfs_route_id)
                    .or_default()
                    .push(route_idx);

                for (pos, &stop_idx) in stop_seq.iter().enumerate() {
                    let entry = &mut routes_for_stop[stop_idx.idx()];
                    if !entry.iter().any(|(r, _)| *r == route_idx) {
                        entry.push((route_idx, pos as u32));
                    }
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

        // 5. Group child stops by their parent_station, so that callers
        //    can later query "all platforms of station X" with one lookup.
        let mut station_children: HashMap<&'gtfs str, Vec<(StopIdx, Tau)>> = HashMap::new();
        for (stop_id, stop) in &gtfs.stops {
            if let Some(parent) = stop.parent_station.as_deref()
                && let Some(&child_idx) = stop_by_id.get(stop_id.as_str())
            {
                // Resolve `parent` against gtfs.stops to find its &'gtfs str
                // key (so the map's key has the right lifetime).
                if let Some((parent_key, _)) = gtfs.stops.get_key_value(parent) {
                    station_children
                        .entry(parent_key.as_str())
                        .or_default()
                        .push((child_idx, 0));
                }
            }
        }

        Ok(Self {
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
            arrival_times,
            departure_times,
            route_for_trip,
            footpaths_for_stops,
            transfer_times,
            transfers_closed: false,
            station_children,
        })
    }

    /// Asserts that the current footpath relation is transitively
    /// closed and instructs the algorithm to use the single-pass
    /// `O(E)` relaxation instead of multi-source Dijkstra. Returns
    /// `self` for chaining.
    ///
    /// Use when you know the underlying `transfers.txt` (or any
    /// pre-processing you've applied) is closed — typically true for
    /// publisher-curated feeds like Berlin VBB or Paris IDFM.
    ///
    /// **Soundness**: asserting closure on a non-closed relation will
    /// cause the algorithm to miss journeys whose optimal path
    /// requires chaining direct walks within a round. If unsure,
    /// don't call this — the Dijkstra fallback is always sound.
    ///
    /// [`GtfsTimetable::with_walking_footpaths`] resets this flag,
    /// because coordinate-derived edges are not closed by construction.
    pub fn assert_footpaths_closed(mut self) -> Self {
        self.transfers_closed = true;
        self
    }

    /// Returns the child platforms of a parent station as a slice ready
    /// to pass to [`Timetable::raptor`] as origins or targets.
    ///
    /// Each entry is `(platform_stop_idx, walk_time)` where `walk_time`
    /// defaults to 0 (the user is willing to use any platform without
    /// further wait). Returns an empty slice if `parent_id` is not the
    /// GTFS id of a parent station, or if the station has no children.
    pub fn station_stops(&self, parent_id: &str) -> &[(StopIdx, Tau)] {
        self.station_children
            .get(parent_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Augments the footpath graph with bidirectional walking edges
    /// between every pair of stops within `max_distance_m` straight-line
    /// distance, computed via an equirectangular projection anchored at
    /// the feed's mean latitude (accurate to ~0.5% at city scale).
    ///
    /// Walk time per edge = `distance_m / walking_speed_m_per_s`,
    /// rounded up. Existing transfers from `transfers.txt` are preserved
    /// — coordinate-derived edges are only added where no explicit
    /// transfer between the pair already exists.
    ///
    /// The algorithm chains walks within a round (footpath relaxation
    /// runs to a fixed point), so the graph does not need to be
    /// transitively closed; pairs beyond `max_distance_m` that are
    /// reachable via a chain of shorter walks are still found.
    ///
    /// Typical values: `max_distance_m = 500` (covers same-block
    /// interchanges), `walking_speed_m_per_s = 1.4` (≈ 5 km/h, the
    /// standard pedestrian rate). Returns `self` so the call chains
    /// after [`GtfsTimetable::new`].
    pub fn with_walking_footpaths(
        mut self,
        gtfs: &'gtfs Gtfs,
        max_distance_m: f64,
        walking_speed_m_per_s: f64,
    ) -> Self {
        // Reference latitude: mean across stops with valid coords.
        let mut sum_lat = 0.0;
        let mut n_with_coords = 0usize;
        for stop in gtfs.stops.values() {
            if let (Some(lat), Some(_)) = (stop.latitude, stop.longitude) {
                sum_lat += lat;
                n_with_coords += 1;
            }
        }
        if n_with_coords == 0 {
            return self;
        }
        let mean_lat_rad = (sum_lat / n_with_coords as f64).to_radians();
        let lon_scale = mean_lat_rad.cos() * EARTH_RADIUS_M;
        let lat_scale = EARTH_RADIUS_M;

        // Project every stop into local Cartesian metres.
        let mut projected: Vec<ProjectedStop> = Vec::with_capacity(n_with_coords);
        for (stop_id, stop) in &gtfs.stops {
            if let (Some(lat), Some(lon)) = (stop.latitude, stop.longitude)
                && let Some(&idx) = self.stop_by_id.get(stop_id.as_str())
            {
                let x = lon.to_radians() * lon_scale;
                let y = lat.to_radians() * lat_scale;
                projected.push(ProjectedStop { pos: [x, y], idx });
            }
        }

        let tree = RTree::bulk_load(projected.clone());
        let r2 = max_distance_m * max_distance_m;

        for from in &projected {
            for near in tree.locate_within_distance(from.pos, r2) {
                if near.idx == from.idx {
                    continue;
                }
                // Skip if an explicit transfers.txt entry already covers
                // this directed pair — keep the publisher's value.
                if self.transfer_times.contains_key(&(from.idx, near.idx)) {
                    continue;
                }
                let dx = from.pos[0] - near.pos[0];
                let dy = from.pos[1] - near.pos[1];
                let dist_m = (dx * dx + dy * dy).sqrt();
                let walk_time = (dist_m / walking_speed_m_per_s).ceil() as Tau;

                self.footpaths_for_stops[from.idx.idx()].push(near.idx);
                self.transfer_times.insert((from.idx, near.idx), walk_time);
            }
        }

        // Coordinate-derived edges are direct only — closure is not
        // preserved. Drop any prior closure assertion.
        self.transfers_closed = false;
        self
    }

    /// Number of trips active on the timetable's service date (i.e. the
    /// trips that survived calendar filtering at construction).
    pub fn n_trips(&self) -> usize {
        self.trip_ids.len()
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

impl<'gtfs> Timetable for GtfsTimetable<'gtfs> {
    fn n_stops(&self) -> usize {
        self.stop_ids.len()
    }

    fn n_routes(&self) -> usize {
        self.route_ids.len()
    }

    fn get_routes_serving_stop(&self, stop: StopIdx) -> &[(RouteIdx, u32)] {
        self.routes_for_stop[stop.idx()].as_slice()
    }

    fn get_stops_after(&self, route: RouteIdx, pos: u32) -> &[StopIdx] {
        let stops = &self.stops_for_route[route.idx()];
        &stops[pos as usize..]
    }

    fn stop_at(&self, route: RouteIdx, pos: u32) -> StopIdx {
        self.stops_for_route[route.idx()][pos as usize]
    }

    fn get_earliest_trip(&self, route: RouteIdx, at: Tau, pos: u32) -> Option<TripIdx> {
        let trips = &self.trips_for_route[route.idx()];
        let dep_row = &self.departure_times[route.idx()][pos as usize];
        let idx = dep_row.partition_point(|&dep| dep < at);
        trips.get(idx).copied()
    }

    fn get_arrival_time(&self, trip: TripIdx, pos: u32) -> Tau {
        let (route_idx, trip_pos) = self.route_for_trip[trip.idx()];
        self.arrival_times[route_idx.idx()][pos as usize][trip_pos]
    }

    fn get_departure_time(&self, trip: TripIdx, pos: u32) -> Tau {
        let (route_idx, trip_pos) = self.route_for_trip[trip.idx()];
        self.departure_times[route_idx.idx()][pos as usize][trip_pos]
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

    fn footpaths_are_transitively_closed(&self) -> bool {
        self.transfers_closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtfs_structures::{Calendar, CalendarDate, StopTime};

    fn st(arr: u32, dep: u32) -> StopTime {
        StopTime {
            arrival_time: Some(arr),
            departure_time: Some(dep),
            ..Default::default()
        }
    }

    /// Build a minimal `Gtfs` with one calendar entry running Mon-Fri
    /// throughout 2026 for service "weekday".
    fn weekday_only_feed() -> Gtfs {
        let mut g = Gtfs::default();
        g.calendar.insert(
            "weekday".into(),
            Calendar {
                id: "weekday".into(),
                monday: true,
                tuesday: true,
                wednesday: true,
                thursday: true,
                friday: true,
                saturday: false,
                sunday: false,
                start_date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                end_date: NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            },
        );
        g
    }

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn calendar_active_on_weekday_inside_window() {
        let gtfs = weekday_only_feed();
        // 2026-05-04 is a Monday inside the window.
        assert!(is_service_active(&gtfs, "weekday", ymd(2026, 5, 4)));
    }

    #[test]
    fn calendar_inactive_on_weekend_inside_window() {
        let gtfs = weekday_only_feed();
        // 2026-05-02 is a Saturday — flag is false.
        assert!(!is_service_active(&gtfs, "weekday", ymd(2026, 5, 2)));
    }

    #[test]
    fn calendar_inactive_outside_date_window() {
        let gtfs = weekday_only_feed();
        // 2025-12-31 is a Wednesday but before start_date.
        assert!(!is_service_active(&gtfs, "weekday", ymd(2025, 12, 31)));
        // 2027-01-04 is a Monday but after end_date.
        assert!(!is_service_active(&gtfs, "weekday", ymd(2027, 1, 4)));
    }

    #[test]
    fn calendar_dates_added_overrides_calendar_inactive() {
        let mut gtfs = weekday_only_feed();
        // Add a Saturday exception.
        gtfs.calendar_dates.insert(
            "weekday".into(),
            vec![CalendarDate {
                service_id: "weekday".into(),
                date: ymd(2026, 5, 2),
                exception_type: Exception::Added,
            }],
        );
        assert!(is_service_active(&gtfs, "weekday", ymd(2026, 5, 2)));
    }

    #[test]
    fn calendar_dates_deleted_overrides_calendar_active() {
        let mut gtfs = weekday_only_feed();
        // Cancel the Monday 2026-05-04 service.
        gtfs.calendar_dates.insert(
            "weekday".into(),
            vec![CalendarDate {
                service_id: "weekday".into(),
                date: ymd(2026, 5, 4),
                exception_type: Exception::Deleted,
            }],
        );
        assert!(!is_service_active(&gtfs, "weekday", ymd(2026, 5, 4)));
    }

    #[test]
    fn unknown_service_id_is_inactive() {
        let gtfs = weekday_only_feed();
        assert!(!is_service_active(
            &gtfs,
            "no-such-service",
            ymd(2026, 5, 4)
        ));
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
