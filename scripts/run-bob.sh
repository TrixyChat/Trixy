#!/bin/sh
set -eu
export TRIXY_DB="${TRIXY_DB:-/tmp/trixy-firebase-bob.db}"
exec cargo run --bin trixy
