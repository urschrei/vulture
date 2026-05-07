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
// Delhi route long_names start with the line colour (RED_, YELLOW_, …);
// map those onto sensible signage hues so the pill identifies the line.
// Anything else falls back to the board's amber.
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

const PILL_DEFAULT = "#f0b441"; // matches CSS --amber

// === MapLibre map (dark Carto basemap) ============================
// One shared instance lives at #map and persists across all three
// demo panels. drawJourney() (added in a later step) clears + redraws
// layers on each query.

const MAP_STYLE = "https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json";

const map = new maplibregl.Map({
    container: "map",
    style: MAP_STYLE,
    center: [77.21, 28.64],   // Delhi default; replaced once the feed loads
    zoom: 9,
    attributionControl: false,
});
map.addControl(new maplibregl.NavigationControl({ visualizePitch: false }), "top-right");

// Track when the style is loaded — we can't add layers before then.
let mapReady = false;
map.on("load", () => { mapReady = true; });

// Single popup instance, reused across clicks.
const stopPopup = new maplibregl.Popup({
    closeButton: true,
    closeOnClick: true,
    maxWidth: "320px",
    className: "stop-popup",
});

function setMapStatus(text) {
    const statusEl = document.getElementById("map-status");
    if (statusEl) statusEl.textContent = text;
}

function escapeHtml(s) {
    return String(s)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&#39;");
}

function fmtTimeShort(s) {
    const hh = Math.floor(s / 3600);
    const mm = Math.floor((s % 3600) / 60);
    return `${String(hh).padStart(2, "0")}:${String(mm).padStart(2, "0")}`;
}

function renderStopPopupHtml(name, events) {
    const parts = [`<p class="stop-popup-name">${escapeHtml(name)}</p>`];
    if (!events || (!events.arrivals?.length && !events.departures?.length)) {
        return parts.join("");
    }
    for (const a of events.arrivals ?? []) {
        parts.push(
            `<div class="stop-popup-event">` +
                `<span class="label">Arrive</span>` +
                `<span><span class="time">${fmtTimeShort(a.time)}</span>` +
                ` · <span class="route">${escapeHtml(a.route)}</span></span>` +
                `</div>`,
        );
    }
    for (const d of events.departures ?? []) {
        parts.push(
            `<div class="stop-popup-event">` +
                `<span class="label">Depart</span>` +
                `<span><span class="time">${fmtTimeShort(d.time)}</span>` +
                ` · <span class="route">${escapeHtml(d.route)}</span></span>` +
                `</div>`,
        );
    }
    return parts.join("");
}

const IDLE_SOURCE_ID = "feed-stops";
const IDLE_LAYER_ID = "feed-stops-layer";

// Returns the id of the first symbol layer in the active basemap
// style. Insert overlay layers immediately before this one so route
// lines and stop dots sit below place / station labels rather than
// painting over them.
function firstSymbolLayerId() {
    const style = map.getStyle();
    if (!style) return undefined;
    for (const l of style.layers) {
        if (l.type === "symbol") return l.id;
    }
    return undefined;
}

// Module-scope lookup tables, refreshed on every feed load. Used by
// drawJourney() (added in a later task) and by renderIdleStops below.
let routesByIdx = new Map();
let stopsByIdx = new Map();

// Walking-panel before/after state. Captured by `onWalking` so the
// map header's toggle can flip the rendered journey without re-running
// the queries.
let walkingState = { before: null, after: null, from: null, to: null };

function renderIdleStops(stops) {
    const features = stops
        .filter((s) => typeof s.lat === "number" && typeof s.lon === "number")
        .map((s) => ({
            type: "Feature",
            properties: { id: s.id, name: s.name },
            geometry: { type: "Point", coordinates: [s.lon, s.lat] },
        }));

    const data = { type: "FeatureCollection", features };

    const apply = () => {
        if (map.getSource(IDLE_SOURCE_ID)) {
            map.getSource(IDLE_SOURCE_ID).setData(data);
        } else {
            map.addSource(IDLE_SOURCE_ID, { type: "geojson", data });
            map.addLayer(
                {
                    id: IDLE_LAYER_ID,
                    type: "circle",
                    source: IDLE_SOURCE_ID,
                    paint: {
                        "circle-radius": 2.5,
                        "circle-color": "#3a3f48",
                        "circle-opacity": 0.8,
                    },
                },
                firstSymbolLayerId(),
            );
        }
        if (features.length === 0) return;
        const bbox = features.reduce(
            (b, f) => {
                const [lon, lat] = f.geometry.coordinates;
                return {
                    minLon: Math.min(b.minLon, lon),
                    maxLon: Math.max(b.maxLon, lon),
                    minLat: Math.min(b.minLat, lat),
                    maxLat: Math.max(b.maxLat, lat),
                };
            },
            { minLon: 180, maxLon: -180, minLat: 90, maxLat: -90 },
        );
        if (Number.isFinite(bbox.minLon)) {
            map.fitBounds(
                [
                    [bbox.minLon, bbox.minLat],
                    [bbox.maxLon, bbox.maxLat],
                ],
                { padding: 32, duration: 600 },
            );
        }
    };

    if (mapReady) apply();
    else map.once("load", apply);
}

// === Mode palette (keyed on GTFS route_type) ======================
// Route palette priority: route.route_color (if the feed sets it) ->
// mode palette below -> fallback. Walking transfers are out of scope
// (vulture's journey output doesn't surface them today).
const MODE_PALETTE = {
    0: "#a78bfa",   // Tram / light rail
    1: "#ef4444",   // Subway / metro
    2: "#38bdf8",   // Rail / commuter
    3: "#f0b441",   // Bus
    4: "#34d399",   // Ferry
    5: "#fb7185",   // Cable car
    6: "#fb7185",   // Aerial
    7: "#fb7185",   // Funicular
    11: "#f0b441",  // Trolleybus
};
const FALLBACK_COLOUR = "#c2342b";

function colourForRoute(route) {
    if (route?.route_color) return route.route_color;
    if (route && route.route_type in MODE_PALETTE) return MODE_PALETTE[route.route_type];
    return FALLBACK_COLOUR;
}

const JOURNEY_LINES_ID = "journey-lines";
const JOURNEY_HALO_ID = "journey-halo";
const JOURNEY_STOPS_ID = "journey-stops";

function clearJourney() {
    // Dismiss any open stop popup before tearing the layers down —
    // its anchor stop may not exist on the next journey.
    stopPopup.remove();
    for (const id of [JOURNEY_LINES_ID, JOURNEY_HALO_ID, JOURNEY_STOPS_ID]) {
        if (map.getLayer(id)) map.removeLayer(id);
        if (map.getSource(id)) map.removeSource(id);
    }
}

/**
 * Render one journey on the map. Each leg becomes a coloured line
 * (with off-white halo underneath) plus circle markers at the
 * board/alight stops. Auto-fits the camera to the union of all leg
 * geometries.
 *
 * @param {object} journey - one WasmJourney; legs[*].shape may be null
 * @param {Map<number, object>} routes - routesByIdx lookup
 * @param {Map<number, object>} stops - stopsByIdx lookup (each {lat, lon})
 */
function drawJourney(journey, routes, stops) {
    if (!journey || !journey.legs?.length) return;
    clearJourney();

    const lineFeatures = [];
    const stopFeatures = [];
    const allCoords = [];
    // stop_idx → { name, lon, lat, colour, arrivals: [{time, route}],
    //              departures: [{time, route}] }
    const stopEvents = new Map();

    for (const leg of journey.legs) {
        const route = routes.get(leg.route);
        const colour = colourForRoute(route);

        // Leg geometry: prefer the per-leg shape, fall back to a
        // straight chord between the leg's stops if the feed has no
        // shapes.txt for this trip.
        let coords;
        if (Array.isArray(leg.shape) && leg.shape.length >= 2) {
            coords = leg.shape.map(([lat, lon]) => [lon, lat]);
        } else {
            const board = stops.get(leg.board_stop);
            const alight = stops.get(leg.alight_stop);
            if (!board || !alight) continue;
            coords = [[board.lon, board.lat], [alight.lon, alight.lat]];
        }
        lineFeatures.push({
            type: "Feature",
            properties: { colour },
            geometry: { type: "LineString", coordinates: coords },
        });
        allCoords.push(...coords);

        // Aggregate per-stop events — a transfer stop appears as
        // both an alight (from the previous leg) and a board (for
        // the next leg), so the popup can show both.
        const routeName = route?.name ?? `route ${leg.route}`;
        const board = stops.get(leg.board_stop);
        const alight = stops.get(leg.alight_stop);
        if (board) {
            if (!stopEvents.has(leg.board_stop)) {
                stopEvents.set(leg.board_stop, {
                    name: board.name,
                    lon: board.lon,
                    lat: board.lat,
                    colour,
                    arrivals: [],
                    departures: [],
                });
            }
            stopEvents.get(leg.board_stop).departures.push({
                time: leg.depart,
                route: routeName,
            });
        }
        if (alight) {
            if (!stopEvents.has(leg.alight_stop)) {
                stopEvents.set(leg.alight_stop, {
                    name: alight.name,
                    lon: alight.lon,
                    lat: alight.lat,
                    colour,
                    arrivals: [],
                    departures: [],
                });
            }
            stopEvents.get(leg.alight_stop).arrivals.push({
                time: leg.arrive,
                route: routeName,
            });
        }
    }

    for (const ev of stopEvents.values()) {
        stopFeatures.push({
            type: "Feature",
            properties: {
                name: ev.name,
                colour: ev.colour,
                events: JSON.stringify({
                    arrivals: ev.arrivals,
                    departures: ev.departures,
                }),
            },
            geometry: { type: "Point", coordinates: [ev.lon, ev.lat] },
        });
    }

    const apply = () => {
        // Insert all journey layers immediately below the basemap's
        // first symbol layer so labels (place names, station names)
        // continue to render on top of the route geometry.
        const labelAnchor = firstSymbolLayerId();

        // Halo (drawn first, underneath).
        map.addSource(JOURNEY_HALO_ID, {
            type: "geojson",
            data: { type: "FeatureCollection", features: lineFeatures },
        });
        map.addLayer(
            {
                id: JOURNEY_HALO_ID,
                type: "line",
                source: JOURNEY_HALO_ID,
                paint: {
                    "line-color": "#f1ebde",
                    "line-width": 6,
                    "line-opacity": 0.75,
                },
                layout: { "line-cap": "round", "line-join": "round" },
            },
            labelAnchor,
        );
        // Stroke (over the halo).
        map.addSource(JOURNEY_LINES_ID, {
            type: "geojson",
            data: { type: "FeatureCollection", features: lineFeatures },
        });
        map.addLayer(
            {
                id: JOURNEY_LINES_ID,
                type: "line",
                source: JOURNEY_LINES_ID,
                paint: {
                    "line-color": ["get", "colour"],
                    "line-width": 3,
                },
                layout: { "line-cap": "round", "line-join": "round" },
            },
            labelAnchor,
        );
        // Stop dots.
        map.addSource(JOURNEY_STOPS_ID, {
            type: "geojson",
            data: { type: "FeatureCollection", features: stopFeatures },
        });
        map.addLayer(
            {
                id: JOURNEY_STOPS_ID,
                type: "circle",
                source: JOURNEY_STOPS_ID,
                paint: {
                    "circle-radius": 5,
                    "circle-color": ["get", "colour"],
                    "circle-stroke-color": "#f1ebde",
                    "circle-stroke-width": 1.5,
                },
            },
            labelAnchor,
        );

        // Fit camera to union of all leg coords with padding 64.
        if (allCoords.length) {
            const lons = allCoords.map((c) => c[0]);
            const lats = allCoords.map((c) => c[1]);
            map.fitBounds(
                [
                    [Math.min(...lons), Math.min(...lats)],
                    [Math.max(...lons), Math.max(...lats)],
                ],
                { padding: 64, duration: 600 },
            );
        }
    };

    if (mapReady) apply();
    else map.once("load", apply);
}

// Click-to-inspect popups on idle and journey stops. Handlers are
// keyed on layer id, so they keep working after drawJourney's
// remove/re-add cycle.
function setupStopPopups() {
    const showIdle = (e) => {
        if (!e.features?.length) return;
        const f = e.features[0];
        stopPopup
            .setLngLat(f.geometry.coordinates)
            .setHTML(renderStopPopupHtml(f.properties.name, null))
            .addTo(map);
    };
    const showJourney = (e) => {
        if (!e.features?.length) return;
        const f = e.features[0];
        let events = null;
        try {
            events = JSON.parse(f.properties.events ?? "null");
        } catch { /* fall through to name-only popup */ }
        stopPopup
            .setLngLat(f.geometry.coordinates)
            .setHTML(renderStopPopupHtml(f.properties.name, events))
            .addTo(map);
    };
    map.on("click", IDLE_LAYER_ID, showIdle);
    map.on("click", JOURNEY_STOPS_ID, showJourney);
    for (const layer of [IDLE_LAYER_ID, JOURNEY_STOPS_ID]) {
        map.on("mouseenter", layer, () => {
            map.getCanvas().style.cursor = "pointer";
        });
        map.on("mouseleave", layer, () => {
            map.getCanvas().style.cursor = "";
        });
    }
}
if (mapReady) setupStopPopups();
else map.once("load", setupStopPopups);

function lineColour(routeName) {
    if (!routeName) return PILL_DEFAULT;
    const prefix = routeName.split(/[_\s]/)[0].toUpperCase();
    return LINE_COLOURS[prefix] || PILL_DEFAULT;
}

// GTFS times can exceed 86400 seconds — "27:30:00" is legitimately a
// trip that crosses midnight. Render those with an explicit `+Nd`
// suffix rather than wrapping silently into a meaningless small-hours
// time like "03:30:00", which would make a journey appear to arrive
// before it departs.
function fmtTime(s) {
    const day = Math.floor(s / 86400);
    const t = s % 86400;
    const hh = Math.floor(t / 3600);
    const mm = Math.floor((t % 3600) / 60);
    const ss = t % 60;
    const base = `${String(hh).padStart(2, "0")}:${String(mm).padStart(2, "0")}:${String(ss).padStart(2, "0")}`;
    return day > 0 ? `${base} +${day}d` : base;
}

function parseHHMM(value) {
    const [h, m] = value.split(":").map(Number);
    return h * 3600 + m * 60;
}

let TT = null;
let STOPS = [];
let STOP_BY_ID = new Map();
let STOP_BY_LABEL = new Map(); // picker label -> stop record

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

// Build a picker-friendly label for each stop. Show the bare name if
// it's non-empty and unique within the feed; otherwise append the id
// in parens to disambiguate; fall back to the id alone if there's no
// name at all (some feeds have stop_id "12345" with empty stop_name).
function buildStopLabels(stops) {
    const nameCounts = new Map();
    for (const s of stops) {
        const n = (s.name || "").trim();
        if (!n) continue;
        nameCounts.set(n, (nameCounts.get(n) || 0) + 1);
    }
    const labels = new Map(); // idx -> label
    for (const s of stops) {
        const n = (s.name || "").trim();
        let label;
        if (!n) label = s.id;
        else if (nameCounts.get(n) === 1) label = n;
        else label = `${n} (${s.id})`;
        labels.set(s.idx, label);
    }
    return labels;
}

// Stream a fetch response with progress reporting via ReadableStream.
// Resolves to a single Uint8Array. `onProgress(received, total|0)` is
// called as bytes arrive; `total` is 0 when content-length is missing.
async function fetchWithProgress(url, onProgress) {
    const resp = await fetch(url, { redirect: "follow" });
    if (!resp.ok) throw new Error(`HTTP ${resp.status} ${resp.statusText}`);
    const total = Number(resp.headers.get("content-length")) || 0;
    if (!resp.body) {
        // Older Safari / very small responses: fall back to ArrayBuffer.
        return new Uint8Array(await resp.arrayBuffer());
    }
    const reader = resp.body.getReader();
    const chunks = [];
    let received = 0;
    while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        chunks.push(value);
        received += value.length;
        onProgress(received, total);
    }
    let len = 0;
    for (const c of chunks) len += c.length;
    const out = new Uint8Array(len);
    let off = 0;
    for (const c of chunks) {
        out.set(c, off);
        off += c.length;
    }
    return out;
}

function setProgress(pct, status) {
    const panel = document.getElementById("feed-progress");
    panel.hidden = false;
    panel.classList.remove("error");
    panel.querySelector(".fill").style.width = `${pct}%`;
    panel.querySelector(".status").textContent = status;
}

function setProgressError(message) {
    const panel = document.getElementById("feed-progress");
    panel.hidden = false;
    panel.classList.add("error");
    panel.querySelector(".fill").style.width = "0%";
    panel.querySelector(".status").textContent = message;
}

function hideProgressLater(ms) {
    setTimeout(() => {
        const panel = document.getElementById("feed-progress");
        if (!panel.classList.contains("error")) panel.hidden = true;
    }, ms);
}

// Replace the active timetable with one built from `bytes` + `date`.
// Frees the previous wasm-side handle eagerly to reclaim memory.
async function loadFeed(url, date, displayName) {
    setProgress(0, "Fetching feed…");
    let bytes;
    try {
        bytes = await fetchWithProgress(url, (got, total) => {
            const pct = total ? (100 * got) / total : Math.min(95, got / 1024 / 5);
            const gotMb = (got / (1024 * 1024)).toFixed(1);
            const totalLabel = total
                ? `${(total / (1024 * 1024)).toFixed(1)} MB`
                : "size unknown";
            setProgress(pct, `Downloading ${gotMb} MB / ${totalLabel}…`);
        });
    } catch (err) {
        // Fetch failures from CORS, network, DNS all surface as TypeError.
        const msg = err instanceof TypeError
            ? `Fetch failed (likely CORS or offline): ${err.message}`
            : `Fetch failed: ${err.message}`;
        setProgressError(msg);
        throw err;
    }

    setProgress(100, "Building timetable…");
    let nextTt;
    try {
        nextTt = new VultureTimetable(bytes, date);
    } catch (err) {
        setProgressError(`GTFS parse / build failed: ${err.message ?? err}`);
        throw err;
    }

    // Free the old timetable's wasm-side memory before switching over;
    // wasm-bindgen doesn't auto-free held struct memory when the JS
    // reference drops, so a long demo session would otherwise leak
    // each loaded feed.
    if (TT && typeof TT.free === "function") TT.free();
    TT = nextTt;

    refreshCatalogue(displayName, date);
    setProgress(100, "Done.");
    hideProgressLater(800);
}

function refreshCatalogue(displayName, date) {
    STOPS = TT.allStops();
    STOP_BY_ID = new Map();
    STOP_BY_LABEL = new Map();
    const labels = buildStopLabels(STOPS);
    for (const s of STOPS) {
        s.label = labels.get(s.idx);
        STOP_BY_ID.set(s.id, s);
        STOP_BY_LABEL.set(s.label, s);
    }
    populatePicker();

    const routes = TT.allRoutes();
    routesByIdx = new Map(routes.map((r) => [r.idx, r]));
    stopsByIdx = new Map(STOPS.map((s) => [s.idx, s]));
    renderIdleStops(STOPS);
    setMapStatus("— no query yet —");

    // Drop any walking-panel state from the previous feed — its
    // journeys reference indices into the old catalogue, and the
    // toggle would otherwise stay visible from the prior session.
    walkingState = { before: null, after: null, from: null, to: null };
    const staleToggle = document.getElementById("map-walking-toggle");
    if (staleToggle) staleToggle.hidden = true;

    document.getElementById("feed-name").textContent = displayName;
    document.getElementById("feed-meta").textContent =
        `· ${date} · ${TT.nStops()} stops · ${TT.nRoutes()} routes`;

    // Stop ids change between feeds — wipe pickers and result panels.
    for (const id of [
        "simple-from",
        "simple-to",
        "range-from",
        "range-to",
        "walking-from",
        "walking-to",
    ]) {
        document.getElementById(id).value = "";
    }
    for (const id of ["simple-output", "range-output", "walking-output"]) {
        clear(document.getElementById(id));
    }
    setSensibleDefaults();
}

async function onFeedSubmit(ev) {
    ev.preventDefault();
    const url = document.getElementById("feed-url").value.trim();
    const date = document.getElementById("feed-date-input").value;
    if (!url || !date) return;

    const sug = document.getElementById("feed-suggested");
    let displayName = "Custom feed";
    if (sug.value || sug.options[sug.selectedIndex].dataset.name) {
        displayName = sug.options[sug.selectedIndex].dataset.name || displayName;
    }
    if (sug.value === "bundled:delhi") displayName = "Delhi Metro (bundled)";

    try {
        await loadFeed(resolveSuggested(url), date, displayName);
        // Collapse the loader once we've successfully switched feeds.
        document.getElementById("feed-details").open = false;
    } catch {
        // loadFeed has already surfaced the error in the progress UI.
    }
}

function resolveSuggested(url) {
    if (url === "bundled:delhi") return FEED_URL;
    return url;
}

function onSuggestionChange(ev) {
    const opt = ev.target.options[ev.target.selectedIndex];
    const value = opt.value;
    const date = opt.dataset.date;
    const urlInput = document.getElementById("feed-url");
    const dateInput = document.getElementById("feed-date-input");
    if (value === "") {
        // Custom URL: leave fields for the user to fill.
        urlInput.value = "";
        urlInput.focus();
        return;
    }
    if (value === "bundled:delhi") {
        urlInput.value = FEED_URL;
    } else {
        urlInput.value = value;
    }
    if (date) dateInput.value = date;
}

async function bootstrap() {
    document.getElementById("feed-name").textContent = "fetching WASM…";
    await init();
    // Pre-fill the loader form with the bundled-feed defaults so the
    // first user click is a one-step "load Delhi" rather than a
    // type-the-URL ceremony.
    const sug = document.getElementById("feed-suggested");
    document.getElementById("feed-url").value = FEED_URL;
    document.getElementById("feed-date-input").value = SERVICE_DATE;
    sug.value = "bundled:delhi";

    await loadFeed(FEED_URL, SERVICE_DATE, "Delhi Metro (bundled)");

    document.getElementById("simple-form").addEventListener("submit", onSimple);
    document.getElementById("range-form").addEventListener("submit", onRange);
    document.getElementById("walking-form").addEventListener("submit", onWalking);
    document.getElementById("feed-form").addEventListener("submit", onFeedSubmit);
    sug.addEventListener("change", onSuggestionChange);
}

function populatePicker() {
    const dl = document.getElementById("stops");
    clear(dl);
    // Sort by label so the dropdown is alphabetical.
    const sorted = STOPS.slice().sort((a, b) => a.label.localeCompare(b.label));
    for (const s of sorted) dl.appendChild(el("option", { value: s.label }));
}

function setStopInputs(idsOrPair, fromIds, toIds) {
    // Find the first stop whose id matches one of `fromIds`; same for `toIds`.
    // Returns true iff both were found.
    const fromHit = fromIds.map((i) => STOP_BY_ID.get(i)).find((s) => s);
    const toHit = toIds.map((i) => STOP_BY_ID.get(i)).find((s) => s);
    if (!fromHit || !toHit) return false;
    for (const id of idsOrPair[0]) document.getElementById(id).value = fromHit.label;
    for (const id of idsOrPair[1]) document.getElementById(id).value = toHit.label;
    return true;
}

function setSensibleDefaults() {
    // Simple/range panels: hard-code the recognisable Delhi termini
    // when the bundled feed is loaded. Heuristically picking defaults
    // for arbitrary feeds reliably produces a "no journey" result
    // (disconnected sub-networks, no service at 09:00) which makes
    // the demo look broken — better to leave those fields empty for
    // unknown feeds and let the user drive.
    setStopInputs(
        [["simple-from", "range-from"], ["simple-to", "range-to"]],
        ["1"], // Dilshad Garden
        ["44"], // Vishwavidyalaya
    );

    // Walking-footpaths panel: prefer the Delhi pair (Rajiv Chowk →
    // Khan Market — central, walkable, demonstrates the augmentation
    // changing the route). Fall back to the closest distinct-name
    // pair from the loaded feed: the demo's value is comparing
    // baseline vs augmented routing, and even an empty-vs-empty
    // comparison is informative for a feed where vulture finds no
    // direct transit between two close-by stops.
    const rajiv = STOPS.find((s) =>
        (s.name || "").toLowerCase().includes("rajiv chowk"),
    );
    const khan = STOPS.find((s) =>
        (s.name || "").toLowerCase().includes("khan market"),
    );
    if (rajiv && khan) {
        document.getElementById("walking-from").value = rajiv.label;
        document.getElementById("walking-to").value = khan.label;
    } else {
        const nearPair = findClosestDistinctPair();
        if (nearPair) {
            document.getElementById("walking-from").value = nearPair[0].label;
            document.getElementById("walking-to").value = nearPair[1].label;
        } else {
            document.getElementById("walking-from").value = "";
            document.getElementById("walking-to").value = "";
        }
    }

    updatePlaceholders();
}

// Closest distinct-name pair of stops with coordinates. Sampled to
// keep the O(n²) search cheap on big feeds (Offenburg has 3,300+ stops;
// even sampled to 150 the pair is representative).
function findClosestDistinctPair() {
    const withCoords = STOPS.filter((s) => s.lat != null && s.lon != null);
    if (withCoords.length < 2) return null;
    const sample = withCoords.length <= 150
        ? withCoords
        : withCoords.filter((_, i) => i % Math.ceil(withCoords.length / 150) === 0);
    let bestA = null, bestB = null, bestD = Infinity;
    for (let i = 0; i < sample.length; i++) {
        for (let j = i + 1; j < sample.length; j++) {
            // Distinct names so the journey reads as "A → B" rather
            // than "Mühlhausen → Mühlhausen" for east/west bus shelters.
            if (sample[i].name && sample[i].name === sample[j].name) continue;
            // Cheap squared-distance in (lat, lon * cos(lat)) space —
            // good enough for ranking, doesn't need true Haversine.
            const dLat = sample[i].lat - sample[j].lat;
            const meanLat = (sample[i].lat + sample[j].lat) / 2;
            const dLon = (sample[i].lon - sample[j].lon) * Math.cos(meanLat * Math.PI / 180);
            const d = dLat * dLat + dLon * dLon;
            if (d < bestD && d > 1e-9) {
                bestD = d;
                bestA = sample[i];
                bestB = sample[j];
            }
        }
    }
    return bestA && bestB ? [bestA, bestB] : null;
}

// Update each picker's placeholder to a stop name from the current
// feed, so a fresh user knows what shape of input to type. Two
// distinct stops ~halfway through the alphabetical list give a
// representative hint without revealing trivia.
function updatePlaceholders() {
    const named = STOPS
        .filter((s) => (s.name || "").trim())
        .sort((a, b) => a.label.localeCompare(b.label));
    if (named.length < 2) return;
    const fromHint = named[Math.floor(named.length / 4)].label;
    const toHint = named[Math.floor((named.length * 3) / 4)].label;
    for (const id of ["simple-from", "range-from", "walking-from"]) {
        document.getElementById(id).placeholder = `e.g. ${fromHint}`;
    }
    for (const id of ["simple-to", "range-to", "walking-to"]) {
        document.getElementById(id).placeholder = `e.g. ${toHint}`;
    }
}

function lookupStopFromInput(input) {
    const raw = input.value.trim();
    // Exact picker label match wins.
    const byLabel = STOP_BY_LABEL.get(raw);
    if (byLabel) return byLabel;
    // Failing that, try a parenthesised id at the end ("Name (id)") and
    // finally the bare value as an id — for stops with no name where
    // the label is just the id.
    const m = raw.match(/\(([^()]+)\)\s*$/);
    if (m) {
        const byId = STOP_BY_ID.get(m[1]);
        if (byId) return byId;
    }
    return STOP_BY_ID.get(raw);
}

function renderJourney(j, opts = {}) {
    const summary = `${j.legs.length} LEG${j.legs.length === 1 ? "" : "S"}`;
    const header = el("div", { class: "journey-header" }, [
        el("span", { class: "arrival" }, fmtTime(j.arrival)),
        el("span", { class: "summary" }, summary),
    ]);
    const legs = el("ul", { class: "legs" });
    for (const leg of j.legs) {
        // Prefer the GTFS short_name for the line-pill — it's typically a
        // 1-3 char route designator (RED, M1, N16B) which is the right
        // size for a board pill. Fall back to the long name if short
        // is unavailable.
        const longName = TT.routeName(leg.route) || `route ${leg.route_id}`;
        const pillText = pillLabel(longName);
        const colour = lineColour(longName);
        const board = TT.stopName(leg.board_stop) || `stop ${leg.board_stop}`;
        const alight = TT.stopName(leg.alight_stop) || `stop ${leg.alight_stop}`;
        legs.appendChild(
            el("li", { class: "leg" }, [
                el("span", { class: "line-pill", style: `background:${colour}` }, pillText),
                // The stops grid: 4 cells = (board name | dep time | alight name | arr time).
                el("div", { class: "stops" }, [
                    el("span", {}, board),
                    el("span", { class: "times" }, fmtTime(leg.depart)),
                    el("span", {}, alight),
                    el("span", { class: "times" }, fmtTime(leg.arrive)),
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

// Compress a long route_long_name into a 1-6 char board pill label.
// Many feeds prefix their long names with the line designator
// (e.g. "RED_Dilshad Garden to Rithala", "N16B Heilbronn—Karlsruhe");
// extract that prefix.
function pillLabel(longName) {
    if (!longName) return "·";
    const trimmed = longName.trim();
    // Take everything before the first underscore, space, dash, or em dash.
    const m = trimmed.match(/^[\p{L}0-9]{1,6}/u);
    if (m) return m[0].toUpperCase();
    return trimmed.slice(0, 4).toUpperCase();
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

    const walkingToggle = document.getElementById("map-walking-toggle");
    if (walkingToggle) walkingToggle.hidden = true;

    if (journeys.length === 0) {
        out.appendChild(
            emptyState(
                "No journey found. Try a different stop pair, an earlier or later departure time, or raising the transfer cap.",
            ),
        );
        setMapStatus(`Stop-to-stop · ${from.name} → ${to.name} · no journey found`);
        return;
    }
    for (const j of journeys) out.appendChild(renderJourney(j));

    drawJourney(journeys[0], routesByIdx, stopsByIdx);
    setMapStatus(`Stop-to-stop · ${from.name} → ${to.name}`);
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
        out.appendChild(
            emptyState(
                "No journeys in this window. Try a wider window or a different stop pair.",
            ),
        );
        const walkingToggle = document.getElementById("map-walking-toggle");
        if (walkingToggle) walkingToggle.hidden = true;
        setMapStatus(`Departure window · ${from.name} → ${to.name} · no journeys`);
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
    const rowEls = [];
    for (let i = 0; i < sorted.length; i++) {
        const rj = sorted[i];
        const j = rj.journey;
        const firstLeg = j.legs[0];
        const firstLegLabel = firstLeg
            ? `${TT.routeName(firstLeg.route) || firstLeg.route_id} from ${TT.stopName(firstLeg.board_stop)}`
            : "—";
        const row = el("tr", { class: "range-row" }, [
            el("td", {}, fmtTime(rj.depart)),
            el("td", {}, fmtTime(j.arrival)),
            el("td", {}, String(j.legs.length)),
            el("td", {}, firstLegLabel),
        ]);
        row.addEventListener("click", () => {
            for (const r of rowEls) r.classList.remove("active");
            row.classList.add("active");
            drawJourney(j, routesByIdx, stopsByIdx);
            setMapStatus(
                `Departure window · option ${i + 1} of ${sorted.length} · ${from.name} → ${to.name}`,
            );
        });
        rowEls.push(row);
        tbody.appendChild(row);
    }
    tbl.appendChild(thead);
    tbl.appendChild(tbody);
    out.appendChild(tbl);

    // Default to the first row on a fresh query.
    const walkingToggle = document.getElementById("map-walking-toggle");
    if (walkingToggle) walkingToggle.hidden = true;
    if (rowEls.length > 0) {
        rowEls[0].classList.add("active");
        drawJourney(sorted[0].journey, routesByIdx, stopsByIdx);
        setMapStatus(
            `Departure window · option 1 of ${sorted.length} · ${from.name} → ${to.name}`,
        );
    } else {
        setMapStatus(`Departure window · ${from.name} → ${to.name} · no journeys`);
    }
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

    // Capture both candidate journeys for the map toggle. `before` is
    // always available (baseline runs unconditionally); `after` only
    // when dist > 0.
    walkingState = {
        before: baseline[0] ?? null,
        after: dist > 0 ? (augmented[0] ?? null) : null,
        from,
        to,
    };

    const toggleEl = document.getElementById("map-walking-toggle");
    const toggleBtn = document.getElementById("walking-toggle-btn");
    const haveBoth = !!(walkingState.before && walkingState.after);
    if (toggleEl) toggleEl.hidden = !haveBoth;

    const renderForState = (state) => {
        const j = state === "before" ? walkingState.before : walkingState.after;
        if (j) {
            drawJourney(j, routesByIdx, stopsByIdx);
            setMapStatus(
                `Walking footpaths · ${state} · ${walkingState.from.name} → ${walkingState.to.name}`,
            );
        } else {
            setMapStatus(
                `Walking footpaths · ${state} · ${walkingState.from.name} → ${walkingState.to.name} · no journey`,
            );
        }
        if (toggleBtn) {
            toggleBtn.dataset.state = state;
            toggleBtn.textContent = state.charAt(0).toUpperCase() + state.slice(1);
        }
    };

    if (toggleBtn) {
        toggleBtn.onclick = () => {
            renderForState(toggleBtn.dataset.state === "after" ? "before" : "after");
        };
    }

    // Default render: prefer "after" if available; otherwise show
    // "before" (the only journey we have when dist === 0).
    if (walkingState.after) {
        renderForState("after");
    } else if (walkingState.before) {
        renderForState("before");
    } else {
        setMapStatus(
            `Walking footpaths · ${from.name} → ${to.name} · no journey`,
        );
    }
}

bootstrap().catch((err) => {
    console.error(err);
    document.getElementById("feed-name").textContent = "init failed";
    document.getElementById("feed-meta").textContent = `· ${err.message}`;
});
