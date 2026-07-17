//! AppKit host for Tish JSX (`tish:macos`). Host setup follows `tish-iced` / `tish-native-ui`
//! (`install_host_for_root` / `native_create_root` + `render` in Rust); vnode mapping mirrors `tish-floem` tag coverage.
//!
//! ## `macos.run`
//! **`macos.run(App, options?)`** installs a deferred host: the first committed root vnode selects the window shell
//! — **`sidebar_window`** / **`SidebarWindow`** (two panes) vs default single-pane content. The shell’s **`NSWindow`**
//! is **non-opaque** with a **clear** background so **`VisualEffect` `behindWindow`** can sample the desktop (stronger vibrancy).
//! Toolbar chrome for the
//! sidebar shell comes from **`SidebarWindow` props** (`sidebarToolbarToggle`, `sidebarTrackingSeparator`,
//! optional `sidebarMinWidth` / `sidebarMinimumThickness`, `sidebarPreferredWidthFraction`, `appearance` / `nsAppearance`,
//! **`titlebarToolbar`** / **`windowToolbar`** (default **`true`**: show normal `toolbarItems` / legacy sidebar toggle in the unified titlebar; set **`false`** to clear toolbar **items** but keep **`NSWindowToolbarStyle::Unified`** so split sidebar layout stays correct, and use a transparent / hidden window title so in-content toolbars can reach the top),
//! `toolbarItems` (strings plus `{ symbol, id, label? }` for SF Symbol toolbar buttons), and `onToolbarAction` / `on_toolbar_action`).
//! Optional **`Window`** / **`macos_window`** wraps a single child for an explicit content shell root.
//! Run options may set **`appearance`** / **`nsAppearance`** (e.g. `"darkAqua"`) for the initial window.
//!
//! Returns **`null`** when `autoRunEventLoop` is true (default); otherwise returns **`{ show, runEventLoop, spawnPeer, nsWindow }`**.
//! **`macos.openWindow(App, options?)`** always opens an **additional** in-process window (new Tish root); use **`nsWindow`** on the returned handle for that window’s API.
//! Application-wide helpers live on **`app`** (**`app.runEventLoop`**, **`app.spawnPeer`**, **`app.activate`**). Global **`window.*`** targets the **current** Tish root (the tree that is rendering or handling the callback).
//! Sidebar shells also expose **`window.toggleSidebar()`** (maps to **`NSSplitViewController` `toggleSidebar:`**) and **`window.sidebarCollapsed()`** (whether the first split item, the sidebar pane, is collapsed).
//! Extra **processes**: **`spawnPeer`** (same binary, separate `NSApplication`) and **`postSessionMessage` / `onSessionMessage`** (distributed notifications).
//!
//! ## Floem tag parity (native AppKit)
//! **Supported (native AppKit):** layout (`scrollable`, `space`, `rule`, **`column`** / **`Column`** / **`VStack`**
//! (vertical stack; when the parent passes a bounded height—e.g. a **`Split`** pane—the **last** child receives
//! the remaining height so **`ScrollView` `height="fill"`** fills under fixed headers), **`ZStack`** / **`zstack`**
//! (**`fillIndex`** / **`fill_child`**: which child gets bounded **`height="fill"`**; default **`0`**. A two-child
//! **`ZStack`** of **`GroupedTable`** + **`VisualEffect`** is special-cased: the host reparents the effect into the
//! **`NSScrollView`** so frosted chrome blurs scrolling rows without covering the vertical scroller—pair with
//! **`documentTopInset`** on **`GroupedTable`**),
//! `row` with optional `weights` and
//! **`columnWidths`** `[px, null|0|\"flex\", …]` for fixed + flex columns; on **`Row`**, `padding*` insets
//! content inside the layer when the row has `backgroundColor` / `borderRadius` / `onClick` (full-width chrome).
//! Shell rows also honor **`alignItems`** / **`verticalAlign`**: `"center"` | `"end"` / `"bottom"` (default `"start"`)
//! for cross-axis placement, and **`height` / `h`** as a minimum outer height when larger than content.
//! Other nodes still use padding as outer inset. `text` (optional
//! `wrap`), `button` (optional **`bezelStyle`**: **`toolbar`**, **`glass`**/**`liquidGlass`**, **`circular`**, **`borderless`**/**`shadowless`** → flat **`NSShadowlessSquare`** (not **`accessoryBarAction`**’s rounded plate), …; symbol-only **`toolbar`**/**`glass`** default to bordered capsule / glass — **`bordered: false`** for flat icons), `textinput` / `password` with `onChange`, `checkbox`, `toggler`, `slider`, `progress_bar`,
//! `pick_list`, `radio`, `image` (optional `symbol` / `sfSymbol` / `sf_symbol` for SF Symbols), `tooltip`,
//! `list` / `table` (scrollable multi-line text), `text_editor` (`NSTextView` + `value` / `onChange`), **`markdown_text`** (read-only `NSTextView` + Markdown → attributed string), `tabs`, **`split`** (`orientation`: **`horizontal`** = two columns; **`vertical`** / **`stacked`** = two rows), `webview`, root **`sidebar_window`** (two children:
//! sidebar + detail), plus **`window.*`**, **`useState`**, and **`useMemo`** on the module object.
//! **`VisualEffect`** / **`visual_effect`**: **`material`** (string or raw Apple integer), **`blendingMode`** /
//! **`blending`** (`withinWindow` | `behindWindow`), **`state`** (`active` | `inactive` | `followsWindow`),
//! **`emphasized`**, optional **`scrollerGutterRight`** (when a **`VisualEffect`** is a **`ZStack` overlay** above a
//! scroll view, narrows the effect; on **`ScrollView`** / **`list`** / **`text_editor`** / **`markdown_text`** it may set
//! **`NSScrollView` `contentInsets.right`** so row backgrounds do not bleed into an overlay scroller track).
//! **`GroupedTable`** instead uses a **legacy** vertical scroller and lays out rows at **clip width minus scroller lane**
//! (resize-safe; embedded **`VisualEffect`** headers use the same lane), merged **`style`** / layer fields (`backgroundColor`, `borderRadius`, `opacity`,
//! `borderWidth`, `borderColor`), and **`appearance`** / **`nsAppearance`** (e.g. `darkAqua`).
//! **Diagnostics**: set environment variable **`TISH_MACOS_WARN_UNKNOWN_PROPS=1`** to log unknown top-level props (after `style` merge) for **`Row`**, **`ScrollView`**, and **`VisualEffect`** once per `(tag, key)` per process.
//! **`ScrollView`** / **`GroupedTable`**: optional numeric **`documentTopInset`** (aliases **`contentInsetTop`**) adds
//! empty space at the **top of the document** so the first row can scroll under frosted chrome; when
//! **`documentTopInset` > 0**, **`NSScrollView` `scrollerInsets.top`** matches it so the vertical scroller track is not
//! clipped under a top overlay. For **`GroupedTable`**, a sibling **`VisualEffect`** in a two-child **`ZStack`** is
//! reparented into the scroll view for vibrancy over list rows (see **`ZStack`** above).
//! **`GroupedTable`** / **`grouped_table`**: same flipped **`NSScrollView`** stack as **`ScrollView`**, with
//! **`section`** / **`Section`** headers (by default **`NSVisualEffectView`** **`HeaderView`** material; set
//! **`vibrantSectionHeaders`** **`false`** for plain text headers) and row content; section headers scroll with the list
//! (**not** pinned to the window). **`section`** alone is laid out like **`column`**.
//! **Still thin vs web:** styling/CSS, rich text, menus beyond Quit, etc.

#[cfg(target_os = "macos")]
mod appkit;

#[cfg(target_os = "macos")]
pub use appkit::macos_object;

#[cfg(not(target_os = "macos"))]
use std::sync::Arc;
#[cfg(not(target_os = "macos"))]
use tishlang_core::{ObjectMap, PropMap, Value};

/// Non-macOS stub: `macos.run` logs; `window.*`, `useState`, and `useMemo` resolve for CI `cargo check`.
#[cfg(not(target_os = "macos"))]
pub fn macos_object() -> Value {
    let noop = Value::native(|_a: &[Value]| Value::Null);
    let run = Value::native(|_a: &[Value]| {
        eprintln!("tishlang_macos: macos.run is only available on macOS.");
        Value::Null
    });
    let open_window = Value::native(|_a: &[Value]| {
        eprintln!("tishlang_macos: macos.openWindow is only available on macOS.");
        let show = Value::native(|_a: &[Value]| Value::Null);
        let run_el = Value::native(|_a: &[Value]| Value::Null);
        let spawn = Value::native(|_a: &[Value]| Value::Null);
        let mut ns = ObjectMap::default();
        ns.insert(Arc::from("setTitle"), noop.clone());
        ns.insert(Arc::from("focus"), noop.clone());
        ns.insert(Arc::from("snapToRegion"), noop.clone());
        let mut h = ObjectMap::default();
        h.insert(Arc::from("show"), show);
        h.insert(Arc::from("runEventLoop"), run_el);
        h.insert(Arc::from("spawnPeer"), spawn);
        h.insert(
            Arc::from("nsWindow"),
            Value::object(ns),
        );
        Value::object(h)
    });
    let run_event_loop = Value::native(|_a: &[Value]| {
        eprintln!("tishlang_macos: macos.runEventLoop is only available on macOS.");
        Value::Null
    });
    let mut app_map = ObjectMap::default();
    app_map.insert(
        Arc::from("runEventLoop"),
        Value::native(|_a: &[Value]| Value::Null),
    );
    app_map.insert(Arc::from("spawnPeer"), noop.clone());
    app_map.insert(Arc::from("activate"), noop.clone());
    let mut macos_inner = ObjectMap::default();
    macos_inner.insert(Arc::from("run"), run);
    macos_inner.insert(Arc::from("openWindow"), open_window);
    macos_inner.insert(Arc::from("runEventLoop"), run_event_loop);
    macos_inner.insert(Arc::from("postSessionMessage"), noop.clone());
    macos_inner.insert(Arc::from("onSessionMessage"), noop.clone());
    macos_inner.insert(Arc::from("spawnPeer"), noop.clone());
    macos_inner.insert(
        Arc::from("isPeerChild"),
        Value::native(|_a: &[Value]| Value::Bool(false)),
    );
    macos_inner.insert(
        Arc::from("preferencesGet"),
        Value::native(|_a: &[Value]| Value::Null),
    );
    macos_inner.insert(Arc::from("preferencesSet"), noop.clone());
    macos_inner.insert(Arc::from("playNamedSound"), noop.clone());
    let macos_val = Value::object(macos_inner);

    let zero = Value::native(|_a: &[Value]| Value::Number(0.0));
    let empty_title = Value::native(|_a: &[Value]| {
        Value::String("".into())
    });
    let mut win = ObjectMap::default();
    win.insert(Arc::from("title"), empty_title);
    win.insert(Arc::from("setTitle"), noop.clone());
    win.insert(Arc::from("innerWidth"), zero.clone());
    win.insert(Arc::from("innerHeight"), zero.clone());
    win.insert(Arc::from("setContentSize"), noop.clone());
    win.insert(Arc::from("setMinContentSize"), noop.clone());
    win.insert(Arc::from("setMaxContentSize"), noop.clone());
    win.insert(Arc::from("minimize"), noop.clone());
    win.insert(Arc::from("zoom"), noop.clone());
    win.insert(Arc::from("close"), noop.clone());
    win.insert(Arc::from("focus"), noop.clone());
    win.insert(Arc::from("snapToRegion"), noop.clone());
    win.insert(Arc::from("toggleSidebar"), noop.clone());
    win.insert(
        Arc::from("sidebarCollapsed"),
        Value::native(|_a: &[Value]| Value::Bool(false)),
    );
    let window_val = Value::object(win);

    let mut root = ObjectMap::default();
    root.insert(Arc::from("macos"), macos_val);
    root.insert(
        Arc::from("app"),
        Value::object(app_map),
    );
    root.insert(Arc::from("window"), window_val);
    root.insert(
        Arc::from("useState"),
        Value::native(tishlang_ui::runtime::native_use_state),
    );
    root.insert(
        Arc::from("useMemo"),
        Value::native(tishlang_ui::runtime::native_use_memo),
    );
    root.insert(
        Arc::from("useEffect"),
        Value::native(tishlang_ui::runtime::native_use_effect),
    );
    root.insert(Arc::from("Window"), noop.clone());
    root.insert(Arc::from("SidebarWindow"), noop);
    root.insert(
        Arc::from("postSessionMessage"),
        Value::native(|_a: &[Value]| Value::Null),
    );
    root.insert(
        Arc::from("onSessionMessage"),
        Value::native(|_a: &[Value]| Value::Null),
    );
    Value::object(root)
}
