use raptor::{RouteIdx, StopIdx, Tau, Timetable, TripIdx};

const R0_STOPS: [StopIdx; 10] = [
    StopIdx::new(0),
    StopIdx::new(1),
    StopIdx::new(2),
    StopIdx::new(3),
    StopIdx::new(4),
    StopIdx::new(5),
    StopIdx::new(6),
    StopIdx::new(7),
    StopIdx::new(8),
    StopIdx::new(9),
];

const R1_STOPS: [StopIdx; 4] = [
    StopIdx::new(2),
    StopIdx::new(10),
    StopIdx::new(11),
    StopIdx::new(9),
];

struct TwoRoutes;

impl TwoRoutes {
    fn pos_on_r0(stop: StopIdx) -> Option<u32> {
        R0_STOPS.iter().position(|&s| s == stop).map(|p| p as u32)
    }
    fn pos_on_r1(stop: StopIdx) -> Option<u32> {
        R1_STOPS.iter().position(|&s| s == stop).map(|p| p as u32)
    }
}

impl Timetable for TwoRoutes {
    fn n_stops(&self) -> usize {
        12
    }
    fn n_routes(&self) -> usize {
        2
    }

    fn get_routes_serving_stop(&self, stop: StopIdx) -> &[(RouteIdx, u32)] {
        const R0: RouteIdx = RouteIdx::new(0);
        const R1: RouteIdx = RouteIdx::new(1);
        // Stop 2 is on both routes (R0 pos 2, R1 pos 0); stop 9 is on
        // both (R0 pos 9, R1 pos 3). Stops 10, 11 are R1-only; the rest
        // are R0-only.
        match stop.get() {
            2 => &[(R0, 2), (R1, 0)],
            9 => &[(R0, 9), (R1, 3)],
            10 => &[(R1, 1)],
            11 => &[(R1, 2)],
            n if n < 10 => match n {
                0 => &[(R0, 0)],
                1 => &[(R0, 1)],
                3 => &[(R0, 3)],
                4 => &[(R0, 4)],
                5 => &[(R0, 5)],
                6 => &[(R0, 6)],
                7 => &[(R0, 7)],
                8 => &[(R0, 8)],
                _ => &[],
            },
            _ => &[],
        }
    }

    fn get_stops_after(&self, route: RouteIdx, pos: u32) -> &[StopIdx] {
        if route.get() == 0 {
            &R0_STOPS[pos as usize..]
        } else {
            &R1_STOPS[pos as usize..]
        }
    }

    fn stop_at(&self, route: RouteIdx, pos: u32) -> StopIdx {
        if route.get() == 0 {
            R0_STOPS[pos as usize]
        } else {
            R1_STOPS[pos as usize]
        }
    }

    fn get_earliest_trip(&self, route: RouteIdx, at: Tau, pos: u32) -> Option<TripIdx> {
        let trip = TripIdx::new(route.get());
        (at < self.get_departure_time(trip, pos)).then_some(trip)
    }

    fn get_arrival_time(&self, trip: TripIdx, pos: u32) -> Tau {
        if trip.get() == 0 {
            // R0 trip: arrival at pos n is the stop_id (also n) * 10
            R0_STOPS[pos as usize].get() as Tau * 10
        } else {
            // R1 trip: arrival at position p is (p + 2) * 10
            (pos as Tau + 2) * 10
        }
    }

    fn get_departure_time(&self, trip: TripIdx, pos: u32) -> Tau {
        self.get_arrival_time(trip, pos) + 5
    }

    fn get_footpaths_from(&self, stop: StopIdx) -> &[StopIdx] {
        if stop.get() == 2 {
            const SELF: [StopIdx; 1] = [StopIdx::new(2)];
            &SELF
        } else {
            &[]
        }
    }
}

#[allow(dead_code)]
fn _suppress_unused() {
    let _ = TwoRoutes::pos_on_r0(StopIdx::new(0));
    let _ = TwoRoutes::pos_on_r1(StopIdx::new(0));
}

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::new().filter_or("RAPTOR_EXAMPLE_LOG_LEVEL", "info"),
    )
    .init();

    let mock = TwoRoutes;
    let journey = mock.raptor(10, 0, StopIdx::new(1), StopIdx::new(9));

    println!("{journey:#?}");
}
