//! Cross-city benchmark across multiple GTFS feeds.
//!
//! Looks for feeds in `aux/external/{helsinki,berlin,paris}.zip` and the
//! tracked `aux/dmrc_gtfs.zip`. For each feed found, loads it, runs three
//! representative queries, and reports load time, query latency
//! (median of 50 runs), and journey arrival time.
//!
//! Skips feeds that are not present without failing – fetch them with
//! `scripts/fetch-bench-feeds.sh` first if you want the full table.
//!
//! Usage:
//!     cargo run --release --example cross-city-bench

use gtfs_structures::Gtfs;
use jiff::civil::date;
use std::path::Path;
use std::time::Instant;
use vulture::gtfs::GtfsTimetable;
use vulture::{RaptorCache, SecondOfDay, Timetable};

const DEPARTURE_TIME: SecondOfDay = SecondOfDay(9 * 3600);
const QUERY_REPEATS: usize = 50;

struct FeedSpec {
    name: &'static str,
    path: &'static str,
    /// Service date used for calendar filtering. The bundled Delhi feed's
    /// calendar ends 2025-12-31, so it gets an older date; the recently
    /// fetched feeds use 2026-05-04 (a Monday).
    service_date: jiff::civil::Date,
    /// If `Some(distance_m)`, the timetable is augmented with
    /// coordinate-derived walking footpaths via
    /// [`GtfsTimetable::with_walking_footpaths`] using the standard
    /// 1.4 m/s walking speed.
    walking_footpaths_m: Option<f64>,
    queries: &'static [QuerySpec],
}

struct QuerySpec {
    label: &'static str,
    origin: Endpoint,
    target: Endpoint,
}

enum Endpoint {
    /// Single stop ID – interpreted as a child platform / standalone stop.
    Stop(&'static str),
    /// Parent station ID – expanded to all child platforms via
    /// `tt.station_stops(...)`.
    Station(&'static str),
}

// Service dates for each feed. Built once at startup; can't be in a
// const because jiff's date constructor isn't const.
fn feeds() -> Vec<FeedSpec> {
    vec![
        FeedSpec {
            name: "Delhi Metro",
            path: "aux/dmrc_gtfs.zip",
            service_date: date(2024, 1, 15), // a Monday in the Delhi calendar window
            walking_footpaths_m: None,
            queries: &[
                QuerySpec {
                    label: "Dilshad Garden -> Shahdara (1 trip)",
                    origin: Endpoint::Stop("1"),
                    target: Endpoint::Stop("4"),
                },
                QuerySpec {
                    label: "Dilshad Garden -> Vishwavidyalaya (2 trips)",
                    origin: Endpoint::Stop("1"),
                    target: Endpoint::Stop("44"),
                },
                QuerySpec {
                    label: "Paschim Vihar West -> Ghitorni (3 trips)",
                    origin: Endpoint::Stop("29"),
                    target: Endpoint::Stop("65"),
                },
            ],
        },
        FeedSpec {
            name: "Helsinki HSL",
            path: "aux/external/helsinki.zip",
            service_date: date(2026, 5, 4),
            walking_footpaths_m: Some(500.0),
            queries: &[
                QuerySpec {
                    label: "Kamppi metro -> Itäkeskus metro (direct, M1/M2 line)",
                    origin: Endpoint::Stop("1040601"),
                    target: Endpoint::Stop("1453601"),
                },
                QuerySpec {
                    label: "Rautatientori -> Pasila (station-to-station, ~3 km north)",
                    origin: Endpoint::Station("1000003"),
                    target: Endpoint::Station("1000202"),
                },
            ],
        },
        FeedSpec {
            name: "Berlin VBB",
            path: "aux/external/berlin.zip",
            service_date: date(2026, 5, 4),
            walking_footpaths_m: None,
            queries: &[
                QuerySpec {
                    // Hand-picked eastbound S-Bahn platform pair (Hbf 1:50 ->
                    // Alex 1:50). Trip 277442991 boards Hbf at 09:15:24 and
                    // reaches Alex at 09:21:48 (~6.5 min ride; query waits
                    // ~15 min for the next eastbound train).
                    label: "Berlin Hauptbahnhof -> Alexanderplatz (hand-picked platforms)",
                    origin: Endpoint::Stop("de:11000:900003201:1:50"),
                    target: Endpoint::Stop("de:11000:900100003:1:50"),
                },
                QuerySpec {
                    // Same Hbf -> Alex journey, but expressed station-to-station:
                    // expand each parent station to all of its child platforms
                    // (~301 children for Hbf, ~50 for Alex) and let the
                    // multi-source/target algorithm pick the best combination.
                    label: "Berlin Hauptbahnhof -> Alexanderplatz (station-to-station)",
                    origin: Endpoint::Station("de:11000:900003201"),
                    target: Endpoint::Station("de:11000:900100003"),
                },
            ],
        },
        FeedSpec {
            name: "Paris IDFM",
            path: "aux/external/paris.zip",
            service_date: date(2026, 5, 4),
            walking_footpaths_m: None,
            queries: &[
                QuerySpec {
                    label: "Châtelet -> Gare du Nord",
                    origin: Endpoint::Stop("IDFM:monomodalStopPlace:45102"),
                    target: Endpoint::Stop("IDFM:monomodalStopPlace:462394"),
                },
                QuerySpec {
                    label: "Châtelet -> La Défense",
                    origin: Endpoint::Stop("IDFM:monomodalStopPlace:45102"),
                    target: Endpoint::Stop("IDFM:monomodalStopPlace:470549"),
                },
                QuerySpec {
                    label: "Châtelet -> Versailles Rive Droite",
                    origin: Endpoint::Stop("IDFM:monomodalStopPlace:45102"),
                    target: Endpoint::Stop("IDFM:monomodalStopPlace:44602"),
                },
            ],
        },
    ]
}

fn main() -> anyhow::Result<()> {
    println!("# Cross-city benchmark\n");
    println!(
        "Departure: 09:00 ({DEPARTURE_TIME}s since midnight); query repeats: {QUERY_REPEATS} (warm cache; median reported)\n"
    );
    println!("| Feed | Service date | Stops | Routes | Trips (active on date) | Load time |");
    println!("|------|--------------|------:|-------:|----------------------:|----------:|");

    let mut feed_results: Vec<(String, Vec<String>)> = Vec::new();

    for feed in feeds() {
        if !Path::new(feed.path).exists() {
            eprintln!("skipping {} (file missing: {})", feed.name, feed.path);
            continue;
        }

        let load_start = Instant::now();
        let gtfs = Gtfs::new(feed.path)?;
        let timetable = {
            let mut tt = GtfsTimetable::new(&gtfs, feed.service_date)?;
            if let Some(d) = feed.walking_footpaths_m {
                tt = tt.with_walking_footpaths(&gtfs, d, 1.4);
            } else {
                // Without coordinate-derived edges, the publisher's
                // transfers.txt is the entire footpath relation. We
                // treat it as the publisher intended (single hop, no
                // chaining), which matches the v0.7 semantics and lets
                // the algorithm use the single-pass O(E) relaxation
                // instead of multi-source Dijkstra.
                tt = tt.assert_footpaths_closed();
            }
            tt
        };
        let load_elapsed = load_start.elapsed();

        println!(
            "| {} | {} | {} | {} | {} | {} |",
            feed.name,
            feed.service_date,
            timetable.n_stops(),
            timetable.n_routes(),
            timetable.n_trips(),
            format_duration(load_elapsed.as_nanos() as u64),
        );

        let mut cache = RaptorCache::for_timetable(&timetable);
        let mut query_lines: Vec<String> = Vec::new();

        for q in feed.queries {
            let origins = match resolve_endpoint(&timetable, &q.origin) {
                Ok(v) => v,
                Err(e) => {
                    query_lines.push(format!("| {} | (origin: {e}) |", q.label));
                    continue;
                }
            };
            let targets = match resolve_endpoint(&timetable, &q.target) {
                Ok(v) => v,
                Err(e) => {
                    query_lines.push(format!("| {} | (target: {e}) |", q.label));
                    continue;
                }
            };

            // Warm up.
            for _ in 0..3 {
                let _ = timetable
                    .query()
                    .from(&origins)
                    .to(&targets)
                    .max_transfers(10)
                    .depart_at(DEPARTURE_TIME)
                    .run_with_cache(&mut cache);
            }

            // Measure.
            let mut samples_ns: Vec<u128> = Vec::with_capacity(QUERY_REPEATS);
            let mut last_arrival: Option<SecondOfDay> = None;
            let mut last_journey_count: usize = 0;
            for _ in 0..QUERY_REPEATS {
                let t0 = Instant::now();
                let journeys = timetable
                    .query()
                    .from(&origins)
                    .to(&targets)
                    .max_transfers(10)
                    .depart_at(DEPARTURE_TIME)
                    .run_with_cache(&mut cache);
                samples_ns.push(t0.elapsed().as_nanos());
                last_journey_count = journeys.len();
                last_arrival = journeys.iter().map(|j| j.arrival()).min();
            }
            samples_ns.sort_unstable();
            let median = samples_ns[samples_ns.len() / 2];
            let arrival_str = match last_arrival {
                Some(t) if t >= DEPARTURE_TIME => {
                    let travel = (t - DEPARTURE_TIME).0;
                    format!("{}m {}s (arr {})", travel / 60, travel % 60, format_hms(t))
                }
                Some(t) => format!(
                    "ARR<DEP ({} < {})",
                    format_hms(t),
                    format_hms(DEPARTURE_TIME)
                ),
                None => "no journey".to_string(),
            };
            query_lines.push(format!(
                "| {} | {} | {} journey(s); travel {} |",
                q.label,
                format_duration(median as u64),
                last_journey_count,
                arrival_str,
            ));
        }

        feed_results.push((feed.name.to_string(), query_lines));
    }

    for (name, lines) in &feed_results {
        println!("\n## {name}\n");
        println!("| Query | Median latency | Result |");
        println!("|-------|---------------:|--------|");
        for line in lines {
            println!("{line}");
        }
    }

    Ok(())
}

/// Resolve a `QuerySpec` endpoint to a `Vec<(StopIdx, walk_time)>`.
/// `Stop` endpoints become a single-element vec; `Station` endpoints
/// expand to all child platforms via `tt.station_stops(...)`.
fn resolve_endpoint(
    tt: &GtfsTimetable,
    e: &Endpoint,
) -> Result<Vec<(vulture::StopIdx, vulture::Duration)>, String> {
    match *e {
        Endpoint::Stop(id) => match tt.stop_idx(id) {
            Some(idx) => Ok(vec![(idx, vulture::Duration::ZERO)]),
            None => Err(format!("unknown stop `{id}`")),
        },
        Endpoint::Station(id) => {
            let children = tt.station_stops(id);
            if children.is_empty() {
                Err(format!("unknown station or no children: `{id}`"))
            } else {
                Ok(children.to_vec())
            }
        }
    }
}

fn format_hms(t: SecondOfDay) -> String {
    let (h, m, s) = t.as_hms();
    format!("{h:02}:{m:02}:{s:02}")
}

fn format_duration(ns: u64) -> String {
    if ns < 1_000 {
        format!("{ns} ns")
    } else if ns < 1_000_000 {
        format!("{:.2} µs", ns as f64 / 1_000.0)
    } else if ns < 1_000_000_000 {
        format!("{:.2} ms", ns as f64 / 1_000_000.0)
    } else {
        format!("{:.2} s", ns as f64 / 1_000_000_000.0)
    }
}
