//! WebAssembly bindings for [vulture](https://crates.io/crates/vulture).
//!
//! Designed for in-browser use: JS supplies a GTFS zip as a
//! `Uint8Array` (typically fetched via `fetch()` on the JS side),
//! the WASM module parses it via [`gtfs_structures::Gtfs::from_reader`],
//! constructs a [`vulture::gtfs::GtfsTimetable`], and exposes a small
//! set of concrete-typed query functions returning JS-friendly
//! journey objects with the GTFS display data (stop names, route
//! names, coordinates, agency) attached.

use std::io::Cursor;

use gtfs_structures::Gtfs;
use jiff::civil::Date;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use vulture::ffi;
use vulture::gtfs::GtfsTimetable as VTimetable;
use vulture::{Duration, RouteIdx, SecondOfDay, StopIdx, Timetable};

/// Opaque handle to a parsed GTFS feed + timetable. The JS side
/// constructs one from a `Uint8Array` of the GTFS zip and passes it
/// into query functions by reference. The struct holds both the
/// parsed [`Gtfs`] (used for display-data lookups: stop names,
/// route names, coordinates) and the algorithm-side [`VTimetable`].
#[wasm_bindgen]
pub struct VultureTimetable {
    gtfs: Gtfs,
    /// Always `Some` outside `with_walking_footpaths`, which takes
    /// the inner by value to apply the augmentation and re-stores
    /// it.
    inner: Option<VTimetable>,
    base_date: Date,
}

#[wasm_bindgen]
impl VultureTimetable {
    /// Parse a GTFS zip (as a `Uint8Array`) and pin the timetable to
    /// `service_date` (ISO `YYYY-MM-DD`). Returns a handle the query
    /// functions take by reference.
    #[wasm_bindgen(constructor)]
    pub fn new(zip_bytes: &[u8], service_date: &str) -> Result<VultureTimetable, JsError> {
        let gtfs = Gtfs::from_reader(Cursor::new(zip_bytes.to_vec()))
            .map_err(|e| JsError::new(&format!("GTFS parse failed: {e}")))?;
        let date: Date = service_date
            .parse()
            .map_err(|e| JsError::new(&format!("Invalid service_date (want YYYY-MM-DD): {e}")))?;
        let inner =
            VTimetable::new(&gtfs, date).map_err(|e| JsError::new(&format!("build: {e}")))?;
        Ok(Self {
            gtfs,
            inner: Some(inner),
            base_date: date,
        })
    }

    /// Number of stops in the loaded timetable.
    #[wasm_bindgen(js_name = nStops)]
    pub fn n_stops(&self) -> usize {
        self.tt().n_stops()
    }

    /// Number of synthetic RAPTOR routes (one or more per GTFS
    /// `route_id`).
    #[wasm_bindgen(js_name = nRoutes)]
    pub fn n_routes(&self) -> usize {
        self.tt().n_routes()
    }

    /// Resolve a GTFS `stop_id` to an integer index, or `undefined`
    /// if not in the timetable. JS callers feed the integer back into
    /// `runArrival` / `runRange`.
    #[wasm_bindgen(js_name = stopIdx)]
    pub fn stop_idx(&self, id: &str) -> Option<u32> {
        self.tt().stop_idx(id).map(|s| s.get())
    }

    /// Returns the GTFS `stop_name` for the given index, or
    /// `undefined` if the stop has no name field set.
    #[wasm_bindgen(js_name = stopName)]
    pub fn stop_name(&self, idx: u32) -> Option<String> {
        let id = self.tt().stop_id(StopIdx::new(idx));
        self.gtfs.stops.get(id).and_then(|s| s.name.clone())
    }

    /// Returns the GTFS `[lat, lon]` for the given stop index, or
    /// `undefined` if the stop has no coordinates set.
    #[wasm_bindgen(js_name = stopCoords)]
    pub fn stop_coords(&self, idx: u32) -> Option<Box<[f64]>> {
        let id = self.tt().stop_id(StopIdx::new(idx));
        let stop = self.gtfs.stops.get(id)?;
        let lat = stop.latitude?;
        let lon = stop.longitude?;
        Some(Box::new([lat, lon]))
    }

    /// Returns the GTFS `route_long_name` (falling back to
    /// `route_short_name` when the long name is empty) for the given
    /// synthetic route index. Several `RouteIdx`s can share a name —
    /// vulture splits one GTFS `route_id` into one or more synthetic
    /// routes by stop-pattern.
    #[wasm_bindgen(js_name = routeName)]
    pub fn route_name(&self, idx: u32) -> Option<String> {
        let id = self.tt().route_id(RouteIdx::new(idx));
        let r = self.gtfs.routes.get(id)?;
        // gtfs-structures keeps these as String — empty when absent —
        // so prefer long_name, fall back to short_name, then None.
        let long = r.long_name.as_deref().filter(|s| !s.is_empty());
        let short = r.short_name.as_deref().filter(|s| !s.is_empty());
        long.or(short).map(|s| s.to_owned())
    }

    /// Returns the agency name for the given route, or `undefined`
    /// if the route has no agency set or the agency record is
    /// missing.
    #[wasm_bindgen(js_name = routeAgency)]
    pub fn route_agency(&self, idx: u32) -> Option<String> {
        let id = self.tt().route_id(RouteIdx::new(idx));
        let r = self.gtfs.routes.get(id)?;
        let agency_id = r.agency_id.as_deref()?;
        self.gtfs
            .agencies
            .iter()
            .find(|a| a.id.as_deref() == Some(agency_id))
            .map(|a| a.name.clone())
    }

    /// Returns the entire stop catalogue as a JS array of
    /// `{idx, id, name, lat, lon}` objects, suitable for populating
    /// a dropdown. Stops without coordinates have `lat`/`lon` set to
    /// `null`.
    #[wasm_bindgen(js_name = allStops)]
    pub fn all_stops(&self) -> Result<JsValue, JsError> {
        let n = self.tt().n_stops();
        let mut out: Vec<StopRow> = Vec::with_capacity(n);
        for i in 0..n as u32 {
            let id = self.tt().stop_id(StopIdx::new(i)).to_owned();
            let stop = self.gtfs.stops.get(&id);
            let name = stop.and_then(|s| s.name.clone()).unwrap_or_default();
            let lat = stop.and_then(|s| s.latitude);
            let lon = stop.and_then(|s| s.longitude);
            out.push(StopRow {
                idx: i,
                id,
                name,
                lat,
                lon,
            });
        }
        serde_wasm_bindgen::to_value(&out).map_err(|e| JsError::new(&format!("{e}")))
    }

    /// Returns the entire synthetic-route catalogue as a JS array of
    /// `{idx, id, name, agency, route_type, route_color}` objects.
    #[wasm_bindgen(js_name = allRoutes)]
    pub fn all_routes(&self) -> Result<JsValue, JsError> {
        let n = self.tt().n_routes();
        let mut out: Vec<RouteRow> = Vec::with_capacity(n);
        for i in 0..n as u32 {
            let id = self.tt().route_id(RouteIdx::new(i)).to_owned();
            let route = self.gtfs.routes.get(&id);
            let route_type = route.map(|r| route_type_int(r.route_type)).unwrap_or(3);
            let route_color = route
                .and_then(|r| r.color)
                .map(|c| format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b));
            out.push(RouteRow {
                idx: i,
                id,
                name: self.route_name(i).unwrap_or_default(),
                agency: self.route_agency(i),
                route_type,
                route_color,
            });
        }
        serde_wasm_bindgen::to_value(&out).map_err(|e| JsError::new(&format!("{e}")))
    }

    /// Augment the timetable's footpath graph with bidirectional
    /// walking edges between every pair of stops within
    /// `max_distance_m` straight-line distance, using
    /// `walking_speed_m_per_s` (1.4 m/s ≈ standard pedestrian rate).
    /// Mutates the timetable in place; subsequent queries see the
    /// added footpaths.
    #[wasm_bindgen(js_name = withWalkingFootpaths)]
    pub fn with_walking_footpaths(&mut self, max_distance_m: f64, walking_speed_m_per_s: f64) {
        let inner = self
            .inner
            .take()
            .expect("inner present outside this method");
        self.inner =
            Some(inner.with_walking_footpaths(&self.gtfs, max_distance_m, walking_speed_m_per_s));
    }

    /// Reset the timetable to its base-date single-day state,
    /// discarding any walking-footpath augmentation. Useful for
    /// "before/after" demos.
    #[wasm_bindgen(js_name = resetFootpaths)]
    pub fn reset_footpaths(&mut self) -> Result<(), JsError> {
        let new_inner = VTimetable::new(&self.gtfs, self.base_date)
            .map_err(|e| JsError::new(&format!("rebuild: {e}")))?;
        self.inner = Some(new_inner);
        Ok(())
    }

    fn tt(&self) -> &VTimetable {
        self.inner.as_ref().expect("inner present")
    }
}

#[derive(Serialize)]
struct StopRow {
    idx: u32,
    id: String,
    name: String,
    lat: Option<f64>,
    lon: Option<f64>,
}

#[derive(Serialize)]
struct RouteRow {
    idx: u32,
    id: String,
    name: String,
    agency: Option<String>,
    /// GTFS `route_type` integer (0=tram, 1=metro, 2=rail, 3=bus,
    /// 4=ferry, 5=cable, 6=aerial, 7=funicular, …).
    route_type: i16,
    /// Hex colour `"#rrggbb"` from `routes.route_color`, or `None`
    /// if unset.
    route_color: Option<String>,
}

/// One transit leg with reconstructed timing (route, trip, boarding
/// and alighting stops + times). Mirrors [`vulture::TimedLeg`].
#[derive(Serialize)]
pub struct WasmTimedLeg {
    pub route: u32,
    pub trip: u32,
    /// GTFS `route_id` (post-vulture-split synthetic route's parent
    /// id), useful for display.
    pub route_id: String,
    /// Stop where the rider boards.
    pub board_stop: u32,
    /// Stop where the rider alights.
    pub alight_stop: u32,
    /// Departure time at `board_stop`, seconds since midnight.
    pub depart: u32,
    /// Arrival time at `alight_stop`, seconds since midnight.
    pub arrive: u32,
    /// Polyline points `[lat, lon]` for this leg's shape segment.
    /// `None` when the feed has no `shapes.txt` for this trip.
    pub shape: Option<Vec<(f32, f32)>>,
}

/// JS-friendly representation of a [`vulture::Journey`] with timed
/// legs attached (every leg includes route/trip/depart/arrive).
#[derive(Serialize)]
pub struct WasmJourney {
    /// Effective arrival time at the journey's target, seconds since
    /// midnight (may exceed 86400 in multi-day timetables).
    pub arrival: u32,
    /// Origin stop index.
    pub origin: u32,
    /// Target stop index.
    pub target: u32,
    /// Per-leg timing reconstruction. May be empty if reconstruction
    /// failed for any reason — vulture emits a `Journey` even when
    /// `with_timing` returns an error, so the JS side gets the
    /// topology without timing in that pathological case.
    pub legs: Vec<WasmTimedLeg>,
}

/// JS-friendly representation of a [`vulture::RangeJourney`].
#[derive(Serialize)]
pub struct WasmRangeJourney {
    pub depart: u32,
    pub journey: WasmJourney,
}

/// Single-criterion arrival-time query. `origins` and `targets` are
/// `Uint32Array`s of stop indices (each entry gets walk-time zero).
/// `depart` is seconds since midnight.
#[wasm_bindgen(js_name = runArrival)]
pub fn run_arrival(
    tt: &VultureTimetable,
    origins: &[u32],
    targets: &[u32],
    max_transfers: u8,
    depart: u32,
    require_wheelchair: bool,
) -> Result<JsValue, JsError> {
    let origins_v: Vec<(StopIdx, Duration)> = origins
        .iter()
        .map(|&s| (StopIdx::new(s), Duration::ZERO))
        .collect();
    let targets_v: Vec<(StopIdx, Duration)> = targets
        .iter()
        .map(|&s| (StopIdx::new(s), Duration::ZERO))
        .collect();
    let inner = tt.tt();
    let journeys = ffi::run_arrival(
        inner,
        &origins_v,
        &targets_v,
        max_transfers,
        SecondOfDay(depart),
        require_wheelchair,
    );
    let out: Vec<WasmJourney> = journeys
        .iter()
        .map(|j| make_wasm_journey(tt, j, SecondOfDay(depart)))
        .collect();
    serde_wasm_bindgen::to_value(&out).map_err(|e| JsError::new(&format!("{e}")))
}

/// Range query: run RAPTOR across a window of departure times. JS
/// passes `departures` as a `Uint32Array` of seconds-since-midnight
/// values. Returns `[{depart, journey}]` Pareto-filtered on
/// `(later depart, fewer transfers, earlier arrival)`.
///
/// Currently single-source / single-target only — the demo's range
/// panel doesn't need multi-endpoint, and rRAPTOR's serial path
/// (the one we use here) requires `ArrivalTime`.
#[wasm_bindgen(js_name = runRange)]
pub fn run_range(
    tt: &VultureTimetable,
    origin: u32,
    target: u32,
    max_transfers: u8,
    departures: &[u32],
    require_wheelchair: bool,
) -> Result<JsValue, JsError> {
    let inner = tt.tt();
    let mut query = inner
        .query()
        .from(StopIdx::new(origin))
        .to(StopIdx::new(target))
        .max_transfers(max_transfers);
    if require_wheelchair {
        query = query.require_wheelchair_accessible();
    }
    let entries = query
        .depart_in_window(departures.iter().map(|&d| SecondOfDay(d)))
        .run();

    let out: Vec<WasmRangeJourney> = entries
        .iter()
        .map(|rj| WasmRangeJourney {
            depart: rj.depart.0,
            journey: make_wasm_journey(tt, &rj.journey, rj.depart),
        })
        .collect();
    serde_wasm_bindgen::to_value(&out).map_err(|e| JsError::new(&format!("{e}")))
}

fn make_wasm_journey(
    tt: &VultureTimetable,
    j: &vulture::Journey<vulture::ArrivalTime>,
    depart: SecondOfDay,
) -> WasmJourney {
    let inner = tt.tt();
    let timed = j
        .with_timing(inner, depart, Duration::ZERO)
        .unwrap_or_default();
    let legs: Vec<WasmTimedLeg> = timed
        .iter()
        .map(|leg| {
            let trip_id = inner.trip_id(leg.trip);
            let shape = inner.shape_for_leg(&tt.gtfs, trip_id, leg.board, leg.alight);
            WasmTimedLeg {
                route: leg.route.get(),
                trip: leg.trip.get(),
                route_id: inner.route_id(leg.route).to_owned(),
                board_stop: leg.board.get(),
                alight_stop: leg.alight.get(),
                depart: leg.depart.0,
                arrive: leg.arrive.0,
                shape,
            }
        })
        .collect();
    WasmJourney {
        arrival: j.label.0.0,
        origin: j.origin.get(),
        target: j.target.get(),
        legs,
    }
}

/// Map [`gtfs_structures::RouteType`] to its canonical GTFS integer.
/// Mirrors the Extended Route Types convention for entries beyond the
/// core 0–7 range (200=coach, 1100=air, 1500=taxi); `Other(i)` is
/// passed through unchanged.
fn route_type_int(rt: gtfs_structures::RouteType) -> i16 {
    use gtfs_structures::RouteType;
    match rt {
        RouteType::Tramway => 0,
        RouteType::Subway => 1,
        RouteType::Rail => 2,
        RouteType::Bus => 3,
        RouteType::Ferry => 4,
        RouteType::CableCar => 5,
        RouteType::Gondola => 6,
        RouteType::Funicular => 7,
        RouteType::Coach => 200,
        RouteType::Air => 1100,
        RouteType::Taxi => 1500,
        RouteType::Other(i) => i,
    }
}
