use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use gtfs_structures::Gtfs;
use jiff::civil::{Date, date};
use std::hint::black_box;
use vulture::gtfs::GtfsTimetable;
use vulture::{Duration, SecondOfDay};
use vulture::{RaptorCache, RaptorCachePool, Timetable};

const GTFS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../aux/dmrc_gtfs.zip");

// Departure time: 09:00:00 in seconds since midnight.
const DEPARTURE_TIME: SecondOfDay = SecondOfDay::hms(9, 0, 0);

// A Monday inside the bundled Delhi feed's calendar window
// (which ends 2025-12-31).
fn service_date() -> Date {
    date(2024, 1, 15)
}

// Query pairs picked to exercise different parts of the algorithm.
// Stop IDs are GTFS string IDs from aux/dmrc_gtfs.zip (Delhi Metro).
//
// The Delhi Metro feed has no transfers.txt, so cross-line journeys rely
// on interchange stations being shared physical stops on multiple routes
// (e.g. Kashmere Gate, Rajiv Chowk).
const QUERIES: &[(&str, &str, &str)] = &[
    // Direct trip on the Red Line east end (Dilshad Garden -> Shahdara).
    // One trip, ~10 minutes.
    ("direct_1trip", "1", "4"),
    // Two-trip cross-network (Dilshad Garden -> Vishwavidyalaya) via
    // Kashmere Gate interchange. ~30 minutes.
    ("transfer_2trip", "1", "44"),
    // Three-trip journey across three lines (Paschim Vihar West ->
    // Ghitorni) via Kirti Nagar and Rajiv Chowk. ~82 minutes.
    ("transfer_3trip", "29", "65"),
];

fn bench_gtfs_load(c: &mut Criterion) {
    c.bench_function("gtfs_load", |b| {
        b.iter(|| {
            let gtfs = Gtfs::new(GTFS_PATH).unwrap();
            let timetable = GtfsTimetable::new(&gtfs, service_date()).unwrap();
            black_box(&timetable);
        });
    });
}

fn bench_gtfs_query(c: &mut Criterion) {
    let gtfs = Gtfs::new(GTFS_PATH).unwrap();
    let timetable = GtfsTimetable::new(&gtfs, service_date()).unwrap();
    let mut cache = RaptorCache::for_timetable(&timetable);

    let mut group = c.benchmark_group("gtfs_query");
    for &(name, start_id, target_id) in QUERIES {
        let start = match timetable.stop_idx(start_id) {
            Some(s) => s,
            None => panic!("query `{name}`: unknown start stop `{start_id}`"),
        };
        let target = match timetable.stop_idx(target_id) {
            Some(s) => s,
            None => panic!("query `{name}`: unknown target stop `{target_id}`"),
        };
        group.bench_with_input(
            BenchmarkId::new("raptor", name),
            &(start, target),
            |b, &(start, target)| {
                b.iter(|| {
                    let journeys = timetable
                        .query()
                        .from(&[(start, Duration::ZERO)])
                        .to(&[(target, Duration::ZERO)])
                        .max_transfers(10)
                        .depart_at(DEPARTURE_TIME)
                        .run_with_cache(&mut cache);
                    black_box(&journeys);
                });
            },
        );
    }
    group.finish();
}

/// Range query across a 60-departure window: 09:00 to 10:00 in 1-minute
/// steps. Compares serial `.run_with_cache(&mut cache)` (rRAPTOR – single
/// reverse-chronological scan reusing labels across departures, paper §4)
/// against parallel `.run_with_pool(&pool)` (naïve batch fanned across
/// cores). For wide windows on multicore the parallel arm wins; for
/// single-core or narrow windows rRAPTOR wins.
fn bench_gtfs_range_query(c: &mut Criterion) {
    let gtfs = Gtfs::new(GTFS_PATH).unwrap();
    let timetable = GtfsTimetable::new(&gtfs, service_date()).unwrap();
    let mut cache = RaptorCache::for_timetable(&timetable);
    let pool = RaptorCachePool::for_timetable(&timetable);

    // 09:00–10:00 in 1-minute steps = 60 departures.
    let window_start = SecondOfDay::hms(9, 0, 0);
    let window_end = SecondOfDay::hms(10, 0, 0);
    let departures: Vec<SecondOfDay> = SecondOfDay::every(window_start, window_end, 60).collect();

    let mut group = c.benchmark_group("gtfs_range");
    for &(name, start_id, target_id) in QUERIES {
        let start = timetable.stop_idx(start_id).expect("unknown start stop");
        let target = timetable.stop_idx(target_id).expect("unknown target stop");

        group.bench_with_input(
            BenchmarkId::new("serial", name),
            &(start, target),
            |b, &(start, target)| {
                b.iter(|| {
                    let profile = timetable
                        .query()
                        .from(&[(start, Duration::ZERO)])
                        .to(&[(target, Duration::ZERO)])
                        .max_transfers(10)
                        .depart_in_window(departures.iter().copied())
                        .run_with_cache(&mut cache);
                    black_box(&profile);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("parallel", name),
            &(start, target),
            |b, &(start, target)| {
                b.iter(|| {
                    let profile = timetable
                        .query()
                        .from(&[(start, Duration::ZERO)])
                        .to(&[(target, Duration::ZERO)])
                        .max_transfers(10)
                        .depart_in_window(departures.iter().copied())
                        .run_with_pool(&pool);
                    black_box(&profile);
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    gtfs_benches,
    bench_gtfs_load,
    bench_gtfs_query,
    bench_gtfs_range_query
);
criterion_main!(gtfs_benches);
