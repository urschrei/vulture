# vulture-dotgraph

Renders the synthetic transit networks built by [`vulture::manual::builders`](../vulture/src/manual/builders.rs) – the same shapes the criterion bench harness exercises – as [Graphviz](https://graphviz.org/) DOT. Useful when you want to see what `grid/raptor/10r_30s_6c` (or any other criterion bench case from [`vulture/benches/raptor.rs`](../vulture/benches/raptor.rs)) actually looks like.

## Usage

```bash
# Pipe straight into `dot` for a PNG.
cargo run -p vulture-dotgraph -- linear --stops 10 --trips 5 | dot -Tpng > linear.png
cargo run -p vulture-dotgraph -- grid --routes 3 --stops-per-route 5 --connectors 2 | dot -Tpng > grid.png
cargo run -p vulture-dotgraph -- hub-spoke --hubs 3 --routes-per-hub 4 --stops-per-spoke 5 | dot -Tpng > hub_spoke.png
cargo run -p vulture-dotgraph -- chain --segments 4 | dot -Tpng > chain.png
cargo run -p vulture-dotgraph -- parallel-paths --path-count 3 --max-legs 4 | dot -Tpng > parallel_paths.png
```

`--help` lists the subcommands and their parameters.

## What it isn't

Not a GTFS visualiser. Real feeds (Berlin VBB at ~42k stops, Paris IDFM at ~54k) are unreadable as flat node-and-edge graphs and `dot` would not finish laying them out anyway. This tool is scoped to the small synthetic networks used for algorithm-correctness fixtures and microbenchmarks.

## Implementation

`vulture::manual::SimpleTimetable::to_dot(&self, name: &str)` does the rendering, gated behind the `dotgraph` Cargo feature on the `vulture` crate (which pulls the optional [`dot_graph`](https://crates.io/crates/dot_graph) dependency). This crate is just a thin `clap` CLI in front of that method plus the bench builders (gated behind the `internal` Cargo feature).
