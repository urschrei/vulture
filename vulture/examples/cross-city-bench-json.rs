//! JSON-emitting cross-city benchmark for cross-implementation comparison.
//!
//! Reads a query specification from `bench-queries.json` (path overridable
//! via the first CLI argument), runs each query through vulture's RAPTOR
//! over a warm `RaptorCache`, and emits a JSON document on stdout
//! containing per-query timing samples and reconstructed journey legs.
//!
//! The matching JS-side harness in `vulture-bench-js/harness.mjs` runs
//! the same queries through `raptor-journey-planner` and emits the same
//! shape, so a downstream diff can compare both implementations on
//! identical input.
//!
//! Usage:
//!     cargo run --release --example cross-city-bench-json \
//!         > vulture-bench-js/results/vulture.json

use anyhow::{Context, Result};
use gtfs_structures::Gtfs;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Instant;
use vulture::gtfs::GtfsTimetable;
use vulture::{Duration, RaptorCache, SecondOfDay, Timetable};

#[derive(Deserialize)]
struct BenchSpec {
    configuration: BenchConfig,
    feeds: Vec<FeedSpec>,
}

#[derive(Deserialize, Serialize, Clone)]
struct BenchConfig {
    departure_seconds: u32,
    max_transfers: u8,
    warmup_iters: usize,
    measure_iters: usize,
}

#[derive(Deserialize)]
struct FeedSpec {
    name: String,
    path: String,
    service_date: String,
    walking_footpaths_m: Option<f64>,
    queries: Vec<QuerySpec>,
}

#[derive(Deserialize)]
struct QuerySpec {
    label: String,
    origin_ids: Vec<String>,
    target_ids: Vec<String>,
}

#[derive(Serialize)]
struct OutputRoot {
    implementation: &'static str,
    version: &'static str,
    configuration: BenchConfig,
    feeds: Vec<OutputFeed>,
}

#[derive(Serialize)]
struct OutputFeed {
    name: String,
    path: String,
    service_date: String,
    walking_footpaths_m: Option<f64>,
    n_stops: usize,
    n_routes: usize,
    n_trips: usize,
    load_time_ns: u128,
    queries: Vec<OutputQuery>,
}

#[derive(Serialize)]
struct OutputQuery {
    label: String,
    origin_ids: Vec<String>,
    target_ids: Vec<String>,
    samples_ns: Vec<u128>,
    journeys: Vec<OutputJourney>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct OutputJourney {
    arrival_seconds: u32,
    n_transit_legs: usize,
    legs: Vec<OutputLeg>,
}

#[derive(Serialize)]
struct OutputLeg {
    #[serde(rename = "type")]
    leg_type: &'static str,
    trip_id: String,
    route_id: String,
    board_stop_id: String,
    board_seconds: u32,
    alight_stop_id: String,
    alight_seconds: u32,
}

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "bench-queries.json".into());
    let raw = std::fs::read_to_string(&path).with_context(|| format!("read {path}"))?;
    let spec: BenchSpec = serde_json::from_str(&raw)?;

    let mut feeds_out = Vec::new();
    for feed in &spec.feeds {
        if !Path::new(&feed.path).exists() {
            eprintln!("skipping {} (file missing: {})", feed.name, feed.path);
            continue;
        }

        let date: Date = feed.service_date.parse()?;
        let depart = SecondOfDay(spec.configuration.departure_seconds);
        let max_transfers = spec.configuration.max_transfers;

        let load_start = Instant::now();
        let gtfs = Gtfs::new(&feed.path)?;
        let mut tt = GtfsTimetable::new(&gtfs, date)?;
        if let Some(d) = feed.walking_footpaths_m {
            tt = tt.with_walking_footpaths(&gtfs, d, 1.4);
        } else {
            tt = tt.assert_footpaths_closed();
        }
        let load_time_ns = load_start.elapsed().as_nanos();
        eprintln!(
            "loaded {} ({} stops, {} routes, {} trips) in {} ms",
            feed.name,
            tt.n_stops(),
            tt.n_routes(),
            tt.n_trips(),
            load_time_ns / 1_000_000,
        );

        let mut cache = RaptorCache::for_timetable(&tt);
        let mut queries_out = Vec::new();

        for q in &feed.queries {
            let origins = resolve_ids(&tt, &q.origin_ids);
            let targets = resolve_ids(&tt, &q.target_ids);
            let (origins, targets) = match (origins, targets) {
                (Ok(o), Ok(t)) => (o, t),
                (Err(e), _) => {
                    queries_out.push(unresolved_query(q, format!("origin: {e}")));
                    continue;
                }
                (_, Err(e)) => {
                    queries_out.push(unresolved_query(q, format!("target: {e}")));
                    continue;
                }
            };

            for _ in 0..spec.configuration.warmup_iters {
                let _ = tt
                    .query()
                    .from(&origins)
                    .to(&targets)
                    .max_transfers(max_transfers)
                    .depart_at(depart)
                    .run_with_cache(&mut cache);
            }

            let mut samples_ns: Vec<u128> = Vec::with_capacity(spec.configuration.measure_iters);
            let mut last_journeys = Vec::new();
            for _ in 0..spec.configuration.measure_iters {
                let t0 = Instant::now();
                let journeys = tt
                    .query()
                    .from(&origins)
                    .to(&targets)
                    .max_transfers(max_transfers)
                    .depart_at(depart)
                    .run_with_cache(&mut cache);
                samples_ns.push(t0.elapsed().as_nanos());
                last_journeys = journeys;
            }

            let mut journeys_out = Vec::new();
            for j in &last_journeys {
                let timed = match j.with_timing(&tt, depart, Duration::ZERO) {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("warn: with_timing failed on {}: {e}", q.label);
                        continue;
                    }
                };
                let legs: Vec<OutputLeg> = timed
                    .iter()
                    .map(|leg| OutputLeg {
                        leg_type: "transit",
                        trip_id: tt.trip_id(leg.trip).to_string(),
                        route_id: tt.route_id(leg.route).to_string(),
                        board_stop_id: tt.stop_id(leg.board).to_string(),
                        board_seconds: leg.depart.0,
                        alight_stop_id: tt.stop_id(leg.alight).to_string(),
                        alight_seconds: leg.arrive.0,
                    })
                    .collect();
                journeys_out.push(OutputJourney {
                    arrival_seconds: j.arrival().0,
                    n_transit_legs: legs.len(),
                    legs,
                });
            }

            queries_out.push(OutputQuery {
                label: q.label.clone(),
                origin_ids: q.origin_ids.clone(),
                target_ids: q.target_ids.clone(),
                samples_ns,
                journeys: journeys_out,
                error: None,
            });
        }

        feeds_out.push(OutputFeed {
            name: feed.name.clone(),
            path: feed.path.clone(),
            service_date: feed.service_date.clone(),
            walking_footpaths_m: feed.walking_footpaths_m,
            n_stops: tt.n_stops(),
            n_routes: tt.n_routes(),
            n_trips: tt.n_trips(),
            load_time_ns,
            queries: queries_out,
        });
    }

    let out = OutputRoot {
        implementation: "vulture",
        version: env!("CARGO_PKG_VERSION"),
        configuration: spec.configuration,
        feeds: feeds_out,
    };
    serde_json::to_writer_pretty(std::io::stdout(), &out)?;
    println!();
    Ok(())
}

fn resolve_ids(
    tt: &GtfsTimetable,
    ids: &[String],
) -> Result<Vec<(vulture::StopIdx, Duration)>, String> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        match tt.stop_idx(id) {
            Some(idx) => out.push((idx, Duration::ZERO)),
            None => return Err(format!("unknown stop `{id}`")),
        }
    }
    Ok(out)
}

fn unresolved_query(q: &QuerySpec, err: String) -> OutputQuery {
    OutputQuery {
        label: q.label.clone(),
        origin_ids: q.origin_ids.clone(),
        target_ids: q.target_ids.clone(),
        samples_ns: vec![],
        journeys: vec![],
        error: Some(err),
    }
}
