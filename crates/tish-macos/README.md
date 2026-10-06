# tish-macos

AppKit host for Tish JSX. Published to npm as **`@tishlang/tish-macos`** — apps `npm install @tishlang/tish-macos` and `import { macos } from "@tishlang/tish-macos"`. (Inside the tish monorepo only, `import … from "tish:macos"` resolves this crate directly — local debug.) See crate docs in `src/lib.rs` and the `examples/` tree.

## Editor: go to definition and hover

Native `window.*` helpers are implemented with Rust closures, not `pub fn` exports, so **`tish-lsp`** resolves them via **`lsp-pragmas.d.tish`** at this repo root (same `// @tish-source` convention as `tish/stdlib/builtins.d.tish`, with an optional `| hover text` suffix).

- **Go to definition** on e.g. `window.innerHeight` jumps to `src/appkit/window_api.rs`.
- **Hover** shows the optional doc line and a **clickable** `file://` link to that Rust location (no compiler checkout required for macos-only sources).

To also jump built-in JS globals (`console`, …) from the main compiler tree, set **`tish.tishlangSourceRoot`** to your **`tish`** repository root (directory that contains `crates/`).

### Quick test layout

1. Open **`tish-macos-dev.code-workspace`** (adds the sibling `tish` folder and sets `tish.tishlangSourceRoot`), **or** open **`examples/kitchen-sink-macos`** alone (folder settings point at `../../../tish` for the compiler repo).
2. Build the language server once: `cargo build -p tishlang_lsp` from the **`tish`** repo (debug binary: `target/debug/tish-lsp`). This repo’s **`.vscode/settings.json`**, **`tish-macos-dev.code-workspace`**, and **`examples/kitchen-sink-macos/.vscode/settings.json`** already set **`tish.languageServerPath`** to that binary via **`${workspaceFolder}`** / **`${workspaceFolder:tish-compiler}`** (expanded by the Tish extension).
3. In `examples/kitchen-sink-macos/src/main.tish`, hover **`innerHeight`** on the `window.innerHeight()` line — you should see the pragma doc and an “Open Rust implementation” link.

## Async on the main thread

On the native backend `await` blocks the thread it runs on, and Tish code must stay on the thread
that started it. In an AppKit app that thread is the main one, so awaiting a fetch there freezes
the UI. Two calls let the app keep going and be called back on the main thread:

- `macos.whenSettled(promise, cb)` waits for any Tish promise (`fetch(url)`, `res.text()`,
  `reader.read()`, …) on a background thread, then calls `cb(value, error)` on the main thread
  (`error` is null when it fulfilled). A non-promise value is passed to `cb` on the next turn.
- `macos.startTimers()` drives the global `setTimeout` / `setInterval` from the main run loop
  (every 32 ms). `macos.run` already does this; call it when your app sets up AppKit itself.

Streaming a response, chunk by chunk as the server sends it:

```tish
fn pump(reader, onChunk, done) {
  macos.whenSettled(reader.read(), (r, err) => {
    if (err !== null || r.done) { done(err); return }
    onChunk(r.value)
    pump(reader, onChunk, done)
  })
}

macos.whenSettled(fetch(url), (res, err) => pump(res.body.getReader(), show, finish))
```

A failed request settles with an error response (`{ ok: false, error }`), like `await fetch`.
See `examples/async-macos`.

## System services

Namespaces on `macos` for macOS services that aren't UI. Callbacks run on the main thread.

| API | What |
|---|---|
| `macos.timeZones.names()` / `.local()` / `.at(id, unix)` / `.byAbbreviation(abbr)` | macOS's time-zone database: ids, the local zone, `{ offset, abbreviation }` at a moment (daylight saving included), a zone for "CET" |
| `macos.dictionary.lookup(word)` | the plain text of each homograph in the system dictionary, first one first |
| `macos.pasteboard.readText()` / `.writeText(text)` / `.watch(cb)` | the general pasteboard's text; `watch` calls `cb({ text, app, appPath })` for each copy, skipping password-manager items, with `app` "" for this app's own writes |
| `macos.workspace.open(target)` / `.reveal(path)` / `.trash(path)` / `.appsFor(path)` / `.openWith(path, app)` | open a URL or path, select a file in Finder, move to the Trash (`{ ok, path, error }`), the apps that can open a file (default first), open with one |
| `macos.watchFolders(paths, latency, cb)` | `cb()` after changes under `paths`, coalesced over `latency` seconds (FSEvents) |
| `macos.onOpenUrl(cb)` | `cb(url)` for each URL macOS opens with the app (a scheme in its Info.plist); call before the run loop starts |

The native backend lowers a bare `x.at(i)` to `Array.prototype.at`, so bind `macos.timeZones.at`
to a name before calling it (`let zoneAt = macos.timeZones.at`).
