# Kitchen sink (macOS)

Build: `npm install && npm run build` (from this directory).

Run: `./dist/kitchen-sink-macos`

## Checklist (plan coverage)

- [x] Scaffold + `macos.run` / `window` import
- [x] Layout: `space`, `rule`, `scrollable`, `div` + `padding`
- [x] `window`: title, inner size, minimize, zoom, focus, close
- [x] `textinput` + `onChange`; `password`
- [x] `checkbox`, `toggler`, `slider`, `progress_bar`
- [x] `pick_list`, `radio`
- [x] `image`, wrapping `text`, `tooltip`
- [x] `list` (scrollable multi-line text), `tabs`, `split`, `webview`
- [x] `webview bridge={true}` + `onBridgeInvoke` (`__TISH_APP__.invoke`)

Tick items as you verify manually on a Mac.

## LSP smoke test

**`.vscode/settings.json`** sets **`tish.languageServerPath`** to **`../../../tish/target/debug/tish-lsp`** (via **`${workspaceFolder}`** expansion) and **`tish.tishlangSourceRoot`** to **`../../../tish`**. Run **`cargo build -p tishlang_lsp`** from the **`tish`** repo once, then **`npm install`** here so **`lsp-pragmas.d.tish`** resolves through **`node_modules/tish-macos`**. Reload the editor after changing the extension or binary.
