#!/usr/bin/env bash
# Shared helpers for building / launching iOS Simulator apps (no Xcode.app UI).
# shellcheck disable=SC2034

resolve_sim_udid() {
  local prefer="${IOS_SIM_DEVICE:-}"
  # Prefer exact name, else first available iPhone*, else first available device.
  IOS_SIM_DEVICE="$prefer" python3 -c '
import json, os, sys
prefer = os.environ.get("IOS_SIM_DEVICE") or ""
data = json.load(sys.stdin)
iphones, any_devs = [], []
for runtime in data.get("devices", {}).values():
    for dev in runtime:
        if not dev.get("isAvailable"):
            continue
        name = dev.get("name") or ""
        udid = dev.get("udid")
        if not udid:
            continue
        any_devs.append((name, udid))
        if name.startswith("iPhone"):
            iphones.append((name, udid))
        if prefer and name == prefer:
            print(udid)
            sys.exit(0)
if prefer:
    print(f"Simulator {prefer!r} not found; falling back.", file=sys.stderr)
if iphones:
    print(f"Using simulator: {iphones[0][0]}", file=sys.stderr)
    print(iphones[0][1])
    sys.exit(0)
if any_devs:
    print(f"Using simulator: {any_devs[0][0]}", file=sys.stderr)
    print(any_devs[0][1])
    sys.exit(0)
sys.exit(1)
' < <(xcrun simctl list devices available -j)
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required tool: $1" >&2
    exit 1
  fi
}

require_ios_toolchain() {
  require_cmd xcodebuild
  require_cmd xcrun
  require_cmd python3
  if ! xcrun simctl help >/dev/null 2>&1; then
    echo "xcrun simctl unavailable — install Xcode + Command Line Tools" >&2
    exit 1
  fi
}
