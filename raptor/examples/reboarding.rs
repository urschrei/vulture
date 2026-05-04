//! Example showing a multi-route RAPTOR query where a passenger reboards
//! a shared route at a later stop reached via a faster feeder route.

use raptor::{RouteIdx, StopIdx, Tau, Timetable, TripIdx};

// Stop indices: S=0, A=1, B=2, C=3, D=4
// Route indices: R1=0 (S->A), R2=1 (S->B), R3=2 (A->B->C->D)
// Trip indices: R1T1=0, R2T1=1, R3early=2, R3late=3
struct ReBoardingTimetable;

const S: StopIdx = StopIdx::new(0);
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

    fn get_routes_serving_stop(&self, stop: StopIdx) -> &[(RouteIdx, u32)] {
        // (route, earliest position of stop on that route)
        match stop.get() {
            0 => &[(R1, 0), (R2, 0)], // S
            1 => &[(R1, 1), (R3, 0)], // A
            2 => &[(R2, 1), (R3, 1)], // B
            3 => &[(R3, 2)],          // C
            4 => &[(R3, 3)],          // D
            _ => &[],
        }
    }

    fn get_stops_after(&self, route: RouteIdx, pos: u32) -> &[StopIdx] {
        const R1_STOPS: [StopIdx; 2] = [StopIdx::new(0), StopIdx::new(1)]; // S, A
        const R2_STOPS: [StopIdx; 2] = [StopIdx::new(0), StopIdx::new(2)]; // S, B
        const R3_STOPS: [StopIdx; 4] = [
            StopIdx::new(1),
            StopIdx::new(2),
            StopIdx::new(3),
            StopIdx::new(4),
        ]; // A, B, C, D
        let order: &[StopIdx] = match route.get() {
            0 => &R1_STOPS,
            1 => &R2_STOPS,
            2 => &R3_STOPS,
            _ => return &[],
        };
        &order[pos as usize..]
    }

    fn stop_at(&self, route: RouteIdx, pos: u32) -> StopIdx {
        let stops = self.get_stops_after(route, 0);
        stops[pos as usize]
    }

    fn get_earliest_trip(&self, route: RouteIdx, at: Tau, pos: u32) -> Option<TripIdx> {
        match route.get() {
            0 => {
                // R1: dep at pos 0 (S) = 0; pos 1 (A) is alight, irrelevant
                let dep = match pos {
                    0 => 0,
                    _ => return None,
                };
                (at <= dep).then_some(R1_T1)
            }
            1 => {
                // R2: dep at pos 0 (S) = 0
                let dep = match pos {
                    0 => 0,
                    _ => return None,
                };
                (at <= dep).then_some(R2_T1)
            }
            2 => {
                // R3: pos 0=A dep 25/105, pos 1=B dep 30/110, pos 2=C dep 40/120
                let (early_dep, late_dep) = match pos {
                    0 => (25, 105),
                    1 => (30, 110),
                    2 => (40, 120),
                    3 => (50, 130),
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

    fn get_arrival_time(&self, trip: TripIdx, pos: u32) -> Tau {
        match (trip.get(), pos) {
            (0, 1) => 100, // R1_T1 at A
            (1, 1) => 30,  // R2_T1 at B
            (3, 1) => 110,
            (3, 2) => 120,
            (3, 3) => 130, // R3_LATE
            (2, 1) => 30,
            (2, 2) => 40,
            (2, 3) => 50, // R3_EARLY
            _ => Tau::MAX,
        }
    }

    fn get_departure_time(&self, trip: TripIdx, pos: u32) -> Tau {
        match (trip.get(), pos) {
            (0, 0) => 0, // R1_T1 at S
            (1, 0) => 0, // R2_T1 at S
            (3, 0) => 105,
            (3, 1) => 110,
            (3, 2) => 120, // R3_LATE
            (2, 0) => 25,
            (2, 1) => 30,
            (2, 2) => 40, // R3_EARLY
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
    let journeys = timetable.raptor(3, 0, &[(S, 0)], &[(D, 0)]);

    println!("{journeys:#?}");
}
