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

impl Timetable for TwoRoutes {
    fn n_stops(&self) -> usize {
        12
    }
    fn n_routes(&self) -> usize {
        2
    }

    fn get_routes_serving_stop(&self, stop: StopIdx) -> &[RouteIdx] {
        const R0: RouteIdx = RouteIdx::new(0);
        const R1: RouteIdx = RouteIdx::new(1);
        const BOTH: [RouteIdx; 2] = [R0, R1];
        const ONLY_R0: [RouteIdx; 1] = [R0];
        const ONLY_R1: [RouteIdx; 1] = [R1];
        let in_r0 = R0_STOPS.contains(&stop);
        let in_r1 = R1_STOPS.contains(&stop);
        match (in_r0, in_r1) {
            (true, true) => &BOTH,
            (true, false) => &ONLY_R0,
            (false, true) => &ONLY_R1,
            (false, false) => &[],
        }
    }

    fn get_earlier_stop(&self, route: RouteIdx, left: StopIdx, right: StopIdx) -> StopIdx {
        if route.get() == 0 {
            if left.get() <= right.get() {
                left
            } else {
                right
            }
        } else {
            let l = R1_STOPS.iter().position(|&a| a == left).unwrap();
            let r = R1_STOPS.iter().position(|&a| a == right).unwrap();
            R1_STOPS[l.min(r)]
        }
    }

    fn get_stops_after(&self, route: RouteIdx, stop: StopIdx) -> &[StopIdx] {
        if route.get() == 0 {
            &R0_STOPS[stop.get() as usize..]
        } else {
            let pos = R1_STOPS.iter().position(|&a| a == stop).unwrap();
            &R1_STOPS[pos..]
        }
    }

    fn get_earliest_trip(&self, route: RouteIdx, at: Tau, stop: StopIdx) -> Option<TripIdx> {
        if route.get() == 0 {
            (at < self.get_departure_time(TripIdx::new(0), stop)).then_some(TripIdx::new(0))
        } else {
            (at < self.get_departure_time(TripIdx::new(1), stop)).then_some(TripIdx::new(1))
        }
    }

    fn get_arrival_time(&self, trip: TripIdx, stop: StopIdx) -> Tau {
        if trip.get() == 0 {
            (stop.get() as Tau) * 10
        } else {
            let pos = R1_STOPS.iter().position(|&a| a == stop).unwrap();
            (pos + 2) * 10
        }
    }

    fn get_departure_time(&self, trip: TripIdx, stop: StopIdx) -> Tau {
        self.get_arrival_time(trip, stop) + 5
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

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::new().filter_or("RAPTOR_EXAMPLE_LOG_LEVEL", "info"),
    )
    .init();

    let mock = TwoRoutes;
    let journey = mock.raptor(10, 0, StopIdx::new(1), StopIdx::new(9));

    println!("{journey:#?}");
}
