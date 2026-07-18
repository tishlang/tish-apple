# multi-window-macos (multi-process)

`app.spawnPeer()` / `macos.spawnPeer()` starts a **second process** running the same binary. Each process has its own `NSApplication`; from Terminal you often see **two Dock icons**.

`MainApp` vs `PeerApp` are **different root components** (selected with `macos.isPeerChild()`), so the two windows are not meant to look like clones of one branchy `App()`.

`postSessionMessage` / `onSessionMessage` use Core Foundation distributed notifications and the shared `TISH_MACOS_SESSION_ID`.

For **one process, multiple windows**, see `examples/multi-window-in-app-macos`.
