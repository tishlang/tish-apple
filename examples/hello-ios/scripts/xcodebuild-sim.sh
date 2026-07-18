#!/usr/bin/env bash
set -euo pipefail

EXAMPLE="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=../../../scripts/ios-sim-common.sh
source "$EXAMPLE/../../scripts/ios-sim-common.sh"

require_ios_toolchain

IOS_SIM_DEVICE="${IOS_SIM_DEVICE:-iPhone 16}"
DERIVED="${IOS_HELLO_DERIVED:-$EXAMPLE/.derivedData}"
PROJECT="$EXAMPLE/ios-shell/HelloIos.xcodeproj"
SCHEME="${IOS_HELLO_SCHEME:-HelloIos}"

UDID="$(resolve_sim_udid)" || {
  echo "No available iOS Simulator. Install one via Xcode → Settings → Platforms." >&2
  exit 1
}
SIM_NAME="$(xcrun simctl list devices available -j | IOS_SIM_UDID="$UDID" python3 -c '
import json, os, sys
udid = os.environ["IOS_SIM_UDID"]
for runtime in json.load(sys.stdin).get("devices", {}).values():
    for dev in runtime:
        if dev.get("udid") == udid:
            print(dev.get("name") or "")
            sys.exit(0)
sys.exit(1)
')"

if [[ ! -f "$EXAMPLE/dist/hello-ios.a" ]]; then
  echo "→ npm run build (staticlib missing)"
  (cd "$EXAMPLE" && npm run build)
fi

echo "→ xcodebuild scheme=$SCHEME destination=iOS Simulator,name=$SIM_NAME"
xcodebuild -project "$PROJECT" \
  -scheme "$SCHEME" \
  -destination "platform=iOS Simulator,name=$SIM_NAME" \
  -derivedDataPath "$DERIVED" \
  build

APP="$DERIVED/Build/Products/Debug-iphonesimulator/${SCHEME}.app"
if [[ ! -d "$APP" ]]; then
  echo "expected app at $APP" >&2
  exit 1
fi
echo "built $APP"
