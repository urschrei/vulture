use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use raptor::Timetable;
use raptor::simple::builders::*;

/// Routes from the first stop to the last on a single route. Tests how performance
/// scales as the number of stops and trips on one route grows. (explained by llm)
fn bench_linear(c: &mut Criterion) {
    let mut group = c.benchmark_group("linear");
    for &(stops, trips) in &[(10, 5), (50, 10), (200, 20)] {
        let tt = build_linear(stops, trips);
        group.bench_with_input(
            BenchmarkId::new("raptor", format!("{stops}s_{trips}t")),
            &tt,
            |b, tt| {
                let source = tt.stop_idx_of(&0usize);
                let target = tt.stop_idx_of(&(stops - 1));
                b.iter(|| tt.raptor(3, 0, source, target))
            },
        );
    }
    group.finish();
}

/// Routes from the top-left to the bottom-right of a grid. Horizontal routes form rows,
/// vertical connector routes link columns. The query must transfer between rows via
/// connectors, testing how the algorithm handles scanning many routes per round. (explained by llm)
fn bench_grid(c: &mut Criterion) {
    let mut group = c.benchmark_group("grid");
    for &(routes, stops_per_route, connectors) in &[(3, 10, 2), (5, 20, 4), (10, 30, 6)] {
        let tt = build_grid(routes, stops_per_route, connectors);
        let source_key = 0usize;
        let target_key = (routes - 1) * stops_per_route + stops_per_route - 1;
        group.bench_with_input(
            BenchmarkId::new(
                "raptor",
                format!("{routes}r_{stops_per_route}s_{connectors}c"),
            ),
            &tt,
            |b, tt| {
                let source = tt.stop_idx_of(&source_key);
                let target = tt.stop_idx_of(&target_key);
                b.iter(|| tt.raptor(5, 0, source, target))
            },
        );
    }
    group.finish();
}

/// Routes from a spoke on the first hub to a spoke on the last hub. Hubs are connected
/// by footpaths, so the query must walk between hubs and then ride outward. Tests footpath
/// handling and route selection in dense, hub-centered networks. (explained by llm)
fn bench_hub_spoke(c: &mut Criterion) {
    let mut group = c.benchmark_group("hub_spoke");
    for &(hubs, routes_per_hub, spokes) in &[(1, 10, 10), (3, 10, 10), (3, 20, 15)] {
        let tt = build_hub_spoke(hubs, routes_per_hub, spokes);
        // Route from first spoke of first hub to last spoke of last hub
        let source_key = hubs; // first non-hub stop
        let last_hub_first_route_last_spoke =
            hubs + (hubs - 1) * routes_per_hub * spokes + (routes_per_hub - 1) * spokes + spokes
                - 1;
        let target_key =
            last_hub_first_route_last_spoke.min(hubs + hubs * routes_per_hub * spokes - 1);
        group.bench_with_input(
            BenchmarkId::new("raptor", format!("{hubs}h_{routes_per_hub}r_{spokes}sp")),
            &tt,
            |b, tt| {
                let source = tt.stop_idx_of(&source_key);
                let target = tt.stop_idx_of(&target_key);
                b.iter(|| tt.raptor(5, 0, source, target))
            },
        );
    }
    group.finish();
}

/// Runs the same grid query with increasing max transfers (k = 1, 3, 5, 10, 20). Tests
/// how much each additional RAPTOR round costs on a fixed network. (explained by llm)
fn bench_transfer_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("transfer_scaling");
    let tt = build_grid(5, 20, 4);
    let source = tt.stop_idx_of(&0usize);
    let target = tt.stop_idx_of(&(4 * 20 + 19usize));
    for k in [1, 3, 5, 10, 20] {
        group.bench_with_input(BenchmarkId::new("raptor", format!("k{k}")), &k, |b, &k| {
            b.iter(|| tt.raptor(k, 0, source, target))
        });
    }
    group.finish();
}

/// A chain of single-hop routes (A→B, B→C, C→D, ...) forces exactly N transfers. Tests
/// how journey reconstruction scales when the path has many transfer steps to walk back
/// through. (explained by llm)
fn bench_reconstruction_depth(c: &mut Criterion) {
    let mut group = c.benchmark_group("reconstruction_depth");
    for segments in [3, 5, 10, 20] {
        let tt = build_chain(segments);
        group.bench_with_input(
            BenchmarkId::new("raptor", format!("{segments}seg")),
            &tt,
            |b, tt| {
                let source = tt.stop_idx_of(&0usize);
                let target = tt.stop_idx_of(&segments);
                b.iter(|| tt.raptor(segments + 1, 0, source, target))
            },
        );
    }
    group.finish();
}

/// Multiple parallel paths from source to target, each with a different number of legs.
/// Tests journey reconstruction when there are many alternative journeys to
/// enumerate. (explained by llm)
fn bench_reconstruction_breadth(c: &mut Criterion) {
    let mut group = c.benchmark_group("reconstruction_breadth");
    for &(paths, max_legs) in &[(3, 3), (5, 5), (10, 10)] {
        let tt = build_parallel_paths(paths, max_legs);
        group.bench_with_input(
            BenchmarkId::new("raptor", format!("{paths}p_{max_legs}l")),
            &tt,
            |b, tt| {
                let source = tt.stop_idx_of(&0usize);
                let target = tt.stop_idx_of(&1usize);
                b.iter(|| tt.raptor(max_legs + 1, 0, source, target))
            },
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
