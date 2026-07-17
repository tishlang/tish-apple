//! AppKit host: `macos.run`, vnode commit, `window` API.

mod build;
mod prop_warn;
mod scroll_chrome_embed;
mod grouped_table;
mod markdown_view;
mod prefs;
mod deferred_host;
mod style;
mod flipped;
mod handlers;
mod window_delegate;
mod patch;
mod router;
mod sidebar_host;
pub(crate) mod session_bus;
mod text_delegate;
mod text_view_delegate;
mod toolbar_delegate;
mod window_api;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use block2::RcBlock;
use core::ptr::NonNull;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSApplication, NSApplicationActivationPolicy, NSAutoresizingMaskOptions, NSBackingStoreType,
    NSColor, NSMenu, NSMenuItem, NSSplitViewController, NSSplitViewItem, NSToolbar, NSView,
    NSViewController, NSWindow, NSWindowStyleMask,
};
use objc2_core_foundation::{CGPoint, CGSize};
use objc2_foundation::{NSRect, NSString, NSTimer};
use tishlang_core::{ObjectMap, PropMap, Value};
use dispatch2::DispatchQueue;
use tishlang_ui::runtime::{
    alloc_root_id, install_host_for_root, native_create_root, with_host_for_root, Host, RootId,
    LEGACY_ROOT_ID,
};
use tishlang_ui::ui_h;

pub use build::BuildCtx;

pub(super) use tish_apple_common::tag::canonical_host_tag;

fn propmap_to_object_map(pm: &PropMap) -> ObjectMap {
    pm.iter()
        .map(|(k, v)| (std::sync::Arc::clone(k), v.clone()))
        .collect()
}

pub use flipped::{FlippedRootView, FlippedSplitPaneRootView};

use build::{commit_root_into, window_shell_effective_props};
use deferred_host::DeferredMacosHost;
use handlers::{
    register_root_window, register_sidebar_split_view_controller, set_detail_metrics_for_root,
    window_for_root, MACOS_MAIN_WINDOW,
};
use router::MacosControlRouter;
use sidebar_host::MacosSidebarHost;
use session_bus::{
    ensure_session_id, on_session_message, post_session_message, spawn_peer_process,
};
use text_delegate::TextFieldDelegate;
use text_view_delegate::TextViewDelegate;
use toolbar_delegate::TishToolbarDelegate;
use window_api::{app_object, ns_window_object_for_root, window_object};
use window_delegate::TishWindowDelegate;

/// Semi-transparent layer tints on split pane roots for the sidebar shell. Set to `false` when done
/// debugging layout vs. missing views.
const DEBUG_TINT_SPLIT_PANE_ROOTS: bool = false;

fn debug_tint_view_layer(view: &NSView, red: f64, green: f64, blue: f64, alpha: f64) {
    if !DEBUG_TINT_SPLIT_PANE_ROOTS {
        return;
    }
    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
        let color = NSColor::colorWithSRGBRed_green_blue_alpha(red, green, blue, alpha);
        let cg = color.CGColor();
        layer.setBackgroundColor(Some(&*cg));
    }
}

/// Toolbar chrome options (also accepted on `SidebarWindow` vnode props).
fn run_with_sidebar_toolbar_chrome_options(value: Option<&Value>) -> (bool, bool) {
    let Some(Value::Object(o)) = value else {
        return (true, true);
    };
    let m = &o.borrow().strings;
    (
        optional_bool_prop(
            m,
            &["sidebarToolbarToggle", "sidebar_toggle", "showSidebarToolbarToggle"],
            true,
        ),
        optional_bool_prop(
            m,
            &[
                "sidebarTrackingSeparator",
                "sidebar_tracking_separator",
                "showSidebarTrackingSeparator",
            ],
            true,
        ),
    )
}

fn optional_bool_prop(m: &PropMap, keys: &[&str], default: bool) -> bool {
    for k in keys {
        if let Some(v) = m.get(*k) {
            return match v {
                Value::Bool(b) => *b,
                Value::Number(n) => *n != 0.0,
                Value::Null => false,
                _ => default,
            };
        }
    }
    default
}

fn root_is_sidebar_vnode(v: &Value) -> bool {
    match v {
        Value::Object(o) => match o.borrow().strings.get("tag") {
            Some(Value::String(s)) => {
                let t = s.as_str();
                t == "sidebar_window" || t == "SidebarWindow"
            }
            _ => false,
        },
        _ => false,
    }
}

fn toolbar_chrome_from_sidebar_vnode(v: &Value) -> (bool, bool) {
    if !root_is_sidebar_vnode(v) {
        return (true, true);
    }
    let props_val = match v {
        Value::Object(o) => Value::object(propmap_to_object_map(&build::vnode_props(&o.borrow().strings))),
        _ => Value::Null,
    };
    run_with_sidebar_toolbar_chrome_options(Some(&props_val))
}

thread_local! {
    static SUPPRESS_LAYOUT_WIDTH_NOTIFY: Cell<bool> = Cell::new(false);
    /// Host is applying a width-driven relayout (ignore nested layout notifications).
    static IN_HOST_RELAYOUT: Cell<bool> = Cell::new(false);
    /// Coalesced relayouts requested during AppKit `layout` (avoid main-queue reentrancy).
    static PENDING_HOST_RELAYOUT: RefCell<HashMap<RootId, f64>> = RefCell::new(HashMap::new());
    static RELAYOUT_TIMER_SCHEDULED: Cell<bool> = Cell::new(false);
    static LAST_NOTIFIED_SIZE: RefCell<HashMap<RootId, (f64, f64)>> = RefCell::new(HashMap::new());
}

fn queue_host_relayout_after_layout(root_id: RootId, width: f64) {
    let already_pending = PENDING_HOST_RELAYOUT.with(|p| {
        let mut m = p.borrow_mut();
        if m.contains_key(&root_id) {
            return true;
        }
        m.insert(root_id, width);
        false
    });
    if already_pending || RELAYOUT_TIMER_SCHEDULED.replace(true) {
        return;
    }
    let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
        RELAYOUT_TIMER_SCHEDULED.set(false);
        drain_pending_host_relayout();
    });
    let _timer = unsafe {
        NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.0, false, &*block)
    };
    drop(_timer);
}

fn drain_pending_host_relayout() {
    let pending: Vec<(RootId, f64)> = PENDING_HOST_RELAYOUT.with(|p| {
        let mut m = p.borrow_mut();
        m.drain().collect()
    });
    for (root_id, width) in pending {
        let _ = with_host_for_root(root_id, |host| host.content_width_changed(width));
    }
}

pub(super) fn set_suppress_layout_notify(suppress: bool) {
    SUPPRESS_LAYOUT_WIDTH_NOTIFY.set(suppress);
}

pub(super) fn notify_root_layout_changed(root_id: RootId, width: f64, height: f64) {
    if SUPPRESS_LAYOUT_WIDTH_NOTIFY.get() || IN_HOST_RELAYOUT.get() {
        return;
    }
    if width < 32.0 || height < 32.0 {
        return;
    }
    let (pw, ph) = LAST_NOTIFIED_SIZE.with(|m| {
        m.borrow()
            .get(&root_id)
            .copied()
            .unwrap_or((f64::NAN, f64::NAN))
    });
    if !pw.is_nan()
        && (pw - width).abs() < 0.5
        && (ph - height).abs() < 0.5
    {
        return;
    }
    LAST_NOTIFIED_SIZE.with(|m| {
        m.borrow_mut().insert(root_id, (width, height));
    });
    let _ = (root_id, width);
}

pub(super) fn notify_split_pane_layout_changed(root_id: RootId) {
    if SUPPRESS_LAYOUT_WIDTH_NOTIFY.get() || IN_HOST_RELAYOUT.get() {
        return;
    }
    let _ = root_id;
}

pub struct MacosHost {
    /// Kept alive: `NSWindow` does not retain its delegate — must be dropped **after** `window`.
    window_delegate: Retained<TishWindowDelegate>,
    window: Retained<NSWindow>,
    /// Dropped before `window` so we release our retain while the window/split hierarchy still holds the view.
    root: Retained<FlippedRootView>,
    ctx: BuildCtx,
    last_vnode: Value,
}

impl MacosHost {
    fn content_width(&self) -> f64 {
        let mut w = self
            .window
            .contentView()
            .map(|v| v.bounds().size.width)
            .unwrap_or(640.0) as f64;
        if w < 32.0 {
            w = 640.0;
        }
        w
    }

    /// Same basis as [`Self::content_width`]: the window content view’s `bounds`.
    /// This must match `FlippedRootView::layout` → `notify_root_layout_changed` so `height={"fill"}`
    /// scroll views use the same height as the laid-out content view. Using `contentLayoutRect`
    /// here while layout reports full `bounds` desynced the main `NSScrollView` from the window
    /// and led to overlapping scroll indicators and jumpy relayout.
    fn content_height(&self) -> f64 {
        let mut h = self
            .window
            .contentView()
            .map(|v| v.bounds().size.height)
            .unwrap_or(480.0) as f64;
        if h < 32.0 {
            h = 480.0;
        }
        h
    }

    fn relayout_with_width(&mut self, _width_from_callback: f64) {
        if matches!(self.last_vnode, Value::Null) {
            return;
        }
        // Always use the window’s content width. Nested views used to call this with document
        // bounds and could squeeze the entire UI to a narrow strip.
        let w = self.content_width();
        let vh = self.content_height();
        let v = self.last_vnode.clone();
        set_suppress_layout_notify(true);
        build::commit_root_into_with_layout(&v, &self.root, w, vh, &self.ctx, Some(&v), false);
        set_suppress_layout_notify(false);
    }

    fn detach_native_actions(&mut self) {
        // Do not call `window.setDelegate(None)` here: after close, AppKit may leave `NSWindow` in a
        // state where messaging it crashes (`objc_msgSend` → `setDelegate:`); dropping our
        // `Retained<NSWindow>` when the host is released tears down the delegate link safely.
        self.root.disconnect_tish_root_routing();
        let root_ns: &NSView =
            unsafe { &*std::ptr::from_ref(&*self.root).cast::<NSView>() };
        build::detach_appkit_control_hooks_under(root_ns);
        // Do not call `setContentView(None)` here: `windowWillClose:` can overlap
        // `NSWindowTransformAnimation`; clearing the content view early tears down CALayers while
        // AppKit still releases animation state → crash in `objc_release` on the next CA commit.
    }
}

impl Host for MacosHost {
    fn commit_root(&mut self, vnode: &Value) {
        let w = self.content_width();
        let vh = self.content_height();
        let prev = if matches!(self.last_vnode, Value::Null) {
            None
        } else {
            Some(&self.last_vnode)
        };
        set_suppress_layout_notify(true);
        commit_root_into(vnode, &self.root, w, vh, &self.ctx, prev);
        set_suppress_layout_notify(false);
        self.last_vnode = vnode.clone();
        let shell = window_shell_effective_props(vnode);
        self.window_delegate
            .sync_from_props(&shell, self.window.as_ref());
        if let Some(ref s) = build::appearance_string_from_props(&shell) {
            if !s.is_empty() {
                apply_window_appearance_to_window(self.window.as_ref(), Some(s.as_str()));
            }
        }
    }

    fn content_width_changed(&mut self, width: f64) {
        if IN_HOST_RELAYOUT.get() {
            return;
        }
        IN_HOST_RELAYOUT.set(true);
        self.relayout_with_width(width);
        IN_HOST_RELAYOUT.set(false);
    }


    fn after_window_shown(&mut self) {
        self.window_delegate.fire_on_open();
    }
}

pub(super) enum MacosRealHost {
    Content(MacosHost),
    Sidebar(MacosSidebarHost),
}

impl Host for MacosRealHost {
    fn commit_root(&mut self, vnode: &Value) {
        match self {
            MacosRealHost::Content(h) => h.commit_root(vnode),
            MacosRealHost::Sidebar(h) => h.commit_root(vnode),
        }
    }

    fn content_width_changed(&mut self, width: f64) {
        match self {
            MacosRealHost::Content(h) => h.content_width_changed(width),
            MacosRealHost::Sidebar(h) => h.content_width_changed(width),
        }
    }

    fn after_window_shown(&mut self) {
        match self {
            MacosRealHost::Content(h) => h.after_window_shown(),
            MacosRealHost::Sidebar(h) => h.after_window_shown(),
        }
    }

    fn detach_native_actions(&mut self) {
        match self {
            MacosRealHost::Content(h) => h.detach_native_actions(),
            MacosRealHost::Sidebar(h) => h.detach_native_actions(),
        }
    }
}

thread_local! {
    static MACOS_APP_SETUP_DONE: Cell<bool> = Cell::new(false);
}

#[derive(Clone, Copy)]
enum OpenLayout {
    Content,
    Sidebar,
}

fn install_main_menu(mtm: MainThreadMarker, app: &NSApplication) {
    let menubar = NSMenu::new(mtm);
    let app_menu_item = NSMenuItem::new(mtm);
    menubar.addItem(&app_menu_item);
    app.setMainMenu(Some(&menubar));

    let app_menu = NSMenu::new(mtm);
    let quit_title = NSString::from_str("Quit");
    let quit_key = NSString::from_str("q");
    let quit_item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &quit_title,
            Some(sel!(terminate:)),
            &quit_key,
        )
    };
    unsafe {
        quit_item.setTarget(Some(app));
    }
    app_menu.addItem(&quit_item);
    app_menu_item.setSubmenu(Some(&app_menu));
}

/// Pump `setTimeout` / `setInterval` on the **main run loop** (~32ms), without a dedicated `std::thread`.
fn install_timer_drain_pump() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
        tishlang_runtime::drain_timers();
        drain_pending_host_relayout();
    });
    // `scheduledTimerWithTimeInterval:repeats:block:` schedules on the current run loop, which
    // retains the timer; dropping our `Retained` is safe after this returns.
    let _timer = unsafe {
        NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.032, true, &*block)
    };
    drop(_timer);
}

fn ensure_macos_application_ready(mtm: MainThreadMarker) {
    MACOS_APP_SETUP_DONE.with(|done| {
        if done.get() {
            return;
        }
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        install_main_menu(mtm, &app);
        install_timer_drain_pump();
        done.set(true);
    });
}

fn appearance_string_from_open_options(opt: Option<&Value>) -> Option<String> {
    let Value::Object(o) = opt? else {
        return None;
    };
    let m = &o.borrow().strings;
    build::appearance_string_from_props(m)
}

fn ns_appearance_named(raw: &str) -> Option<Retained<NSAppearance>> {
    let lower = raw.to_ascii_lowercase();
    match lower.as_str() {
        "darkaqua" | "dark" => unsafe { NSAppearance::appearanceNamed(NSAppearanceNameDarkAqua) },
        "aqua" | "light" => unsafe { NSAppearance::appearanceNamed(NSAppearanceNameAqua) },
        _ => NSAppearance::appearanceNamed(&NSString::from_str(raw)),
    }
}

/// Apply `NSAppearance` from a Tish string (`darkAqua`, full Apple name, …). Used by `macos.run` and shell props.
pub(super) fn apply_window_appearance_to_window(window: &NSWindow, name: Option<&str>) {
    let Some(raw) = name.map(str::trim) else {
        return;
    };
    if raw.is_empty() {
        return;
    }
    if let Some(ref a) = ns_appearance_named(raw) {
        NSAppearanceCustomization::setAppearance(window, Some(a));
    }
}

/// Per-view appearance override (`appearance` / `nsAppearance` on vnode or inside `style`).
pub(super) fn apply_view_appearance_from_props(view: &NSView, props: &PropMap) {
    let Some(raw) = build::appearance_string_from_props(props) else {
        return;
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return;
    }
    if let Some(ref a) = ns_appearance_named(raw) {
        NSAppearanceCustomization::setAppearance(view, Some(a));
    }
}

fn title_from_options_value(opt: Option<&Value>, default: &str) -> String {
    let Some(Value::Object(o)) = opt else {
        return default.to_string();
    };
    let m = &o.borrow().strings;
    for k in ["title", "windowTitle"] {
        if let Some(Value::String(s)) = m.get(k) {
            return s.to_string();
        }
    }
    default.to_string()
}

fn title_from_open_options(args: &[Value], default: &str) -> String {
    title_from_options_value(args.get(1), default)
}

fn open_layout_from_opts(args: &[Value], default: OpenLayout) -> OpenLayout {
    let Some(Value::Object(o)) = args.get(1) else {
        return default;
    };
    let m = &o.borrow().strings;
    match m.get("layout") {
        Some(Value::String(s)) if s.as_str() == "sidebar" => OpenLayout::Sidebar,
        Some(Value::String(s)) if s.as_str() == "content" => OpenLayout::Content,
        _ => default,
    }
}

fn create_content_host(
    mtm: MainThreadMarker,
    title: &str,
    root_id: RootId,
    open_opts: Option<&Value>,
) -> (Retained<NSWindow>, MacosRealHost) {
    let frame = NSRect::new(
        CGPoint::new(80.0, 80.0),
        CGSize::new(720.0, 520.0),
    );
    let style = NSWindowStyleMask::Closable
        | NSWindowStyleMask::Miniaturizable
        | NSWindowStyleMask::Resizable
        | NSWindowStyleMask::Titled;
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            style,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    window.setTitle(&NSString::from_str(title));
    window.center();
    if let Some(s) = appearance_string_from_open_options(open_opts) {
        if !s.is_empty() {
            apply_window_appearance_to_window(&window, Some(s.as_str()));
        }
    }
    // `Retained<NSWindow>` is our ownership; default `isReleasedWhenClosed` is often YES, so
    // closing the sheet/window releases the object while we still hold a reference — later
    // `objc_release` in `Retained::drop` faults. Cocoa apps that keep an owning pointer call this.
    unsafe {
        window.setReleasedWhenClosed(false);
    }

    let root = FlippedRootView::new(
        mtm,
        NSRect::new(CGPoint::ZERO, CGSize::new(720.0, 520.0)),
        root_id,
    );
    root.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable
            | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    window.setContentView(Some(&root));

    register_root_window(root_id, window.clone());
    let window_delegate = TishWindowDelegate::new(mtm, root_id);
    window.setDelegate(Some(ProtocolObject::from_ref(&*window_delegate)));

    let router = MacosControlRouter::new(mtm);
    let text_delegate = TextFieldDelegate::new(mtm);
    let text_view_delegate = TextViewDelegate::new(mtm);
    let ctx = BuildCtx {
        mtm,
        router,
        text_delegate,
        text_view_delegate,
        root_id,
    };

    let win_out = window.clone();
    let host = MacosHost {
        window_delegate,
        window,
        root,
        ctx,
        last_vnode: Value::Null,
    };
    (win_out, MacosRealHost::Content(host))
}

fn create_sidebar_host(
    mtm: MainThreadMarker,
    title: &str,
    chrome: (bool, bool),
    root_id: RootId,
    open_opts: Option<&Value>,
) -> (Retained<NSWindow>, MacosRealHost) {
    let (show_sidebar_toggle, show_sidebar_tracking_separator) = chrome;
    let frame = NSRect::new(
        CGPoint::new(80.0, 80.0),
        CGSize::new(900.0, 560.0),
    );
    let style = NSWindowStyleMask::Closable
        | NSWindowStyleMask::Miniaturizable
        | NSWindowStyleMask::Resizable
        | NSWindowStyleMask::Titled
        | NSWindowStyleMask::FullSizeContentView;
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            style,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    window.setTitle(&NSString::from_str(title));
    window.center();
    if let Some(s) = appearance_string_from_open_options(open_opts) {
        if !s.is_empty() {
            apply_window_appearance_to_window(&window, Some(s.as_str()));
        }
    }
    // Non-opaque window + clear fill so `NSVisualEffectView` `behindWindow` can sample the desktop
    // (opaque windows read as flat grey behind sidebar vibrancy).
    window.setOpaque(false);
    let clear = NSColor::clearColor();
    window.setBackgroundColor(Some(&clear));
    unsafe {
        window.setReleasedWhenClosed(false);
    }

    let tb_id = NSString::from_str("TishSidebarToolbar");
    let toolbar = NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), &tb_id);
    let toolbar_delegate = TishToolbarDelegate::new_legacy(
        mtm,
        show_sidebar_toggle,
        show_sidebar_tracking_separator,
    );
    toolbar.setDelegate(Some(ProtocolObject::from_ref(&*toolbar_delegate)));

    let split_vc = NSSplitViewController::new(mtm);
    let sidebar_vc = NSViewController::new(mtm);
    let detail_vc = NSViewController::new(mtm);

    let zero = NSRect::new(CGPoint::ZERO, CGSize::new(200.0, 400.0));
    let sidebar_root = FlippedSplitPaneRootView::new(mtm, zero, root_id);
    sidebar_root.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable
            | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    let detail_root = FlippedSplitPaneRootView::new(mtm, zero, root_id);
    detail_root.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable
            | NSAutoresizingMaskOptions::ViewHeightSizable,
    );

    let sidebar_view: &NSView = unsafe { &*std::ptr::from_ref(&*sidebar_root).cast() };
    let detail_view: &NSView = unsafe { &*std::ptr::from_ref(&*detail_root).cast() };
    debug_tint_view_layer(sidebar_view, 0.95, 0.2, 0.15, 0.42);
    debug_tint_view_layer(detail_view, 0.15, 0.35, 0.95, 0.42);
    sidebar_vc.setView(sidebar_view);
    detail_vc.setView(detail_view);

    let sidebar_item = NSSplitViewItem::sidebarWithViewController(&sidebar_vc);
    // Required with `FullSizeContentView`: otherwise the sidebar sits below the titlebar and picks
    // up inline “card” styling (wrong position + corner radius vs Notes-style full-height sidebar).
    sidebar_item.setAllowsFullHeightLayout(true);
    let detail_item = NSSplitViewItem::splitViewItemWithViewController(&detail_vc);
    split_vc.addSplitViewItem(&sidebar_item);
    split_vc.addSplitViewItem(&detail_item);

    register_sidebar_split_view_controller(root_id, split_vc.clone());

    window.setContentViewController(Some(&split_vc));

    register_root_window(root_id, window.clone());
    set_detail_metrics_for_root(root_id, Retained::as_ptr(&detail_root) as usize);
    let window_delegate = TishWindowDelegate::new(mtm, root_id);
    window.setDelegate(Some(ProtocolObject::from_ref(&*window_delegate)));

    let router = MacosControlRouter::new(mtm);
    let text_delegate = TextFieldDelegate::new(mtm);
    let text_view_delegate = TextViewDelegate::new(mtm);
    let ctx = BuildCtx {
        mtm,
        router,
        text_delegate,
        text_view_delegate,
        root_id,
    };

    let win_out = window.clone();
    let host = MacosSidebarHost::new(
        split_vc,
        win_out.clone(),
        window_delegate,
        toolbar_delegate,
        toolbar,
        (show_sidebar_toggle, show_sidebar_tracking_separator),
        sidebar_root,
        detail_root,
        ctx,
    );
    (win_out, MacosRealHost::Sidebar(host))
}

fn build_host_for_open_layout(
    args: &[Value],
    layout: OpenLayout,
    root_id: RootId,
) -> Option<(Retained<NSWindow>, MacosRealHost)> {
    let mtm = MainThreadMarker::new().expect("macos window API must run on the main thread");
    ensure_macos_application_ready(mtm);
    let title = title_from_open_options(args, "Tish");
    match layout {
        OpenLayout::Content => Some(create_content_host(mtm, &title, root_id, args.get(1))),
        OpenLayout::Sidebar => {
            let chrome = run_with_sidebar_toolbar_chrome_options(args.get(1));
            Some(create_sidebar_host(
                mtm,
                &title,
                chrome,
                root_id,
                args.get(1),
            ))
        }
    }
}

pub(super) fn bootstrap_macos_real_host(
    vnode: &Value,
    open_options: Option<&Value>,
    root_id: RootId,
) -> Option<MacosRealHost> {
    let mtm = MainThreadMarker::new().expect("macos window API must run on the main thread");
    ensure_macos_application_ready(mtm);
    let title = title_from_options_value(open_options, "Tish");
    if root_is_sidebar_vnode(vnode) {
        let chrome = toolbar_chrome_from_sidebar_vnode(vnode);
        Some(create_sidebar_host(mtm, &title, chrome, root_id, open_options).1)
    } else {
        Some(create_content_host(mtm, &title, root_id, open_options).1)
    }
}

/// Build window + host + first render for `root_id`. Does not show the window or start `NSApplication.run`.
fn macos_prepare_window_for_root(
    args: &[Value],
    layout: OpenLayout,
    root_id: RootId,
) -> Option<()> {
    let app_fn = args.first().cloned()?;
    set_detail_metrics_for_root(root_id, 0);
    let (_win_out, host) = build_host_for_open_layout(args, layout, root_id)?;
    install_host_for_root(root_id, Box::new(host));
    let root_obj = native_create_root(&[Value::Number(root_id as f64)]);
    if let Value::Object(obj) = root_obj {
        if let Some(Value::Function(render)) = obj.borrow().strings.get("render") {
            render.call(&[app_fn]);
        }
    }
    Some(())
}

fn parse_run_options(v: Option<&Value>) -> (bool, bool) {
    let Some(Value::Object(o)) = v else {
        return (true, true);
    };
    let m = &o.borrow().strings;
    let auto_show = match m.get("autoShow") {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => *n != 0.0,
        Some(Value::Null) => false,
        None => true,
        _ => true,
    };
    let auto_run = match m.get("autoRunEventLoop") {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => *n != 0.0,
        Some(Value::Null) => false,
        None => true,
        _ => true,
    };
    (auto_show, auto_run)
}

fn macos_shell_element(tag: &'static str, args: &[Value]) -> Value {
    let props = args.first().cloned().unwrap_or(Value::Null);
    let (props_obj, children_arg) = match &props {
        Value::Object(o) => {
            let m = &o.borrow().strings;
            let ch = m
                .get("children")
                .cloned()
                .unwrap_or_else(|| Value::array(vec![]));
            let mut map = m.clone();
            map.remove(&Arc::from("children"));
            (Value::object(propmap_to_object_map(&map)), ch)
        }
        _ => (
            Value::Null,
            Value::array(vec![]),
        ),
    };
    let norm_children = match children_arg {
        Value::Array(a) => Value::Array(a),
        Value::Null => Value::array(vec![]),
        other => Value::array(vec![other]),
    };
    ui_h(&[
        Value::String(tag.into()),
        props_obj,
        norm_children,
    ])
}

fn macos_shell_sidebar_window(args: &[Value]) -> Value {
    macos_shell_element("sidebar_window", args)
}

fn macos_shell_window(args: &[Value]) -> Value {
    macos_shell_element("macos_window", args)
}

fn macos_show_window(win: &NSWindow, root_id: RootId) {
    let mtm = MainThreadMarker::new().expect("macos.show needs the main thread");
    win.makeKeyAndOrderFront(None);
    NSApplication::sharedApplication(mtm).activate();
    DispatchQueue::main().exec_async(move || {
        let _ = with_host_for_root(root_id, |host| host.after_window_shown());
    });
}

fn macos_run_event_loop_inner() {
    let mtm = MainThreadMarker::new().expect("macos.runEventLoop needs the main thread");
    NSApplication::sharedApplication(mtm).run();
}

fn build_run_handle(root_id: RootId) -> Value {
    let rid = root_id;
    let show = Value::native(move |_| {
        let Some(w) = window_for_root(rid) else {
            return Value::Null;
        };
        macos_show_window(w.as_ref(), rid);
        Value::Null
    });
    let run_el = Value::native(|_| {
        macos_run_event_loop_inner();
        Value::Null
    });
    let spawn = Value::native(spawn_peer_process);
    let ns = ns_window_object_for_root(rid);
    let mut m = ObjectMap::default();
    m.insert(Arc::from("show"), show);
    m.insert(Arc::from("runEventLoop"), run_el);
    m.insert(Arc::from("spawnPeer"), spawn);
    m.insert(Arc::from("nsWindow"), ns);
    Value::object(m)
}

fn macos_run(args: &[Value]) -> Value {
    let app_fn = match args.first() {
        Some(f) => f.clone(),
        None => return Value::Null,
    };
    let opt_ref = args.get(1);
    let (auto_show, auto_run_event_loop) = parse_run_options(opt_ref);
    let open_options = opt_ref.cloned();

    ensure_session_id();
    set_detail_metrics_for_root(LEGACY_ROOT_ID, 0);
    let mtm = MainThreadMarker::new().expect("macos.run must run on the main thread");
    ensure_macos_application_ready(mtm);

    install_host_for_root(
        LEGACY_ROOT_ID,
        Box::new(DeferredMacosHost::new(open_options, LEGACY_ROOT_ID)),
    );
    let root_obj = native_create_root(&[Value::Null]);
    if let Value::Object(obj) = root_obj {
        if let Some(Value::Function(render)) = obj.borrow().strings.get("render") {
            render.call(&[app_fn]);
        }
    }

    let has_window = window_for_root(LEGACY_ROOT_ID).is_some()
        || MACOS_MAIN_WINDOW.with(|c| c.borrow().is_some());
    if !has_window {
        return Value::Null;
    }

    let handle = build_run_handle(LEGACY_ROOT_ID);

    if auto_show {
        if let Value::Object(hm) = &handle {
            if let Some(Value::Function(show_fn)) = hm.borrow().strings.get("show") {
                show_fn.call(&[]);
            }
        }
    }

    if auto_run_event_loop {
        macos_run_event_loop_inner();
        Value::Null
    } else {
        handle
    }
}

fn macos_open_window(args: &[Value]) -> Value {
    let root_id = alloc_root_id();
    let layout = open_layout_from_opts(args, OpenLayout::Content);
    if macos_prepare_window_for_root(args, layout, root_id).is_none() {
        return Value::Null;
    }
    build_run_handle(root_id)
}

fn macos_run_event_loop(_: &[Value]) -> Value {
    let mtm = MainThreadMarker::new().expect("macos.runEventLoop needs the main thread");
    NSApplication::sharedApplication(mtm).run();
    Value::Null
}

pub fn macos_object() -> Value {
    let run = Value::native(macos_run);
    let open_window = Value::native(macos_open_window);
    let run_event_loop = Value::native(macos_run_event_loop);
    let post_sess = Value::native(post_session_message);
    let on_sess = Value::native(on_session_message);
    let mut macos_inner = ObjectMap::default();
    macos_inner.insert(Arc::from("run"), run);
    macos_inner.insert(Arc::from("openWindow"), open_window);
    macos_inner.insert(Arc::from("runEventLoop"), run_event_loop);
    macos_inner.insert(Arc::from("postSessionMessage"), post_sess.clone());
    macos_inner.insert(Arc::from("onSessionMessage"), on_sess.clone());
    macos_inner.insert(
        Arc::from("spawnPeer"),
        Value::native(spawn_peer_process),
    );
    macos_inner.insert(
        Arc::from("isPeerChild"),
        Value::native(|_| {
            Value::Bool(std::env::var("TISH_MACOS_CHILD").map(|s| s == "1").unwrap_or(false))
        }),
    );
    macos_inner.insert(
        Arc::from("preferencesGet"),
        Value::native(|a: &[Value]| {
            let key = a.first().map(|v| v.to_display_string()).unwrap_or_default();
            prefs::preference_get(&key)
                .map(|s| Value::String(s.into()))
                .unwrap_or(Value::Null)
        }),
    );
    macos_inner.insert(
        Arc::from("preferencesSet"),
        Value::native(|a: &[Value]| {
            let key = a.first().map(|v| v.to_display_string()).unwrap_or_default();
            let val = a.get(1).map(|v| v.to_display_string()).unwrap_or_default();
            prefs::preference_set(&key, &val);
            Value::Null
        }),
    );
    macos_inner.insert(
        Arc::from("playNamedSound"),
        Value::native(|a: &[Value]| {
            let name = a.first().map(|v| v.to_display_string()).unwrap_or_default();
            prefs::play_named_sound(&name);
            Value::Null
        }),
    );
    let macos_val = Value::object(macos_inner);

    let mut root = ObjectMap::default();
    root.insert(Arc::from("macos"), macos_val);
    root.insert(Arc::from("app"), app_object());
    root.insert(Arc::from("window"), window_object());
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
    root.insert(
        Arc::from("Window"),
        Value::native(macos_shell_window),
    );
    root.insert(
        Arc::from("SidebarWindow"),
        Value::native(macos_shell_sidebar_window),
    );
    root.insert(Arc::from("postSessionMessage"), post_sess);
    root.insert(Arc::from("onSessionMessage"), on_sess);
    Value::object(root)
}
