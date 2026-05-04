//! Example showing a multi-route RAPTOR query where a passenger reboards
//! a shared route at a later stop reached via a faster feeder route.

use raptor::{RouteIdx, StopIdx, Tau, Timetable, TripIdx};

// Stop indices: S=0, A=1, B=2, C=3, D=4
// Route indices: R1=0 (S->A), R2=1 (S->B), R3=2 (A->B->C->D)
// Trip indices: R1T1=0, R2T1=1, R3early=2, R3late=3
struct ReBoardingTimetable;

const S: StopIdx = StopIdx::new(0);
const A: StopIdx = StopIdx::new(1);
const B: StopIdx = StopIdx::new(2);
const C: StopIdx = StopIdx::new(3);
const D: StopIdx = StopIdx::new(4);

const R1: RouteIdx = RouteIdx::new(0);
const R2: RouteIdx = RouteIdx::new(1);
const R3: RouteIdx = RouteIdx::new(2);

const R1_T1: TripIdx = TripIdx::new(0);
const R2_T1: TripIdx = TripIdx::new(1);
const R3_EARLY: TripIdx = TripIdx::new(2);
const R3_LATE: TripIdx = TripIdx::new(3);

impl Timetable for ReBoardingTimetable {
    fn n_stops(&self) -> usize {
        5
    }
    fn n_routes(&self) -> usize {
        3
    }

    fn get_routes_serving_stop(&self, stop: StopIdx) -> &[RouteIdx] {
        match stop.get() {
            0 => &[R1, R2], // S
            1 => &[R1, R3], // A
            2 => &[R2, R3], // B
            3 | 4 => &[R3], // C, D
            _ => &[],
        }
    }

    fn get_earlier_stop(&self, route: RouteIdx, left: StopIdx, right: StopIdx) -> StopIdx {
        let order: &[StopIdx] = match route.get() {
            0 => &[S, A],
            1 => &[S, B],
            2 => &[A, B, C, D],
            _ => return left,
        };
        let l = order.iter().position(|&c| c == left).unwrap_or(99);
        let r = order.iter().position(|&c| c == right).unwrap_or(99);
        order[l.min(r)]
    }

    fn get_stops_after(&self, route: RouteIdx, stop: StopIdx) -> &[StopIdx] {
        let order: &[StopIdx] = match route.get() {
            0 => &[S, A],
            1 => &[S, B],
            2 => &[A, B, C, D],
            _ => return &[],
        };
        let pos = order.iter().position(|&c| c == stop).unwrap_or(0);
        &order[pos..]
    }

    fn get_earliest_trip(&self, route: RouteIdx, at: Tau, stop: StopIdx) -> Option<TripIdx> {
        match route.get() {
            0 => {
                let dep = match stop.get() {
                    0 => 0,
                    1 => 100,
                    _ => return None,
                };
                (at <= dep).then_some(R1_T1)
            }
            1 => {
                let dep = match stop.get() {
                    0 => 0,
                    2 => 30,
                    _ => return None,
                };
                (at <= dep).then_some(R2_T1)
            }
            2 => {
                let (early_dep, late_dep) = match stop.get() {
                    1 => (25, 105),
                    2 => (30, 110),
                    3 => (40, 120),
                    4 => (50, 130),
                    _ => return None,
                };
                if at <= early_dep {
                    Some(R3_EARLY)
                } else if at <= late_dep {
                    Some(R3_LATE)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn get_arrival_time(&self, trip: TripIdx, stop: StopIdx) -> Tau {
        match (trip.get(), stop.get()) {
            (0, 1) => 100, // R1_T1 at A
            (1, 2) => 30,  // R2_T1 at B
            (3, 2) => 110,
            (3, 3) => 120,
            (3, 4) => 130, // R3_LATE
            (2, 2) => 30,
            (2, 3) => 40,
            (2, 4) => 50, // R3_EARLY
            _ => Tau::MAX,
        }
    }

    fn get_departure_time(&self, trip: TripIdx, stop: StopIdx) -> Tau {
        match (trip.get(), stop.get()) {
            (0, 0) => 0, // R1_T1 at S
            (1, 0) => 0, // R2_T1 at S
            (3, 1) => 105,
            (3, 2) => 110,
            (3, 3) => 120, // R3_LATE
            (2, 1) => 25,
            (2, 2) => 30,
            (2, 3) => 40, // R3_EARLY
            _ => Tau::MAX,
        }
    }

    fn get_footpaths_from(&self, _: StopIdx) -> &[StopIdx] {
        &[]
    }
}

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::new().filter_or("RAPTOR_EXAMPLE_LOG_LEVEL", "info"),
    )
    .init();

    println!("Query: S -> D, departure time 0");
    println!("Expected: S --(R2)--> B --(R3/early)--> D, arrives @ t=50\n");

    let timetable = ReBoardingTimetable;
    let journeys = timetable.raptor(3, 0, S, D);

    println!("{journeys:#?}");
}
