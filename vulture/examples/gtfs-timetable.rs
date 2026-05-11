//! Load a real GTFS feed and run a single-departure query against it.
//!
//! Usage:
//!
//! ```text
//! cargo run --release --example gtfs-timetable -- <feed.zip> <YYYY-MM-DD> <start_stop_id> <target_stop_id>
//! ```
//!
//! The bundled Delhi Metro feed at `aux/dmrc_gtfs.zip` makes a good smoke test:
//!
//! ```text
//! cargo run --release --example gtfs-timetable -- aux/dmrc_gtfs.zip 2024-01-15 1 44
//! ```
//!
//! Stops and routes are looked up by their GTFS IDs and printed using their
//! human-readable names when those are available.

use std::env;
use std::process::ExitCode;
use std::time::Duration as StdDuration;

use anyhow::{Context, Result, anyhow, bail};
use gtfs_structures::Gtfs;
use humantime::format_duration;
use jiff::civil::Date;
use vulture::{Duration, Journey, SecondOfDay, Timetable, gtfs::GtfsTimetable};

const DEPARTURE: SecondOfDay = SecondOfDay::hms(19, 15, 0);

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut args = env::args();
    let prog = args.next().unwrap_or_else(|| "gtfs-timetable".into());
    let (zip, date, start, target) = match (args.next(), args.next(), args.next(), args.next()) {
        (Some(z), Some(d), Some(s), Some(t)) if args.next().is_none() => (z, d, s, t),
        _ => {
            bail!("usage: {prog} <feed.zip> <YYYY-MM-DD> <start_stop_id> <target_stop_id>");
        }
    };
    let service_date: Date = date.parse().context("parsing service date")?;

    let gtfs = Gtfs::new(&zip).with_context(|| format!("loading {zip}"))?;
    let timetable = GtfsTimetable::new(&gtfs, service_date)
        .with_context(|| format!("building timetable for {service_date}"))?;

    let start_idx = timetable
        .stop_idx(&start)
        .ok_or_else(|| anyhow!("unknown start stop: {start}"))?;
    let target_idx = timetable
        .stop_idx(&target)
        .ok_or_else(|| anyhow!("unknown target stop: {target}"))?;

    let journeys = timetable
        .query()
        .from(&[(start_idx, Duration::ZERO)])
        .to(&[(target_idx, Duration::ZERO)])
        .max_transfers(10)
        .depart_at(DEPARTURE)
        .run();

    if journeys.is_empty() {
        println!("No journeys found from {start} to {target} on {service_date}.");
        return Ok(());
    }

    for (i, journey) in journeys.iter().enumerate() {
        let travel = StdDuration::from_secs((journey.arrival() - DEPARTURE).0.into());
        println!("Journey {} ({}):", i + 1, format_duration(travel));
        print_journey(&gtfs, &timetable, journey, &start);
        println!();
    }

    Ok(())
}

fn print_journey(gtfs: &Gtfs, tt: &GtfsTimetable, journey: &Journey, start_id: &str) {
    print!("  \"{}\"", stop_name(gtfs, start_id));
    for (route_idx, stop_idx) in &journey.plan {
        let route_id = tt.route_id(*route_idx);
        let stop_id = tt.stop_id(*stop_idx);
        print!(
            " --[{}]-> \"{}\"",
            route_name(gtfs, route_id),
            stop_name(gtfs, stop_id),
        );
    }
    println!();
}

fn stop_name<'a>(gtfs: &'a Gtfs, id: &'a str) -> &'a str {
    gtfs.stops
        .get(id)
        .and_then(|s| s.name.as_deref())
        .unwrap_or(id)
}

fn route_name<'a>(gtfs: &'a Gtfs, id: &'a str) -> &'a str {
    gtfs.routes
        .get(id)
        .and_then(|r| r.short_name.as_deref().or(r.long_name.as_deref()))
        .unwrap_or(id)
}
