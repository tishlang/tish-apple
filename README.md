# tish-apple

Reusable Apple-platform UI hosts for [Tish](https://tishlang.com) JSX.

## Layout

```
crates/
  tish-apple-common/   shared tag/style/handler helpers
  tish-macos/          AppKit host (`tish:macos`)
  tish-ios/            UIKit host (`tish:ios`)
examples/
  hello-macos/         AppKit sample
  hello-ios/           staticlib + Xcode shell (simulator)
```

## hello-macos

```bash
cd crates/tish-macos
npm run build:hello-macos
npm run dev:hello-macos
```

Or from the workspace example:

```bash
cd examples/hello-macos
npm install
npm run build
./dist/hello-macos
```

Apps under `~/Projects/tish/` resolve `tish:macos` via `tish-apple/crates/tish-macos` (compiler walk-up). Outside that tree, use a `file:` dependency on `crates/tish-macos`.

## hello-ios

```bash
cd examples/hello-ios
npm install
npm run build          # → dist/hello-ios.a
open ios-shell/HelloIos.xcodeproj
```

Or from jukebox: `just build-hello-ios` / `just dev-ios-sim`.

## iOS staticlib build

From any Tish project with `tish-ios` as a dependency:

```bash
tish build --target native --native-backend rust \
  --crate-type staticlib --ios-triple aarch64-apple-ios-sim \
  src/main.tish -o dist/my-app
```

The Xcode shell calls `tish_ios_launch()` exported from the staticlib.
