//! Microbenchmarks against the synthetic networks from
//! [`vulture::manual::builders`]. Each group probes a different scaling axis:
//! more stops/trips on one route, dense grid scanning, footpath-mediated hub
//! transfers, the cost of additional RAPTOR rounds, and journey
//! reconstruction depth and breadth.

use criterion::measurement::WallTime;
use criterion::{BenchmarkGroup, BenchmarkId, Criterion, criterion_group, criterion_main};
use vulture::Timetable;
use vulture::manual::SimpleTimetable;
use vulture::manual::builders::{
    build_chain, build_grid, build_hub_spoke, build_linear, build_parallel_paths,
};
use vulture::{Duration, SecondOfDay};

type Net = SimpleTimetable<usize, usize, usize>;

/// Register one `raptor/<param>` benchmark inside `group`. All RAPTOR queries
/// in this file share the same shape (single source, single target, fixed
/// departure at 00:00), so the helper just varies the routing inputs.
fn run_raptor(
    group: &mut BenchmarkGroup<'_, WallTime>,
    param: String,
    tt: &Net,
    source_key: usize,
    target_key: usize,
    max_transfers: u8,
) {
    let source = tt.stop_idx_of(&source_key);
    let target = tt.stop_idx_of(&target_key);
    group.bench_with_input(BenchmarkId::new("raptor", param), tt, move |b, tt| {
        b.iter(|| {
            tt.query()
                .from(&[(source, Duration::ZERO)])
                .to(&[(target, Duration::ZERO)])
                .max_transfers(max_transfers)
                .depart_at(SecondOfDay(0))
                .run()
        })
    });
}

/// Stop 0 → last stop on a single route. Scales with stops × trips.
fn bench_linear(c: &mut Criterion) {
    let mut group = c.benchmark_group("linear");
    for &(stops, trips) in &[(10usize, 5usize), (50, 10), (200, 20)] {
        let tt = build_linear(stops, trips);
        run_raptor(
            &mut group,
            format!("{stops}s_{trips}t"),
            &tt,
            0,
            stops - 1,
            3,
        );
    }
    group.finish();
}

/// Top-left → bottom-right of a grid: routes must transfer between rows via
/// vertical connectors, exercising round-by-round route scanning.
fn bench_grid(c: &mut Criterion) {
    let mut group = c.benchmark_group("grid");
    for &(routes, stops_per_route, connectors) in
        &[(3usize, 10usize, 2usize), (5, 20, 4), (10, 30, 6)]
    {
        let tt = build_grid(routes, stops_per_route, connectors);
        let target = (routes - 1) * stops_per_route + stops_per_route - 1;
        run_raptor(
            &mut group,
            format!("{routes}r_{stops_per_route}s_{connectors}c"),
            &tt,
            0,
            target,
            5,
        );
    }
    group.finish();
}

/// First spoke of hub 0 → last spoke of the last hub. Hubs are linked by
/// 2-second footpaths, so a journey crosses one or more hub-to-hub transfers.
fn bench_hub_spoke(c: &mut Criterion) {
    let mut group = c.benchmark_group("hub_spoke");
    for &(hubs, routes_per_hub, spokes) in &[(1usize, 10usize, 10usize), (3, 10, 10), (3, 20, 15)] {
        let tt = build_hub_spoke(hubs, routes_per_hub, spokes);
        let source = hubs; // first spoke (hub keys occupy 0..hubs)
        let target = hubs + hubs * routes_per_hub * spokes - 1; // last spoke of last hub
        run_raptor(
            &mut group,
            format!("{hubs}h_{routes_per_hub}r_{spokes}sp"),
            &tt,
            source,
            target,
            5,
        );
    }
    group.finish();
}

/// Same grid, varying `max_transfers`: each additional round of RAPTOR has a
/// cost; this group measures it on a fixed network.
fn bench_transfer_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("transfer_scaling");
    let tt = build_grid(5, 20, 4);
    let target = 4 * 20 + 19;
    for k in [1u8, 3, 5, 10, 20] {
        run_raptor(&mut group, format!("k{k}"), &tt, 0, target, k);
    }
    group.finish();
}

/// Chain of single-leg routes: stop 0 → stop `segments`. Forces exactly
/// `segments` transfers, so the work is dominated by journey reconstruction
/// walking back through the transfer chain.
fn bench_reconstruction_depth(c: &mut Criterion) {
    let mut group = c.benchmark_group("reconstruction_depth");
    for segments in [3usize, 5, 10, 20] {
        let tt = build_chain(segments);
        run_raptor(
            &mut group,
            format!("{segments}seg"),
            &tt,
            0,
            segments,
            (segments + 1) as u8,
        );
    }
    group.finish();
}

/// Parallel paths from stop 0 to stop 1: many alternative journeys to
/// enumerate during reconstruction.
fn bench_reconstruction_breadth(c: &mut Criterion) {
    let mut group = c.benchmark_group("reconstruction_breadth");
    for &(paths, max_legs) in &[(3usize, 3usize), (5, 5), (10, 10)] {
        let tt = build_parallel_paths(paths, max_legs);
        run_raptor(
            &mut group,
            format!("{paths}p_{max_legs}l"),
            &tt,
            0,
            1,
            (max_legs + 1) as u8,
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_linear,
    bench_grid,
    bench_hub_spoke,
    bench_transfer_scaling,
    bench_reconstruction_depth,
    bench_reconstruction_breadth,
);
criterion_main!(benches);
