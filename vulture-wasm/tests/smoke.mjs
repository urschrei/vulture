// Smoke test: load the bundled Delhi Metro GTFS, build a timetable,
// run a query, assert the journey shape matches what vulture's
// native tests assert.
//
// Run with:  node vulture-wasm/tests/smoke.mjs
//
// Requires the wasm-pack `nodejs` build to exist:
//   (cd vulture-wasm && wasm-pack build --target nodejs --release)

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { VultureTimetable, runArrival, runRange } from "../pkg/vulture_wasm.js";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..", "..");
const gtfsBytes = readFileSync(join(repoRoot, "aux", "dmrc_gtfs.zip"));

console.log(`loaded ${gtfsBytes.length} bytes of GTFS zip`);

const tt = new VultureTimetable(gtfsBytes, "2024-01-15");
console.log(`stops=${tt.nStops()} routes=${tt.nRoutes()}`);

const start = tt.stopIdx("1"); // Dilshad Garden
const target = tt.stopIdx("44"); // Vishwavidyalaya
if (start === undefined || target === undefined) {
    throw new Error("known stops not found");
}
console.log(`stop_idx: start=${start} target=${target}`);

// Depart at 09:00:00 = 32400 s; up to 10 transfers; no wheelchair filter.
const journeys = runArrival(
    tt,
    new Uint32Array([start]),
    new Uint32Array([target]),
    10,
    9 * 3600,
    false,
);

const fmt = (s) => {
    const hh = Math.floor(s / 3600);
    const mm = Math.floor((s % 3600) / 60);
    const ss = s % 60;
    return `${String(hh).padStart(2, "0")}:${String(mm).padStart(2, "0")}:${String(ss).padStart(2, "0")}`;
};

console.log(`got ${journeys.length} journey(s):`);
for (const j of journeys) {
    console.log(
        `  ${tt.stopName(j.origin)} -> ${tt.stopName(j.target)}, arrives ${fmt(j.arrival)}`,
    );
    for (const leg of j.legs) {
        console.log(
            `    ${fmt(leg.depart)} ${tt.stopName(leg.board_stop)} -> ${fmt(leg.arrive)} ${tt.stopName(leg.alight_stop)}  on ${tt.routeName(leg.route)}`,
        );
    }
}

if (journeys.length === 0) {
    throw new Error("expected at least one journey");
}
const j = journeys[0];
if (j.origin !== start || j.target !== target) {
    throw new Error(`origin/target mismatch: ${JSON.stringify(j)}`);
}
if (j.legs.length === 0) {
    throw new Error("expected at least one transit leg");
}
if (!j.legs.every((l) => l.depart > 0 && l.arrive > l.depart)) {
    throw new Error("legs missing timing");
}

// Range-query smoke: 09:00, 09:15, 09:30 → at least one Pareto entry.
const departures = new Uint32Array([9 * 3600, 9 * 3600 + 900, 9 * 3600 + 1800]);
const range = runRange(tt, start, target, 10, departures, false);
console.log(`runRange returned ${range.length} entries`);
if (range.length === 0) {
    throw new Error("expected at least one range entry");
}

// Bulk catalogue smoke.
const stops = tt.allStops();
const routes = tt.allRoutes();
console.log(`allStops: ${stops.length} stops; allRoutes: ${routes.length} routes`);
console.log(`  example stop: ${JSON.stringify(stops[0])}`);
console.log(`  example route: ${JSON.stringify(routes[0])}`);

console.log("smoke test passed");
