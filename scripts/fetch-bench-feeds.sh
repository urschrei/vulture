#!/usr/bin/env bash
# Fetch GTFS feeds used by the cross-city benchmark example into
# `aux/external/`. The directory is gitignored.
#
# Each feed is downloaded once and reused; rerun this script to refresh
# the snapshot. Total download is ~300 MB at the time of writing.
#
# License attribution (every feed is redistributable but requires
# attribution where redistributed downstream – keep these intact in any
# derived work):
#
# - Helsinki HSL feed:    CC-BY 4.0 (HSL / Helsingin seudun liikenne)
# - Berlin/Brandenburg VBB: CC-BY 3.0 DE (Verkehrsverbund Berlin-Brandenburg)
# - Paris IDFM feed:      Licence Mobilités (Île-de-France Mobilités)
#
# See docs/cross-city-benchmarks.md for the canonical attribution list
# and the sources of these URLs.

set -euo pipefail

cd "$(dirname "$0")/.."
mkdir -p aux/external

declare -a FEEDS=(
    "helsinki|http://dev.hsl.fi/gtfs/hsl.zip"
    "berlin|https://www.vbb.de/vbbgtfs"
    "paris|https://eu.ftp.opendatasoft.com/stif/GTFS/IDFM-gtfs.zip"
)

for entry in "${FEEDS[@]}"; do
    name="${entry%%|*}"
    url="${entry#*|}"
    target="aux/external/${name}.zip"
    if [[ -f "$target" ]]; then
        echo "skip $name (already at $target)"
        continue
    fi
    echo "fetch $name ..."
    curl -fsSL --retry 3 -o "$target" "$url"
    bytes=$(wc -c < "$target")
    echo "  -> $target ($bytes bytes)"
done

echo
echo "Done. Run the bench with:"
echo "    cargo run --release --example cross-city-bench"
