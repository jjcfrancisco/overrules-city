#!/usr/bin/env just --justfile
# overrules · live city. `just` lists the recipes.

set shell := ['bash', '-euo', 'pipefail', '-c']

# Lower Manhattan: west south east north. data/extract.sql uses the same box.
bbox := '-74.0200 40.7000 -73.9700 40.7350'
wasm_target := 'wasm32-unknown-unknown'
# Must match the wasm-bindgen version pinned in Cargo.toml.
wasm_bindgen_version := '0.2.123'
# MapLibre GL JS, served from web/vendor.
maplibre := '5.24.0'

# Apple's `ar` drops non-Mach-O members, which breaks a wasm32 dependency; use LLVM's.
llvm_ar := if os() == 'macos' { `ls -d /opt/homebrew/opt/llvm*/bin/llvm-ar /usr/local/opt/llvm*/bin/llvm-ar 2>/dev/null | head -n1 || true` } else { '' }
wasm_env := if llvm_ar != '' { 'AR_wasm32_unknown_unknown=' + llvm_ar } else { '' }

@_default:
    just --list --unsorted

# ─── run ─────────────────────────────────────────────────────────────────────

# Build everything that's missing and serve at http://localhost:<port>
play port='8790': wasm
    @test -f web/data/area.geojson || just prep
    @test -f web/vendor/fonts.css || just vendor
    @echo "→ http://localhost:{{ port }}"
    python3 -m http.server -d web {{ port }}

# ─── build ───────────────────────────────────────────────────────────────────

# Download Lower Manhattan's road segments from Overture (needs DuckDB)
data:
    mkdir -p data
    duckdb < data/extract.sql

# Turn the segments into the GeoJSON the page loads (web/data/area.geojson)
prep:
    @test -f data/segment.parquet || just data
    mkdir -p web/data
    cargo run --release -q -p overrules-city-prep -- data/segment.parquet web/data/area.geojson {{ bbox }}

# Compile the city to WebAssembly into web/pkg
wasm: _wasm-target _wasm-bindgen
    {{ wasm_env }} cargo build -q -p overrules-city --profile wasm-release --target {{ wasm_target }}
    wasm-bindgen --target web --no-typescript --out-dir web/pkg \
        target/{{ wasm_target }}/wasm-release/overrules_city.wasm

# Download MapLibre GL JS and the fonts into web/vendor
vendor:
    #!/usr/bin/env bash
    set -euo pipefail
    tmp=$(mktemp -d)
    mkdir -p web/vendor/fonts
    (cd "$tmp" && npm pack --silent maplibre-gl@{{ maplibre }} >/dev/null && tar xzf maplibre-gl-*.tgz)
    cp "$tmp"/package/dist/maplibre-gl.{js,css} web/vendor/
    cp "$tmp"/package/LICENSE.txt web/vendor/LICENSE-maplibre.txt
    rm -rf "$tmp"/package
    : > web/vendor/fonts.css
    for spec in "big-shoulders-display:Big Shoulders Display:700 900" "inter:Inter:400 500 600" \
                "ibm-plex-sans:IBM Plex Sans:500 600" "jetbrains-mono:JetBrains Mono:400 500"; do
        IFS=: read -r pkg family weights <<< "$spec"
        (cd "$tmp" && npm pack --silent "@fontsource/$pkg" >/dev/null && mkdir -p "$pkg" && tar xzf fontsource-"$pkg"-*.tgz -C "$pkg")
        cp "$tmp/$pkg/package/LICENSE" "web/vendor/fonts/LICENSE-$pkg.txt"
        for w in $weights; do
            f="$pkg-latin-$w-normal.woff2"
            cp "$tmp/$pkg/package/files/$f" web/vendor/fonts/
            printf "@font-face { font-family: '%s'; font-weight: %s; font-style: normal; font-display: swap; src: url(fonts/%s) format('woff2'); }\n" "$family" "$w" "$f" >> web/vendor/fonts.css
        done
    done
    rm -rf "$tmp"
    echo "maplibre-gl {{ maplibre }} and fonts → web/vendor"

# ─── check ───────────────────────────────────────────────────────────────────

# Native tests: the city runs, the game rules hold, the tour tells the truth
test:
    cargo test --release -q -- --nocapture

# Formatting, lints (native and wasm32) and tests
check: _wasm-target
    cargo fmt --all -- --check
    cargo clippy -q --all-targets -- -D warnings
    {{ wasm_env }} cargo clippy -q -p overrules-city --target {{ wasm_target }} -- -D warnings
    just test

# Remove build outputs (keeps the downloaded data)
clean:
    cargo clean
    rm -rf web/pkg web/data web/vendor

_wasm-target:
    rustup target list --installed | grep -q '^{{ wasm_target }}$' || rustup target add {{ wasm_target }}
    {{ if os() == 'macos' { if llvm_ar == '' { 'echo "wasm32 builds on macOS need llvm-ar: brew install llvm" >&2; exit 1' } else { 'true' } } else { 'true' } }}

_wasm-bindgen:
    #!/usr/bin/env bash
    set -euo pipefail
    have=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || true)
    if [[ "$have" != "{{ wasm_bindgen_version }}" ]]; then
        echo "installing wasm-bindgen-cli {{ wasm_bindgen_version }} (found: ${have:-none})"
        cargo install wasm-bindgen-cli --version {{ wasm_bindgen_version }} --locked
    fi
