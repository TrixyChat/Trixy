#!/bin/sh
set -eu
rm -f /tmp/trixy-firebase-alice.db /tmp/trixy-firebase-alice.db-shm /tmp/trixy-firebase-alice.db-wal
rm -f /tmp/trixy-firebase-bob.db /tmp/trixy-firebase-bob.db-shm /tmp/trixy-firebase-bob.db-wal
rm -f /tmp/trixy-firebase-carol.db /tmp/trixy-firebase-carol.db-shm /tmp/trixy-firebase-carol.db-wal
echo "Local Alice/Bob/Carol test databases removed."
