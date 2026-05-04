//! Check whether any Paris trip has non-monotonic arrival/departure times
//! across its stop_times. A "decrease" between consecutive stops is the
//! GTFS day-wrap signature: the trip crosses midnight but the publisher
//! encoded the post-midnight times as 00:xx instead of 24:xx.

use gtfs_structures::Gtfs;

const FEED: &str = "aux/external/paris.zip";

fn main() -> anyhow::Result<()> {
    let gtfs = Gtfs::new(FEED)?;
    let mut bad = 0usize;
    let mut shown = 0usize;
    for (trip_id, trip) in &gtfs.trips {
        let mut prev_dep: Option<u32> = None;
        for (i, st) in trip.stop_times.iter().enumerate() {
            let dep = st.departure_time.unwrap_or(0);
            if let Some(p) = prev_dep {
                if dep < p {
                    bad += 1;
                    if shown < 10 {
                        println!("trip {trip_id} stop {i}: dep_time={} (prev was {})", dep, p);
                        shown += 1;
                    }
                    break;
                }
            }
            prev_dep = Some(dep);
        }
    }
    println!(
        "\nbad (non-monotonic dep across stops) trips: {bad} / {}",
        gtfs.trips.len()
    );
    Ok(())
}
