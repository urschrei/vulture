# vulture-wasm

WebAssembly bindings for [vulture](../vulture/), targeting browsers via
[wasm-bindgen](https://rustwasm.github.io/wasm-bindgen/) and
[wasm-pack](https://rustwasm.github.io/wasm-pack/).

The crate exposes a single struct `VultureTimetable` and three free
functions (`runArrival`, `runRange`, plus catalogue accessors on the
struct) — the JS side fetches a GTFS zip with `fetch()`, hands the
bytes to `new VultureTimetable(bytes, "YYYY-MM-DD")`, and runs queries
against it. There is no HTTP code inside the WASM blob; the network
layer stays in JS where the browser's CORS / caching / progress
plumbing already lives.

## Build

```sh
# From the repo root:
./scripts/build-demo.sh                   # build into docs/demo
./scripts/build-demo.sh --serve           # build + spin up local http server

# Or call wasm-pack directly:
wasm-pack build vulture-wasm --target web --release
wasm-pack build vulture-wasm --target nodejs --release   # for the smoke test
```

`scripts/build-demo.sh` reports the wasm bundle size; the typical
release-mode output is ~700 KB raw / ~275 KB gzipped for browser
delivery.

## Smoke test

```sh
# Build the nodejs target first.
wasm-pack build vulture-wasm --target nodejs --release
node vulture-wasm/tests/smoke.mjs
```

Loads the bundled Delhi Metro feed, runs a single-departure query
and a 3-departure range query, prints the journey itinerary with
stop names and route names. Asserts on the expected shape.

## Demo page

The `docs/demo/` directory hosts an HTML demo using the wasm-pack
`web` target. It loads the bundled Delhi Metro feed, populates a
station picker, and exposes three panels:

1. **Stop-to-stop** — single-departure query with full timed itinerary.
2. **Departure window** — `runRange` over a configurable time slice,
   showing the Pareto-filtered options.
3. **Walking footpaths** — toggle `withWalkingFootpaths` to see how
   coordinate-derived walking edges affect routing.

Live at <https://urschrei.github.io/vulture/> after the
`pages.yml` workflow deploys.

## Status

This crate is `publish = false` — the binding surface is still being
shaped and pinning a `vulture-wasm` version on crates.io would
prematurely commit to a JS API. The wheel for browser use comes from
the GitHub Pages deploy, not from npm.
