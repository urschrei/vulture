//! Hand-rolled in-memory [`Timetable`](crate::Timetable) for tests, fixtures,
//! and small synthetic networks.
//!
//! The bundled [`SimpleTimetable`](crate::manual::SimpleTimetable) builder lets you describe a network
//! with `.route(...)` / `.footpath(...)` / `.transfer_time(...)` calls
//! and arbitrary key types. Production callers
//! usually want the GTFS adapter at [`crate::gtfs::GtfsTimetable`] instead;
//! this module shines when you want a tiny well-known network for unit
//! tests, examples, or property-test generators.

/// Benchmark timetable builders.
#[cfg(feature = "internal")]
pub mod builders;

use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;

use crate::{Duration, RouteIdx, SecondOfDay, StopIdx, Timetable, TripIdx};

/// A generic in-memory [`Timetable`] backed by interning tables and dense `Vec`s.
///
/// Generic over key types `S` / `R` / `T` (each `Hash + Eq + Clone`) so tests
/// and benches can use ergonomic keys – enums, integers, strings – while the
/// internal storage is dense `u32` indices throughout. Build with `.route(...)`
/// and `.footpath(...)` / `.transfer_time(...)`; resolve external keys to
/// indices via [`SimpleTimetable::stop_idx_of`] / [`SimpleTimetable::route_idx_of`]
/// / [`SimpleTimetable::trip_idx_of`] when constructing queries.
pub struct SimpleTimetable<S = u32, R = u32, T = u32>
where
    S: Hash + Eq + Clone,
    R: Hash + Eq + Clone,
    T: Hash + Eq + Clone,
{
    stop_keys: Vec<S>,
    stop_by_key: HashMap<S, StopIdx>,
    route_keys: Vec<R>,
    route_by_key: HashMap<R, RouteIdx>,
    trip_keys: Vec<T>,
    trip_by_key: HashMap<T, TripIdx>,

    /// RouteIdx -> ordered stop sequence.
    routes: Vec<Vec<StopIdx>>,
    /// TripIdx -> (route, per-stop (arrival, departure) aligned with the route's stops).
    /// `None` slots are reserved-but-unassigned trips: the builder pre-grows
    /// the vector when a `TripIdx` lands past the current end, but never
    /// silently fills those holes with a placeholder route.
    #[allow(clippy::type_complexity)]
    trips: Vec<Option<(RouteIdx, Vec<(SecondOfDay, SecondOfDay)>)>>,
    /// StopIdx -> reachable stops via footpath.
    footpaths: Vec<Vec<StopIdx>>,
    /// (from, to) -> transfer time.
    transfer_times: HashMap<(StopIdx, StopIdx), Duration>,
    /// StopIdx -> routes serving the stop (computed incrementally).
    /// For each stop, the routes serving it paired with the *earliest*
    /// position of the stop on that route. Each route appears at most
    /// once per stop.
    routes_for_stop: Vec<Vec<(RouteIdx, u32)>>,
}

impl<S, R, T> SimpleTimetable<S, R, T>
where
    S: Hash + Eq + Clone,
    R: Hash + Eq + Clone,
    T: Hash + Eq + Clone,
{
    /// Creates an empty timetable.
    pub fn new() -> Self {
        Self {
            stop_keys: Vec::new(),
            stop_by_key: HashMap::new(),
            route_keys: Vec::new(),
            route_by_key: HashMap::new(),
            trip_keys: Vec::new(),
            trip_by_key: HashMap::new(),
            routes: Vec::new(),
            trips: Vec::new(),
            footpaths: Vec::new(),
            transfer_times: HashMap::new(),
            routes_for_stop: Vec::new(),
        }
    }

    fn intern_stop(&mut self, key: S) -> StopIdx {
        if let Some(&idx) = self.stop_by_key.get(&key) {
            return idx;
        }
        let idx = StopIdx::new(self.stop_keys.len() as u32);
        self.stop_keys.push(key.clone());
        self.stop_by_key.insert(key, idx);
        self.footpaths.push(Vec::new());
        self.routes_for_stop.push(Vec::new());
        idx
    }

    fn intern_route(&mut self, key: R) -> RouteIdx {
        if let Some(&idx) = self.route_by_key.get(&key) {
            return idx;
        }
        let idx = RouteIdx::new(self.route_keys.len() as u32);
        self.route_keys.push(key.clone());
        self.route_by_key.insert(key, idx);
        self.routes.push(Vec::new());
        idx
    }

    fn intern_trip(&mut self, key: T) -> TripIdx {
        if let Some(&idx) = self.trip_by_key.get(&key) {
            return idx;
        }
        let idx = TripIdx::new(self.trip_keys.len() as u32);
        self.trip_keys.push(key.clone());
        self.trip_by_key.insert(key, idx);
        idx
    }

    /// Adds a route with its stops and trips.
    pub fn route(mut self, id: R, stops: &[S], trips: &[(T, &[(SecondOfDay, SecondOfDay)])]) -> Self
    where
        R: Debug,
        T: Debug,
    {
        let route_idx = self.intern_route(id);
        let stop_idxs: Vec<StopIdx> = stops.iter().map(|s| self.intern_stop(s.clone())).collect();
        self.routes[route_idx.idx()] = stop_idxs.clone();

        // Update the routes_for_stop reverse index, recording the earliest
        // position of each stop on this route. Iterating left-to-right and
        // skipping if route_idx is already present means the FIRST visit
        // (smallest position) is what gets stored – required so loop routes
        // queue correctly in the algorithm.
        for (pos, &s) in stop_idxs.iter().enumerate() {
            let entry = &mut self.routes_for_stop[s.idx()];
            if !entry.iter().any(|(r, _)| *r == route_idx) {
                entry.push((route_idx, pos as u32));
            }
        }

        for (trip_id, times) in trips {
            assert_eq!(
                times.len(),
                stops.len(),
                "trip {trip_id:?} has {} times but route has {} stops",
                times.len(),
                stops.len()
            );
            let trip_idx = self.intern_trip(trip_id.clone());
            // trips Vec must be at least trip_idx.idx() + 1 long
            while self.trips.len() <= trip_idx.idx() {
                self.trips.push(None);
            }
            self.trips[trip_idx.idx()] = Some((route_idx, times.to_vec()));
        }
        self
    }

    /// Ensures `key` has a `StopIdx` assigned without otherwise modifying the
    /// timetable. Useful when callers need to look up a stop by key (via
    /// [`stop_idx_of`](Self::stop_idx_of)) for stops that aren't directly
    /// named by any route or footpath.
    pub fn register_stop(mut self, key: S) -> Self {
        let _ = self.intern_stop(key);
        self
    }

    /// Adds a footpath from one stop to another.
    pub fn footpath(mut self, from: S, to: S) -> Self {
        let from_idx = self.intern_stop(from);
        let to_idx = self.intern_stop(to);
        self.footpaths[from_idx.idx()].push(to_idx);
        self
    }

    /// Sets the transfer time for a footpath.
    pub fn transfer_time(mut self, from: S, to: S, time: Duration) -> Self {
        let from_idx = self.intern_stop(from);
        let to_idx = self.intern_stop(to);
        self.transfer_times.insert((from_idx, to_idx), time);
        self
    }

    /// Returns the index assigned to the given stop key. Panics if the
    /// key was never inserted via [`route`](Self::route),
    /// [`footpath`](Self::footpath), or [`transfer_time`](Self::transfer_time).
    pub fn stop_idx_of(&self, key: &S) -> StopIdx {
        *self
            .stop_by_key
            .get(key)
            .expect("stop key not registered with this SimpleTimetable")
    }

    /// Returns the index assigned to the given route key.
    pub fn route_idx_of(&self, key: &R) -> RouteIdx {
        *self
            .route_by_key
            .get(key)
            .expect("route key not registered with this SimpleTimetable")
    }

    /// Returns the index assigned to the given trip key.
    pub fn trip_idx_of(&self, key: &T) -> TripIdx {
        *self
            .trip_by_key
            .get(key)
            .expect("trip key not registered with this SimpleTimetable")
    }
}

impl<S, R, T> Default for SimpleTimetable<S, R, T>
where
    S: Hash + Eq + Clone,
    R: Hash + Eq + Clone,
    T: Hash + Eq + Clone,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<S, R, T> Timetable for SimpleTimetable<S, R, T>
where
    S: Hash + Eq + Clone + Debug,
    R: Hash + Eq + Clone + Debug,
    T: Hash + Eq + Clone + Debug,
{
    fn n_stops(&self) -> usize {
        self.stop_keys.len()
    }

    fn n_routes(&self) -> usize {
        self.route_keys.len()
    }

    fn get_routes_serving_stop(&self, stop: StopIdx) -> &[(RouteIdx, u32)] {
        self.routes_for_stop[stop.idx()].as_slice()
    }

    fn get_stops_after(&self, route: RouteIdx, pos: u32) -> &[StopIdx] {
        &self.routes[route.idx()][pos as usize..]
    }

    fn stop_at(&self, route: RouteIdx, pos: u32) -> StopIdx {
        self.routes[route.idx()][pos as usize]
    }

    fn get_earliest_trip(&self, route: RouteIdx, at: SecondOfDay, pos: u32) -> Option<TripIdx> {
        self.trips
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| slot.as_ref().map(|entry| (i, entry)))
            .filter(|(_, (r, _))| *r == route)
            .filter(|(_, (_, times))| times[pos as usize].1 >= at)
            .min_by_key(|(_, (_, times))| times[pos as usize].1)
            .map(|(trip_idx, _)| TripIdx::new(trip_idx as u32))
    }

    fn get_arrival_time(&self, trip: TripIdx, pos: u32) -> SecondOfDay {
        let (_route, times) = self.trips[trip.idx()].as_ref().expect("trip slot assigned");
        times[pos as usize].0
    }

    fn get_departure_time(&self, trip: TripIdx, pos: u32) -> SecondOfDay {
        let (_route, times) = self.trips[trip.idx()].as_ref().expect("trip slot assigned");
        times[pos as usize].1
    }

    fn get_footpaths_from(&self, stop: StopIdx) -> &[StopIdx] {
        self.footpaths[stop.idx()].as_slice()
    }

    fn get_transfer_time(&self, from: StopIdx, to: StopIdx) -> Duration {
        self.transfer_times
            .get(&(from, to))
            .copied()
            .unwrap_or(Duration(1))
    }
}

#[cfg(feature = "dotgraph")]
impl<S, R, T> SimpleTimetable<S, R, T>
where
    S: Hash + Eq + Clone + Debug + std::fmt::Display,
    R: Hash + Eq + Clone + Debug,
    T: Hash + Eq + Clone + Debug,
{
    /// Renders the timetable as a Graphviz DOT string.
    pub fn to_dot(&self, name: &str) -> Result<String, std::io::Error> {
        use dot_graph::{Edge, Graph, Kind, Node, Style};

        let mut graph = Graph::new(name, Kind::Digraph);

        for (idx, key) in self.stop_keys.iter().enumerate() {
            let _ = idx;
            graph.add_node(Node::new(&format!("s{key}")));
        }

        const COLORS: &[&str] = &[
            "red",
            "blue",
            "green",
            "orange",
            "purple",
            "brown",
            "deeppink",
            "darkgreen",
            "navy",
            "goldenrod",
        ];

        for (route_idx, route_stops) in self.routes.iter().enumerate() {
            let color = COLORS[route_idx % COLORS.len()];
            let route_idx_typed = RouteIdx::new(route_idx as u32);
            #[allow(clippy::type_complexity)]
            let route_trips: Vec<(
                usize,
                &(RouteIdx, Vec<(SecondOfDay, SecondOfDay)>),
            )> = self
                .trips
                .iter()
                .enumerate()
                .filter_map(|(i, slot)| slot.as_ref().map(|entry| (i, entry)))
                .filter(|(_, (r, _))| *r == route_idx_typed)
                .collect();

            for (i, window) in route_stops.windows(2).enumerate() {
                let from_key = &self.stop_keys[window[0].idx()];
                let to_key = &self.stop_keys[window[1].idx()];
                let route_key = &self.route_keys[route_idx];
                let mut label = format!("R{route_key:?}");
                for (trip_idx, (_, times)) in &route_trips {
                    let trip_key = &self.trip_keys[*trip_idx];
                    let dep = times[i].1;
                    let arr = times[i + 1].0;
                    label.push_str(&format!("\\nT{trip_key:?}: {dep}\u{2192}{arr}"));
                }
                graph.add_edge(
                    Edge::new(&format!("s{from_key}"), &format!("s{to_key}"), &label)
                        .color(Some(color)),
                );
            }
        }

        for (from_idx, targets) in self.footpaths.iter().enumerate() {
            let from_key = &self.stop_keys[from_idx];
            let from_typed = StopIdx::new(from_idx as u32);
            for &to in targets {
                let to_key = &self.stop_keys[to.idx()];
                let time = self
                    .transfer_times
                    .get(&(from_typed, to))
                    .copied()
                    .unwrap_or(Duration(1));
                graph.add_edge(
                    Edge::new(
                        &format!("s{from_key}"),
                        &format!("s{to_key}"),
                        &format!("t={time}"),
                    )
                    .style(Style::Dashed)
                    .color(Some("gray")),
                );
            }
        }

        graph.to_dot_string()
    }
}
