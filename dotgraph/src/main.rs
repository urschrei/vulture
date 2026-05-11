use clap::{Parser, Subcommand};
use vulture::manual::{SimpleTimetable, builders};

/// Render one of the synthetic networks from `vulture::manual::builders` as
/// Graphviz DOT. Pipe the output into `dot -Tpng` (or similar) to view.
#[derive(Parser)]
#[command(name = "vulture-dotgraph", about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    shape: Shape,
}

#[derive(Subcommand)]
enum Shape {
    /// Single route with N stops and M trips.
    Linear {
        #[arg(long, short)]
        stops: usize,
        #[arg(long, short)]
        trips: usize,
    },
    /// Grid of horizontal routes joined by vertical connectors.
    Grid {
        #[arg(long, short)]
        routes: usize,
        #[arg(long, short = 'c')]
        stops_per_route: usize,
        #[arg(long)]
        connectors: usize,
    },
    /// Hub-and-spoke network with footpath-connected hubs.
    HubSpoke {
        #[arg(long)]
        hubs: usize,
        #[arg(long, short = 'r')]
        routes_per_hub: usize,
        #[arg(long, short = 's')]
        stops_per_spoke: usize,
    },
    /// Chain of single-leg routes that force a transfer at every segment.
    Chain {
        #[arg(long, short)]
        segments: usize,
    },
    /// Parallel paths from source to target, each with a different leg count.
    ParallelPaths {
        #[arg(long, short = 'p')]
        path_count: usize,
        #[arg(long, short = 'm')]
        max_legs: usize,
    },
}

impl Shape {
    fn build(self) -> (SimpleTimetable<usize, usize, usize>, &'static str) {
        match self {
            Self::Linear { stops, trips } => (builders::build_linear(stops, trips), "linear"),
            Self::Grid {
                routes,
                stops_per_route,
                connectors,
            } => (
                builders::build_grid(routes, stops_per_route, connectors),
                "grid",
            ),
            Self::HubSpoke {
                hubs,
                routes_per_hub,
                stops_per_spoke,
            } => (
                builders::build_hub_spoke(hubs, routes_per_hub, stops_per_spoke),
                "hub_spoke",
            ),
            Self::Chain { segments } => (builders::build_chain(segments), "chain"),
            Self::ParallelPaths {
                path_count,
                max_legs,
            } => (
                builders::build_parallel_paths(path_count, max_legs),
                "parallel_paths",
            ),
        }
    }
}

fn main() {
    let (timetable, name) = Cli::parse().shape.build();
    let dot = timetable.to_dot(name).expect("rendering DOT failed");
    print!("{dot}");
}
