#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust/Cargo is required. Install Rust from https://rustup.rs and run this script again." >&2
  exit 1
fi

if ! cargo packager --version >/dev/null 2>&1; then
  echo "Installing cargo-packager..."
  cargo install cargo-packager --version 0.11.8 --locked
fi

rm -rf dist
cargo packager --release --formats app dmg

echo
echo "Installer build complete. Look in: $ROOT_DIR/dist"
