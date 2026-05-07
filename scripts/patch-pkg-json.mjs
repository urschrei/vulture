// Post-process vulture-wasm/pkg/package.json so it's npm-publish-ready
// and Node-ESM-friendly. wasm-pack --target web does not write
// "type": "module" or a Node-resolvable entry, so `import init from
// "vulture-wasm"` would fail without these additions.

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const pkgPath = join(here, "..", "vulture-wasm", "pkg", "package.json");

const pkg = JSON.parse(readFileSync(pkgPath, "utf8"));

pkg.type = "module";
pkg.main = "vulture_wasm.js";
pkg.exports = {
    ".": "./vulture_wasm.js",
    "./vulture_wasm_bg.wasm": "./vulture_wasm_bg.wasm",
};
pkg.homepage = "https://urschrei.github.io/vulture/";
pkg.bugs = { url: "https://github.com/urschrei/vulture/issues" };
pkg.keywords = ["gtfs", "routing", "raptor"];

// wasm-pack defaults to ["./snippets/*"] but this crate ships no JS
// snippets, so the entry was dead config. Drop it.
delete pkg.sideEffects;

writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
console.log(`patched ${pkgPath}: + type=module, main, exports, homepage, bugs, keywords; - sideEffects`);
