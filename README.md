# tish-apple

Reusable Apple-platform UI hosts for [Tish](https://tishlang.com) JSX.

**Release:** [docs/RELEASE.md](docs/RELEASE.md) — conventional commits → prerelease → promote → crates.io + npm (`@tishlang/tish-macos`, `@tishlang/tish-ios`).

## Layout

```
crates/
  tish-apple-common/   shared tag/style/handler helpers
  tish-macos/          AppKit host (npm: `@tishlang/tish-macos`)
  tish-ios/            UIKit host (npm: `@tishlang/tish-ios`)
examples/
  hello-macos/         pure-native AppKit
  hello-ios/           pure-native UIKit (`ios.run`)
  kitchen-sink-macos/
  sidebar-macos/
  multi-window-macos/
  multi-window-in-app-macos/
  ai-messenger-macos/
```

Pure-native examples stay here (plan). Cross-device BrokerCore / native↔webview parity is in **tish-desktop** (`examples/hello-ios`). Hosts may path-depend on the standalone `tishlang_broker` crate under that umbrella — not on `tishlang_desktop` / the language repo.

## Clean

Remove all `dist/`, `target/`, and `node_modules/` directories under this repo:

```bash
npm run clean
```

## macOS examples

From any example directory:

```bash
cd examples/hello-macos   # or kitchen-sink-macos, sidebar-macos, …
npm install
npm run build
npm run dev
```

Or from the host crate (scripts still live under `crates/tish-macos`):

```bash
cd crates/tish-macos
npm run build:hello-macos
npm run dev:hello-macos
```

`examples/*` consume the **published npm packages**: they depend on `@tishlang/tish` (CLI) + `@tishlang/tish-macos` and `import { macos } from "@tishlang/tish-macos"` — the same flow as any app outside this repo. For local host development, the copies under `crates/tish-macos/examples/` import `tish:macos` instead, which the compiler walk-up resolves to `tish-apple/crates/tish-macos` (local debug only — never the published package).

| Example | Notes |
|---------|--------|
| `hello-macos` | Minimal `macos.run` window |
| `kitchen-sink-macos` | Widget + window API exercise |
| `sidebar-macos` | `NSSplitViewController` + SF Symbols toolbar |
| `multi-window-macos` | `spawnPeer` + session bus |
| `multi-window-in-app-macos` | `macos.openWindow` (one Dock icon) |
| `ai-messenger-macos` | AIM-style multi-window mock |

## hello-ios (pure native)

```bash
cd examples/hello-ios && npm install && npm run run
# or: npm run example:hello-ios
```

Host-only UIKit hello. For BrokerCore + native/webview surface switcher, use **tish-desktop** `examples/hello-ios`.

## iOS staticlib build

From any Tish project with `@tishlang/tish-ios` as a dependency:

```bash
tish build --target native --native-backend rust \
  --crate-type staticlib --ios-triple aarch64-apple-ios-sim \
  src/main.tish -o dist/my-app
```

The Xcode shell calls `tish_ios_launch()` exported from the staticlib.
