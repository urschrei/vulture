#!/usr/bin/env bash
# Build the in-browser vulture demo. Outputs to docs/demo/.
#
# Usage:
#   ./scripts/build-demo.sh           # build, copy assets
#   ./scripts/build-demo.sh --serve   # build + spin up a local http server
#
# Requires: rust toolchain, wasm-pack, python3 (for --serve).

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
demo_dir="$repo_root/docs/demo"

cd "$repo_root"

if ! command -v wasm-pack >/dev/null 2>&1; then
    echo "wasm-pack not found. Install with: cargo install wasm-pack" >&2
    exit 1
fi

echo "Building vulture-wasm (release, target=web)..."
wasm-pack build vulture-wasm --target web --release --out-dir "$demo_dir/pkg"

echo "Copying GTFS feed (Delhi Metro)..."
cp "$repo_root/aux/dmrc_gtfs.zip" "$demo_dir/dmrc_gtfs.zip"

# Wasm size summary so a regression in bundle size is visible.
wasm_path="$demo_dir/pkg/vulture_wasm_bg.wasm"
raw_size="$(wc -c <"$wasm_path")"
gz_size="$(gzip -c "$wasm_path" | wc -c)"
printf "wasm: %s raw, %s gzipped\n" \
    "$(awk "BEGIN { printf \"%.0f KB\", $raw_size / 1024 }")" \
    "$(awk "BEGIN { printf \"%.0f KB\", $gz_size / 1024 }")"

echo "Demo built into $demo_dir"

if [[ "${1:-}" == "--serve" ]]; then
    port="${PORT:-8765}"
    echo "Serving on http://localhost:$port (Ctrl-C to stop)..."
    python3 -m http.server "$port" --directory "$demo_dir"
fi
