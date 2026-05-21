# Sidebar (macOS)

Demonstrates **`macos.run(App, { … })`** with a root **`SidebarWindow`** (two panes), **SF Symbols** on `<image symbol={true} …>`, a **`pick_list`** for mailbox selection (one `onChange` so the compiler does not move the setter into multiple closures), and the system **toolbar sidebar toggle** (and tracking separator) from AppKit. Toolbar chrome is controlled with **`sidebarToolbarToggle`** / **`sidebarTrackingSeparator`** on **`SidebarWindow`** props. The toggle icon is **not** Tish UI — it is **`NSToolbarToggleSidebarItemIdentifier`**.

Build: `npm install && npm run build` (from this directory).

Run: `./dist/sidebar-macos`

## Manual checklist

- [ ] Toolbar shows sidebar toggle and divider-aligned separator; click toggles collapse/expand.
- [ ] Dragging the split divider updates layout; detail `window.innerWidth` / `innerHeight` match the detail pane.
- [ ] Sidebar shows SF Symbol images and a pick list; changing the list updates the detail copy.
