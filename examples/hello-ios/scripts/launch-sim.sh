#!/usr/bin/env bash
set -euo pipefail

EXAMPLE="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=../../../scripts/ios-sim-common.sh
source "$EXAMPLE/../../scripts/ios-sim-common.sh"

require_ios_toolchain

IOS_SIM_DEVICE="${IOS_SIM_DEVICE:-iPhone 16}"
IOS_HELLO_BUNDLE="${IOS_HELLO_BUNDLE:-com.tishlang.helloios}"
DERIVED="${IOS_HELLO_DERIVED:-$EXAMPLE/.derivedData}"
SCHEME="${IOS_HELLO_SCHEME:-HelloIos}"
APP="$DERIVED/Build/Products/Debug-iphonesimulator/${SCHEME}.app"

if [[ ! -d "$APP" ]]; then
  echo "App bundle not found at $APP" >&2
  echo "Build first: npm run xcodebuild:sim   (or npm run run)" >&2
  exit 1
fi

UDID="$(resolve_sim_udid)" || {
  echo "No available iOS Simulator. Open Xcode → Settings → Platforms and install one." >&2
  exit 1
}

echo "→ boot / install / launch $IOS_HELLO_BUNDLE on $UDID"
xcrun simctl boot "$UDID" 2>/dev/null || true
open -a Simulator
xcrun simctl install "$UDID" "$APP"
xcrun simctl launch "$UDID" "$IOS_HELLO_BUNDLE"
echo "launched $IOS_HELLO_BUNDLE"
