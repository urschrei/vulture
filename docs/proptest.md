# Prompt: Property-Based Tests for raptor-rs

## Context

We are working in a fork of `keogami/raptor-rs`, a Rust implementation of the RAPTOR
public transit routing algorithm (Delling, Pajor & Werneck, *Transportation
Science* 49(3), 2015, DOI 10.1287/trsc.2014.0534). The v0.2.0 baseline has
known soundness issues documented in `soundness.md` — the existing hand-
written tests pass because they only exercise synthetic networks that
sidestep the unsound code paths.

Your task: build a property-based test harness using Hegel that compares
the algorithm's output against a brute-force reference solver on randomly
generated networks. This is the single most valuable test in the
repository — it should catch every existing soundness issue and prevent
regressions as we fix them.
Property to verify
For any well-formed timetable T, source stop pₛ, target stop pₜ, departure
time τ, and max-transfers bound k:

The Pareto front of (arrival, transfers) pairs returned by
T.raptor(k, τ, pₛ, pₜ) equals the Pareto front computed by a brute-
force time-expanded Dijkstra on the same inputs.

Equality is over Pareto fronts, not journeys. Two implementations may
legitimately return different journey witnesses (different boarding stops,
different equally-fast trips) for the same (arrival, transfers) point.
The front is the invariant; the witnesses are not. Assert on the set of
(arrival, transfers) pairs only.
Critical convention to nail down before coding
RAPTOR's transfers parameter is misleadingly named. The paper counts
trips taken, not transfers. A journey "board R1 to B, board R2 to D"
has 2 trips and 1 transfer. The trait's transfers parameter is the trip
count.
Decide and document in a top-of-file comment in the test module: we are
matching trip counts. Both the RAPTOR caller and the reference solver
must use the same convention. This is the single most likely source of
"works on paper, fails in tests" debugging time. Get it right once and
write it down.
Design: spec, render, compare
The harness has three logical pieces:

A declarative network spec — a small, easy-to-shrink data structure
that fully describes a generated test case.
A deterministic renderer that turns a spec into a concrete
SimpleTimetable<u8, u8, u16>.
A reference solver — a brute-force time-expanded Dijkstra that
takes the spec (or the rendered timetable; whichever is more
convenient) and returns the ground-truth Pareto front.

This separation matters: the test framework shrinks the spec on
failure, and the renderer guarantees that every shrunk spec produces a
valid timetable. Don't generate SimpleTimetable directly — that path
shrinks badly and produces invalid intermediate states.
The network spec
rustpub struct NetworkSpec {
    pub n_stops: u8,                  // 2..=6
    pub routes: Vec<RouteSpec>,       // 1..=4
    pub footpaths: Vec<FootpathSpec>, // 0..=6
    pub query: QuerySpec,
}

pub struct RouteSpec {
    /// Stop indices in route order. 2..=4 distinct stops, all < n_stops.
    pub stop_sequence: Vec<u8>,
    /// (arrival, departure) per stop, per trip. 1..=3 trips per route.
    pub trips: Vec<Vec<(u16, u16)>>,
}

pub struct FootpathSpec {
    pub from: u8,           // < n_stops
    pub to: u8,             // < n_stops, != from
    pub walk_time: u16,     // 1..=300
}

pub struct QuerySpec {
    pub ps: u8,             // < n_stops
    pub pt: u8,             // < n_stops
    pub tau: u16,           // 0..=500
    pub max_transfers: u8,  // 1..=5
}
Adjust field types to whatever Hegel finds easiest to generate and shrink;
the ranges are what matter. Keep integers small to keep shrinking
effective.
Invariants the renderer must enforce
Some of these can be enforced at generation time, some are easier to fix
up in the renderer. Choose whichever produces cleaner shrinking — but the
output of the renderer must always satisfy all of them:

All stop indices in stop_sequence are < n_stops and distinct within
a route.
Each trip has the same length as its route's stop_sequence.
Within a trip, (arrival, departure) pairs are monotonically non-
decreasing across stops; departure ≥ arrival at each stop.
Across trips on the same route: no overtaking. For any two trips on
the same route, if trip A departs the first stop before trip B, then A
arrives at every subsequent stop no later than B. RAPTOR assumes this
(paper §3.1).
Footpath endpoints differ and both are < n_stops.
Footpaths are transitively closed. If the spec contains A→B and
B→C, the rendered timetable contains A→C with walk_time = walk(A,B) + walk(B,C). Easiest to compute closure in the renderer
(Floyd–Warshall on min-plus is fine for ≤6 stops) rather than
constraining the generator. The current implementation requires
transitively closed footpaths; until that's relaxed, leaving non-closed
inputs to the harness just produces noise.
ps and pt may be equal (degenerate-but-valid case; both solvers
must handle it).

Generation strategy
Three generator layers, structured so failures shrink to the smallest
layer that exhibits the bug:

Layer 1: trivial networks. 2–4 stops, 1–2 routes, 1–2 trips per
route, no footpaths. Covers the regime the existing hand-written tests
cover; should pass even on the buggy v0.2.0 baseline.
Layer 2: networks with footpaths. Adds 1–4 footpaths with non-zero
walking times. Targets soundness issues A–D from soundness.md. This
layer should fail on the v0.2.0 baseline — that's how we know the
harness works.
Layer 3: full networks. The complete generator, used in CI with a
larger case count.

Expose each layer as its own generator, so individual tests can target
individual layers and shrinking stays local to the layer.
The renderer
rustpub fn render(spec: &NetworkSpec) -> SimpleTimetable<u8, u8, u16>;
Total (no panics on any spec the generators produce) and deterministic
(same spec → byte-identical timetable). Build using SimpleTimetable's
existing builder API in raptor/src/simple/mod.rs.
The footpath transitive closure goes here, not in the generator: take
the generated footpath list, compute the all-pairs shortest-path closure
under min-plus algebra, emit the closure to the timetable.
The reference solver
A time-expanded Dijkstra. This is correctness-critical. Optimise
nothing. Write the most boring textbook implementation possible. If we
ever find ourselves debugging the reference solver, we've done something
wrong.
rustpub fn reference_solve(
    spec: &NetworkSpec,
    ps: u8,
    pt: u8,
    tau: u16,
    max_trips: u8,
) -> Vec<(u16, u8)>;  // Pareto front: (arrival, trips), sorted by trips ascending
Construction:

Nodes: (stop, time) for every time at which something happens at
that stop — every trip arrival, every trip departure, every footpath
endpoint time. The set is finite and small (a few hundred nodes for our
spec range). Build it explicitly as a BTreeSet<(u8, u16)> first.
Edges:

Ride (cost (arrival_time, +0 trips)): for each consecutive
(stop_i, stop_j) on each trip, edge from
(stop_i, departure_time_at_i) to (stop_j, arrival_time_at_j).
Board (cost (departure_time, +1 trip)): for each trip serving
stop, for each "current time" node (stop, t) with
t ≤ departure_time_of_trip_at_stop, edge to
(stop, departure_time_of_trip_at_stop).
Walk (cost (t + walk_time, +0 trips)): for each footpath
(stop_a, stop_b, walk_time) and each node (stop_a, t), edge to
(stop_b, t + walk_time).
Wait (cost (t', +0 trips)): from (stop, t) to (stop, t')
for any t' > t where (stop, t') is also a node. Lets passengers
sit at a stop without boarding.


Start: (ps, tau) with cost (tau, 0).
Goal: any (pt, *).

Cost model:
2D cost (arrival_time, n_trips). Multi-criterion Dijkstra: maintain a
set of non-dominated cost vectors per node (a Pareto front), not a
single best. A new candidate at a node is processed iff it is not
dominated by anything currently in that node's front.
For our spec ranges this is tiny — a few hundred nodes, fronts of size
≤ 5. BinaryHeap<Reverse<...>> plus HashMap<Node, Vec<Cost>> is
sufficient.
Output:
Collect all non-dominated (arrival, trips) pairs at any (pt, *)
node, filter to those with trips ≤ max_trips, return sorted by trips
ascending.
Sanity:

When ps == pt, return [(tau, 0)].
When the network is disconnected from the source, return [].

The harness should exercise both.
The Pareto-front comparison
Helper:
rustfn pareto_front(journeys: &[Journey<u8, u8>]) -> Vec<(u16, u8)>;
Extract (arrival, plan.len() as u8), sort by trips ascending, keep only
points where arrival is strictly less than the best seen so far. The
trip count under the convention documented at the top of the module is
plan.len() directly (see the existing Journey documentation in
raptor/src/lib.rs).
Compare fronts as BTreeSet<(u16, u8)>. On mismatch, dump:

The full NetworkSpec (small enough to read).
Both raw outputs (RAPTOR journey list, reference Pareto front).
A diff of the two fronts (which points are in one but not the other).

File layout
Suggested, not prescriptive:
raptor/src/proptest_support/
    mod.rs        // module entry, the property tests themselves
    spec.rs       // NetworkSpec types, generators, the renderer
    reference.rs  // the Dijkstra reference solver
    README.md     // notes for future contributors
Wire into raptor/src/lib.rs behind #[cfg(test)]:
rust#[cfg(test)]
mod proptest_support;
The module is internal; nothing here goes in the public API.
README content
The README should cover:

The trips-vs-transfers convention (one paragraph, with an example).
The generator layers and what soundness issues each targets (table
format, link to soundness.md).
How to reproduce a failure from whatever Hegel uses for failure
persistence.
How to extend with a new generator layer (e.g., for McRAPTOR in a
later phase of the roadmap).

Wall-clock budget
Target: full property-test suite under 10 seconds on a developer laptop
at default case counts. CI can crank Layer 3 higher for overnight runs.
If we exceed budget, the most likely culprit is over-generating "wait"
edges in the reference solver — cap them to transitions between adjacent
timepoints at a stop rather than all-pairs.
What to skip

Multi-criterion labels. Out of scope until Phase 2 of the roadmap.
Range queries (rRAPTOR). Out of scope.
Realtime overlays. Out of scope.
The GTFS adapter. Test the core algorithm via SimpleTimetable. A
separate harness for GtfsTimetable is worth doing eventually for
soundness issue E (route-pattern splitting), but isolating algorithm
bugs from adapter bugs is the priority.
Performance benchmarks. Different concern, different harness.

An open question: Does Hegel have opinions about how to express the "this layer should fail on the baseline" expectation? Some PBT frameworks have a notion of "expected failure" or "regression test"; others don't. If Hegel does, it's worth using that idiom rather than just letting the test go red — because then v0.3.0's deliverable is "the expected-failure tests turn green," which is a clearer signal.

What success looks like
When you're done:

cargo test --lib runs the full property-test suite in under 10
seconds and currently fails on the v0.2.0 baseline — at minimum on
Layer 2 (footpaths) due to soundness issues A and B. This is expected
and good; it confirms the harness detects real bugs.
The failing case shrinks to something readable — ideally a 3-stop,
1-route, 1-footpath spec where the optimal journey requires walking
from the source.
The README is enough that someone unfamiliar with the project can run
the harness, interpret a failure, and add a new generator layer.
No unwrap() or panic!() in reference.rs on any spec the
generators can produce.

A failing test on the unmodified v0.2.0 baseline is the deliverable —
that's how we know the harness works. Phase 0 of the roadmap (the actual
soundness fixes) is a separate piece of work that this harness will guide
and validate.
