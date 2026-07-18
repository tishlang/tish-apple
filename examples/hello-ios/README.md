# hello-ios (pure native)

Minimal **tish-apple** UIKit host demo: `ios.run` + host tags. No tish-desktop, no Tauri.

Cross-device BrokerCore / native↔webview parity lives in **tish-desktop**:
`tish-desktop/examples/hello-ios`.

## Quick start

```bash
cd examples/hello-ios
npm install
npm run run          # staticlib → xcodebuild → simctl
```

From tish-apple root: `npm run example:hello-ios`
