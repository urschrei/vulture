// Vulture demo: load the bundled Delhi Metro GTFS, build a timetable
// in WASM, wire up three demo panels (single, range, walking-footpaths).

import init, {
    VultureTimetable,
    runArrival,
    runRange,
} from "./pkg/vulture_wasm.js";

const SERVICE_DATE = "2024-01-15";
const FEED_URL = "./dmrc_gtfs.zip";

// Delhi Metro routes don't ship colour fields, but the route long_name
// is prefixed with the line colour ("RED_…", "YELLOW_…"). Map those
// to display colours so the journey timeline is readable.
const LINE_COLOURS = {
    RED: "#dc2626",
    YELLOW: "#ca8a04",
    BLUE: "#2563eb",
    GREEN: "#16a34a",
    VIOLET: "#7c3aed",
    PINK: "#db2777",
    ORANGE: "#ea580c",
    MAGENTA: "#c026d3",
    GRAY: "#6b7280",
    GREY: "#6b7280",
    AIRPORT: "#0891b2",
    RAPID: "#f97316",
};

function lineColour(routeName) {
    if (!routeName) return "#6b7280";
    const prefix = routeName.split(/[_\s]/)[0].toUpperCase();
    return LINE_COLOURS[prefix] || "#6b7280";
}

function fmtTime(s) {
    const hh = Math.floor(s / 3600) % 24;
    const mm = Math.floor((s % 3600) / 60);
    const ss = s % 60;
    return `${String(hh).padStart(2, "0")}:${String(mm).padStart(2, "0")}:${String(ss).padStart(2, "0")}`;
}

function parseHHMM(value) {
    const [h, m] = value.split(":").map(Number);
    return h * 3600 + m * 60;
}

let TT = null;
let STOPS = [];
let STOP_BY_ID = new Map();
let STOP_BY_NAME = new Map(); // "Name (id)" -> stopIdx

function el(tag, attrs = {}, children = []) {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) {
        if (k === "class") e.className = v;
        else if (k === "style") e.style.cssText = v;
        else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
        else e.setAttribute(k, v);
    }
    for (const c of [].concat(children)) {
        if (c == null) continue;
        e.appendChild(typeof c === "string" ? document.createTextNode(c) : c);
    }
    return e;
}

function clear(node) {
    while (node.firstChild) node.removeChild(node.firstChild);
}

function parsePicker(value) {
    // Datalist values look like "Name (id)" because two stops can share a
    // name; the parenthesised id is the canonical lookup key.
    const m = value.match(/\(([^()]+)\)\s*$/);
    if (m) return m[1];
    return value;
}

async function bootstrap() {
    document.getElementById("stats").textContent = "fetching WASM…";
    await init();
    document.getElementById("stats").textContent = "fetching feed…";
    const resp = await fetch(FEED_URL);
    if (!resp.ok) throw new Error(`fetch ${FEED_URL}: ${resp.status}`);
    const bytes = new Uint8Array(await resp.arrayBuffer());
    document.getElementById("stats").textContent = "building timetable…";
    TT = new VultureTimetable(bytes, SERVICE_DATE);
    STOPS = TT.allStops();
    for (const s of STOPS) {
        STOP_BY_ID.set(s.id, s);
        STOP_BY_NAME.set(`${s.name} (${s.id})`, s);
    }
    populatePicker();
    setSensibleDefaults();

    document.getElementById("stats").textContent =
        `${TT.nStops()} stops · ${TT.nRoutes()} routes · ready`;

    document.getElementById("simple-form").addEventListener("submit", onSimple);
    document.getElementById("range-form").addEventListener("submit", onRange);
    document.getElementById("walking-form").addEventListener("submit", onWalking);
}

function populatePicker() {
    const dl = document.getElementById("stops");
    clear(dl);
    // Sort by name for usability.
    const sorted = STOPS.slice().sort((a, b) => a.name.localeCompare(b.name));
    for (const s of sorted) {
        // The visible label is "Name (id)"; users typing matches either.
        dl.appendChild(el("option", { value: `${s.name} (${s.id})` }));
    }
}

function setSensibleDefaults() {
    // Pick stops by id from the bundled feed so the demo "just works"
    // on first load. "1" is Dilshad Garden (Red Line east terminus),
    // "44" is Vishwavidyalaya (Yellow Line).
    const dilshad = STOP_BY_ID.get("1");
    const vish = STOP_BY_ID.get("44");
    if (dilshad && vish) {
        const a = `${dilshad.name} (${dilshad.id})`;
        const b = `${vish.name} (${vish.id})`;
        document.getElementById("simple-from").value = a;
        document.getElementById("simple-to").value = b;
        document.getElementById("range-from").value = a;
        document.getElementById("range-to").value = b;
    }
    // For the walking-footpaths demo, pick a pair of central stations
    // close enough that walking augmentation has a chance to matter.
    // Rajiv Chowk (id varies; fall back to a sensible default).
    const central = STOPS.find((s) =>
        s.name.toLowerCase().includes("rajiv chowk"),
    );
    const khan = STOPS.find((s) => s.name.toLowerCase().includes("khan market"));
    if (central && khan) {
        document.getElementById("walking-from").value = `${central.name} (${central.id})`;
        document.getElementById("walking-to").value = `${khan.name} (${khan.id})`;
    }
}

function lookupStopFromInput(input) {
    const raw = input.value.trim();
    const byKey = STOP_BY_NAME.get(raw);
    if (byKey) return byKey;
    const id = parsePicker(raw);
    return STOP_BY_ID.get(id);
}

function renderJourney(j, opts = {}) {
    const summary = `${j.legs.length} trip${j.legs.length === 1 ? "" : "s"}`;
    const header = el("div", { class: "journey-header" }, [
        el("span", { class: "arrival" }, `arrives ${fmtTime(j.arrival)}`),
        el("span", { class: "summary" }, summary),
    ]);
    const legs = el("ul", { class: "legs" });
    for (const leg of j.legs) {
        const routeName = TT.routeName(leg.route) || `route ${leg.route_id}`;
        const colour = lineColour(routeName);
        const board = TT.stopName(leg.board_stop) || `stop ${leg.board_stop}`;
        const alight = TT.stopName(leg.alight_stop) || `stop ${leg.alight_stop}`;
        legs.appendChild(
            el("li", { class: "leg" }, [
                el("span", { class: "line-pill", style: `background:${colour}` }, routeName),
                el("div", { class: "stops" }, [
                    el("span", {}, [
                        el("strong", {}, board),
                        el("span", { class: "times" }, ` dep ${fmtTime(leg.depart)}`),
                    ]),
                    el("span", {}, [
                        el("strong", {}, alight),
                        el("span", { class: "times" }, ` arr ${fmtTime(leg.arrive)}`),
                    ]),
                ]),
            ]),
        );
    }
    return el(
        "div",
        { class: "journey" },
        opts.title
            ? [el("h3", {}, opts.title), header, legs]
            : [header, legs],
    );
}

function emptyState(text) {
    return el("div", { class: "empty" }, text);
}

function errorState(text) {
    return el("div", { class: "error" }, text);
}

function timing(elapsed) {
    return el("span", { class: "timing" }, `(${elapsed.toFixed(2)} ms)`);
}

function onSimple(ev) {
    ev.preventDefault();
    const out = document.getElementById("simple-output");
    clear(out);

    const from = lookupStopFromInput(document.getElementById("simple-from"));
    const to = lookupStopFromInput(document.getElementById("simple-to"));
    if (!from || !to) {
        out.appendChild(errorState("Pick stops from the dropdown."));
        return;
    }

    const depart = parseHHMM(document.getElementById("simple-depart").value);
    const max = Number(document.getElementById("simple-max").value || 5);

    const t0 = performance.now();
    const journeys = runArrival(
        TT,
        new Uint32Array([from.idx]),
        new Uint32Array([to.idx]),
        max,
        depart,
        false,
    );
    const elapsed = performance.now() - t0;

    const summary = el("p", { class: "intro" }, [
        `${from.name} → ${to.name}, depart ${fmtTime(depart)}: `,
        `${journeys.length} option${journeys.length === 1 ? "" : "s"}`,
        timing(elapsed),
    ]);
    out.appendChild(summary);

    if (journeys.length === 0) {
        out.appendChild(emptyState("No journey found within the transfer cap."));
        return;
    }
    for (const j of journeys) out.appendChild(renderJourney(j));
}

function onRange(ev) {
    ev.preventDefault();
    const out = document.getElementById("range-output");
    clear(out);

    const from = lookupStopFromInput(document.getElementById("range-from"));
    const to = lookupStopFromInput(document.getElementById("range-to"));
    if (!from || !to) {
        out.appendChild(errorState("Pick stops from the dropdown."));
        return;
    }

    const start = parseHHMM(document.getElementById("range-start").value);
    const end = parseHHMM(document.getElementById("range-end").value);
    const step = Number(document.getElementById("range-step").value);
    if (end <= start) {
        out.appendChild(errorState("Window end must be after start."));
        return;
    }

    const departures = [];
    for (let t = start; t <= end; t += step) departures.push(t);

    const t0 = performance.now();
    const entries = runRange(
        TT,
        from.idx,
        to.idx,
        10,
        new Uint32Array(departures),
        false,
    );
    const elapsed = performance.now() - t0;

    out.appendChild(
        el("p", { class: "intro" }, [
            `${from.name} → ${to.name}, ${departures.length} departures across ${fmtTime(start)}–${fmtTime(end)}: `,
            `${entries.length} kept after Pareto filter`,
            timing(elapsed),
        ]),
    );

    if (entries.length === 0) {
        out.appendChild(emptyState("No journeys in this window."));
        return;
    }

    const tbl = el("table", { class: "range-table" });
    const thead = el("thead", {}, [
        el("tr", {}, [
            el("th", {}, "Depart"),
            el("th", {}, "Arrive"),
            el("th", {}, "Trips"),
            el("th", {}, "First leg"),
        ]),
    ]);
    const tbody = el("tbody");
    // Range entries come back in rRAPTOR's reverse-chronological scan order;
    // sort by depart ascending for a more familiar timetable view.
    const sorted = entries.slice().sort((a, b) => a.depart - b.depart);
    for (const rj of sorted) {
        const j = rj.journey;
        const firstLeg = j.legs[0];
        const firstLegLabel = firstLeg
            ? `${TT.routeName(firstLeg.route) || firstLeg.route_id} from ${TT.stopName(firstLeg.board_stop)}`
            : "—";
        tbody.appendChild(
            el("tr", {}, [
                el("td", {}, fmtTime(rj.depart)),
                el("td", {}, fmtTime(j.arrival)),
                el("td", {}, String(j.legs.length)),
                el("td", {}, firstLegLabel),
            ]),
        );
    }
    tbl.appendChild(thead);
    tbl.appendChild(tbody);
    out.appendChild(tbl);
}

function onWalking(ev) {
    ev.preventDefault();
    const out = document.getElementById("walking-output");
    clear(out);

    const from = lookupStopFromInput(document.getElementById("walking-from"));
    const to = lookupStopFromInput(document.getElementById("walking-to"));
    if (!from || !to) {
        out.appendChild(errorState("Pick stops from the dropdown."));
        return;
    }

    const depart = parseHHMM(document.getElementById("walking-depart").value);
    const dist = Number(document.getElementById("walking-dist").value);

    // Baseline: no walking augmentation.
    TT.resetFootpaths();
    const t0a = performance.now();
    const baseline = runArrival(
        TT,
        new Uint32Array([from.idx]),
        new Uint32Array([to.idx]),
        10,
        depart,
        false,
    );
    const elapsedA = performance.now() - t0a;

    // With walking footpaths.
    let augmented = [];
    let elapsedB = 0;
    if (dist > 0) {
        TT.withWalkingFootpaths(dist, 1.4);
        const t0b = performance.now();
        augmented = runArrival(
            TT,
            new Uint32Array([from.idx]),
            new Uint32Array([to.idx]),
            10,
            depart,
            false,
        );
        elapsedB = performance.now() - t0b;
    }

    const compare = el("div", { class: "compare-grid" });
    const left = el("div", {}, [
        el("h3", {}, ["Baseline ", timing(elapsedA)]),
    ]);
    if (baseline.length === 0) left.appendChild(emptyState("No journey."));
    else for (const j of baseline) left.appendChild(renderJourney(j));
    compare.appendChild(left);

    if (dist > 0) {
        const right = el("div", {}, [
            el("h3", {}, [`with ${dist} m walking footpaths `, timing(elapsedB)]),
        ]);
        if (augmented.length === 0) right.appendChild(emptyState("No journey."));
        else for (const j of augmented) right.appendChild(renderJourney(j));
        compare.appendChild(right);
    } else {
        const right = el("div", {}, [
            el("h3", {}, "Walking off"),
            emptyState("Set a non-zero max distance to see the augmented routing."),
        ]);
        compare.appendChild(right);
    }

    out.appendChild(compare);
}

bootstrap().catch((err) => {
    console.error(err);
    document.getElementById("stats").textContent = `error: ${err.message}`;
});
