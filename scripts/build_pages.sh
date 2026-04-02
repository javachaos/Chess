#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PAGES_DIR="$ROOT_DIR/dist/pages"
WASM_TARGET_DIR="$ROOT_DIR/target/wasm32-unknown-unknown/release"
WASM_OUT_DIR="$PAGES_DIR/pkg"
CARGO_BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"
WASM_BINDGEN_BIN="${WASM_BINDGEN_BIN:-}"

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is required for the web build because the wasm target is installed there." >&2
  exit 1
fi

if [[ -z "$WASM_BINDGEN_BIN" ]]; then
  if command -v wasm-bindgen >/dev/null 2>&1; then
    WASM_BINDGEN_BIN="$(command -v wasm-bindgen)"
  elif [[ -x "$CARGO_BIN_DIR/wasm-bindgen" ]]; then
    WASM_BINDGEN_BIN="$CARGO_BIN_DIR/wasm-bindgen"
  fi
fi

if [[ -z "$WASM_BINDGEN_BIN" ]]; then
  echo "Install wasm-bindgen-cli first:" >&2
  echo "  cargo install wasm-bindgen-cli --version 0.2.114 --locked" >&2
  echo "If it is already installed, add $CARGO_BIN_DIR to your PATH." >&2
  exit 1
fi

export RUSTC="${RUSTC:-$(rustup which rustc)}"

rustup run stable cargo build --release --lib --target wasm32-unknown-unknown

mkdir -p "$PAGES_DIR"
cp "$ROOT_DIR/web/index.html" "$ROOT_DIR/web/styles.css" "$ROOT_DIR/web/bootstrap.js" "$ROOT_DIR/web/favicon.svg" "$PAGES_DIR/"

"$WASM_BINDGEN_BIN" \
  --target web \
  --no-typescript \
  --out-dir "$WASM_OUT_DIR" \
  "$WASM_TARGET_DIR/chess_engine.wasm"

touch "$PAGES_DIR/.nojekyll"

echo "Built GitHub Pages site in $PAGES_DIR"
