// Usage: cargo run --example gtfs-timetable <path_to_zip> <YYYY-MM-DD> <start_stop> <target_stop>

use gtfs_structures::Gtfs;
use humantime::format_duration;
use jiff::civil::Date;
use raptor::{Journey, Timetable, gtfs::GtfsTimetable};
use std::{env, time::Duration};

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::new().filter_or("RAPTOR_EXAMPLE_LOG_LEVEL", "info"),
    )
    .init();

    let args: Vec<String> = env::args().collect();
    if args.len() != 5 {
        eprintln!(
            "Usage: {} <path_to_zip> <YYYY-MM-DD> <start_stop> <target_stop>",
            args[0]
        );
        std::process::exit(1);
    }

    let path = &args[1];
    let service_date: Date = args[2].parse()?;
    let start = args[3].as_str();
    let target = args[4].as_str();

    let gtfs = Gtfs::new(path)?;
    let timetable = GtfsTimetable::new(&gtfs, service_date)?;

    let start_idx = timetable
        .stop_idx(start)
        .ok_or_else(|| anyhow::anyhow!("unknown start stop: {start}"))?;
    let target_idx = timetable
        .stop_idx(target)
        .ok_or_else(|| anyhow::anyhow!("unknown target stop: {target}"))?;

    let departure_time = 19 * 3600 + 15 * 60;
    let journeys = timetable.raptor(10, departure_time, &[(start_idx, 0)], &[(target_idx, 0)]);

    if journeys.is_empty() {
        println!("No journeys found.");
        return Ok(());
    }

    for (i, journey) in journeys.iter().enumerate() {
        let travel_time = Duration::from_secs((journey.arrival() - departure_time) as u64);
        println!("Journey {} ({}):", i + 1, format_duration(travel_time));
        print_journey(&gtfs, &timetable, journey, start);
        println!();
    }

    log::debug!("{journeys:#?}");

    Ok(())
}

fn print_journey<'gtfs>(
    gtfs: &'gtfs Gtfs,
    timetable: &GtfsTimetable<'gtfs>,
    journey: &Journey,
    start: &'gtfs str,
) {
    let start_name = gtfs
        .stops
        .get(start)
        .and_then(|s| s.name.as_deref())
        .unwrap_or(start);
    print!("\"{}\" ", start_name);

    for (route, stop) in journey.plan.iter() {
        let route_id = timetable.route_id(*route);
        let route_name = gtfs
            .routes
            .get(route_id)
            .and_then(|r| r.short_name.as_deref().or(r.long_name.as_deref()))
            .unwrap_or(route_id);

        let raw_stop_id = timetable.stop_id(*stop);
        let stop_name = gtfs
            .stops
            .get(raw_stop_id)
            .and_then(|s| s.name.as_deref())
            .unwrap_or(raw_stop_id);

        print!("-[\"{}\"]-> \"{}\" ", route_name, stop_name);
    }
}
