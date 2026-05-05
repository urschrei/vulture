// Read two run-outputs from the comparison harness (one per
// implementation) and emit a side-by-side markdown table on stdout.
//
// Usage:
//     node compare.mjs <vulture.json> <raptor-js.json>
//
// Defaults to results/vulture.json and results/raptor-js.json under
// this directory.

import * as fs from "node:fs";
import { fileURLToPath } from "node:url";
import * as path from "node:path";

const HERE = path.dirname(fileURLToPath(import.meta.url));

function load(p) {
    const abs = path.isAbsolute(p) ? p : path.join(HERE, p);
    return JSON.parse(fs.readFileSync(abs, "utf8"));
}

function quantile(sortedAsc, q) {
    if (sortedAsc.length === 0) return null;
    const i = Math.min(sortedAsc.length - 1, Math.floor(q * sortedAsc.length));
    return sortedAsc[i];
}

function summarise(samplesNs) {
    if (!Array.isArray(samplesNs) || samplesNs.length === 0) return null;
    const sorted = [...samplesNs].sort((a, b) => a - b);
    return {
        n: sorted.length,
        min: sorted[0],
        median: quantile(sorted, 0.5),
        p95: quantile(sorted, 0.95),
        max: sorted[sorted.length - 1],
    };
}

function fmtTime(ns) {
    if (ns == null) return "—";
    if (ns < 1_000) return `${Math.round(ns)} ns`;
    if (ns < 1_000_000) return `${(ns / 1_000).toFixed(1)} µs`;
    if (ns < 1_000_000_000) return `${(ns / 1_000_000).toFixed(2)} ms`;
    return `${(ns / 1_000_000_000).toFixed(2)} s`;
}

function paretoKey(journeys) {
    // Compare on the (arrival, n_transit_legs) Pareto frontier so
    // tie-break differences between impls don't show as disagreements.
    return [...journeys]
        .map((j) => `${j.arrival_seconds}/${j.n_transit_legs}`)
        .sort()
        .join(",");
}

function main() {
    const vPath = process.argv[2] ?? "results/vulture.json";
    const jPath = process.argv[3] ?? "results/raptor-js.json";
    const v = load(vPath);
    const j = load(jPath);

    const out = [];
    out.push(`# Cross-implementation comparison`);
    out.push("");
    out.push(`- Vulture: ${v.implementation} v${v.version}`);
    out.push(`- Planar:  ${j.implementation} v${j.version}`);
    out.push(
        `- Methodology: ${v.configuration.warmup_iters} warmup + ${v.configuration.measure_iters} timed iterations per query, warm cache, depart 09:00, max ${v.configuration.max_transfers} transfers.`,
    );
    out.push("");
    out.push("## Per-feed");

    const jByFeed = new Map(j.feeds.map((f) => [f.name, f]));

    for (const vf of v.feeds) {
        const jf = jByFeed.get(vf.name);
        out.push("");
        out.push(`### ${vf.name}`);
        out.push("");
        if (!jf) {
            out.push(`*Planar feed result missing.*`);
            continue;
        }
        out.push(
            `Load time: vulture ${fmtTime(vf.load_time_ns)}, planar ${fmtTime(jf.load_time_ns)} ` +
                `(stops ${vf.n_stops}/${jf.n_stops}; trips ${vf.n_trips}/${jf.n_trips}).`,
        );
        out.push("");
        out.push("| Query | Vulture (median, p95) | Planar (median, p95) | Planar / vulture | Result |");
        out.push("|-------|----------------------:|---------------------:|-----------------:|--------|");

        const jByLabel = new Map(jf.queries.map((q) => [q.label, q]));
        for (const vq of vf.queries) {
            const jq = jByLabel.get(vq.label);
            if (!jq) {
                out.push(`| ${vq.label} | – | (planar query missing) | – | – |`);
                continue;
            }
            const vs = summarise(vq.samples_ns);
            const js = summarise(jq.samples_ns);
            const vCol = vs ? `${fmtTime(vs.median)}, ${fmtTime(vs.p95)}` : (vq.error ?? "—");
            const jCol = js ? `${fmtTime(js.median)}, ${fmtTime(js.p95)}` : (jq.error ?? "—");
            const ratio =
                vs && js && vs.median > 0
                    ? `${(js.median / vs.median).toFixed(0)}×`
                    : "—";
            const vKey = paretoKey(vq.journeys);
            const jKey = paretoKey(jq.journeys);
            let result;
            if (vKey === jKey && vKey !== "") {
                result = `match (${vq.journeys.length} journey${vq.journeys.length === 1 ? "" : "s"})`;
            } else if (vKey === "" && jKey === "") {
                result = "both empty";
            } else if (jKey === "") {
                result = `**planar empty** (vulture has ${vq.journeys.length})`;
            } else if (vKey === "") {
                result = `**vulture empty** (planar has ${jq.journeys.length})`;
            } else {
                result = `**MISMATCH** (v: ${vKey}; p: ${jKey})`;
            }
            out.push(`| ${vq.label} | ${vCol} | ${jCol} | ${ratio} | ${result} |`);
        }
    }

    out.push("");
    process.stdout.write(out.join("\n") + "\n");
}

main();
