#!/bin/sh
set -eu
export TRIXY_DB="${TRIXY_DB:-/tmp/trixy-firebase-carol.db}"
exec cargo run --bin trixy
