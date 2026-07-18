# multi-window-in-app-macos (same process)

`macos.openWindow(DetailApp, { title })` creates a **new `NSWindow`**, a new Tish **root id**, and mounts `DetailApp` there. The main window keeps running `MainApp`. Both share **one** `NSApplication` (typically **one Dock tile** when run from the command line).

The return value includes **`show`**, **`runEventLoop`**, **`spawnPeer`**, and **`nsWindow`** (methods like `setTitle` / `focus` bound to that `NSWindow`).

Application-wide APIs: **`app.runEventLoop`**, **`app.spawnPeer`**, **`app.activate`**. Global **`window.*`** refers to the **current** root (the tree that is rendering or whose UI fired the callback).
