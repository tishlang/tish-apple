#!/usr/bin/env bash
# Full path: Rust staticlib → xcodebuild → simctl launch (no Xcode UI).
set -euo pipefail

EXAMPLE="$(cd "$(dirname "$0")/.." && pwd)"
cd "$EXAMPLE"

echo "→ npm run build (Rust staticlib)"
npm run build

echo "→ xcodebuild (Swift shell)"
bash scripts/xcodebuild-sim.sh

echo "→ launch simulator"
exec bash scripts/launch-sim.sh
