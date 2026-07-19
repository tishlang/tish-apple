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
