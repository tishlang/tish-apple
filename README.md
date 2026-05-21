# tish-apple

Reusable Apple-platform UI hosts for [Tish](https://tishlang.com) JSX.

## Layout

```
crates/
  tish-apple-common/   shared tag/style/handler helpers
  tish-macos/          AppKit host (`tish:macos`)
  tish-ios/            UIKit host (`tish:ios`)
examples/
  hello-ios/           staticlib + Xcode shell (simulator)
```

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
