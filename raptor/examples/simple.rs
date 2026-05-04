use raptor::{RouteIdx, StopIdx, Tau, Timetable, TripIdx};

// a single route with stops [0..10]
struct SingleRoute;

impl Timetable for SingleRoute {
    fn n_stops(&self) -> usize {
        10
    }
    fn n_routes(&self) -> usize {
        1
    }

    fn get_routes_serving_stop(&self, _stop: StopIdx) -> &[RouteIdx] {
        const ROUTES: [RouteIdx; 1] = [RouteIdx::new(0)];
        &ROUTES
    }

    fn get_earlier_stop(&self, _route: RouteIdx, left: StopIdx, right: StopIdx) -> StopIdx {
        if left.get() <= right.get() {
            left
        } else {
            right
        }
    }

    fn get_stops_after(&self, _route: RouteIdx, stop: StopIdx) -> &[StopIdx] {
        const STOPS: [StopIdx; 10] = [
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
        &STOPS[stop.get() as usize..]
    }

    fn get_arrival_time(&self, _trip: TripIdx, stop: StopIdx) -> Tau {
        (stop.get() as Tau) * 10
    }

    fn get_departure_time(&self, _trip: TripIdx, stop: StopIdx) -> Tau {
        (stop.get() as Tau) * 10 + 5
    }

    fn get_footpaths_from(&self, _stop: StopIdx) -> &[StopIdx] {
        &[]
    }

    fn get_earliest_trip(&self, _route: RouteIdx, at: Tau, stop: StopIdx) -> Option<TripIdx> {
        (at < self.get_departure_time(TripIdx::new(0), stop)).then_some(TripIdx::new(0))
    }
}

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::new().filter_or("RAPTOR_EXAMPLE_LOG_LEVEL", "info"),
    )
    .init();

    let mock = SingleRoute;
    let journey = mock.raptor(10, 0, StopIdx::new(0), StopIdx::new(9));

    println!("{journey:#?}");
}
