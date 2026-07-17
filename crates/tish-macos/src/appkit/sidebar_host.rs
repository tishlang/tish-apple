//! `Host` implementation for `sidebar_window` + `NSSplitViewController` shell.

use std::cell::Cell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2_app_kit::{
    NSSplitViewController, NSSplitViewItemBehavior, NSTitlebarSeparatorStyle, NSToolbar, NSView,
    NSWindow, NSWindowTitleVisibility, NSWindowToolbarStyle,
};
use objc2_core_foundation::CGFloat;

use tishlang_core::{ObjectMap, PropMap, Value};
use tishlang_ui::runtime::Host;

use super::build::{
    appearance_string_from_props, commit_sidebar_window_into, effective_props, props_f64,
    sidebar_window_shell_props, toolbar_entries_from_props, vnode_props,
    window_shell_effective_props, BuildCtx,
};
use super::flipped::FlippedSplitPaneRootView;
use super::handlers::{clear_toolbar_handlers, set_toolbar_action_callback};
use super::set_suppress_layout_notify;
use super::toolbar_delegate::TishToolbarDelegate;
use super::window_delegate::TishWindowDelegate;

/// When absent, the unified titlebar `NSToolbar` is shown (legacy behavior).
fn sidebar_titlebar_toolbar_enabled(eff: &PropMap) -> bool {
    const KEYS: &[&str] = &[
        "titlebarToolbar",
        "titlebar_toolbar",
        "windowToolbar",
        "window_toolbar",
    ];
    for k in KEYS {
        if let Some(v) = eff.get(*k) {
            return match v {
                Value::Bool(b) => *b,
                Value::Number(n) => *n != 0.0,
                Value::Null => false,
                _ => true,
            };
        }
    }
    true
}

/// When `titlebarToolbar` is off we still keep `NSWindowToolbarStyle::Unified` + an attached
/// `NSToolbar` (empty items) so `NSSplitViewController` sidebar chrome stays correct; only the
/// **window title strip** is hidden / transparent so in-content toolbars can meet the top.
fn sync_sidebar_window_titlebar_chrome(window: &NSWindow, show_unified_toolbar: bool) {
    if show_unified_toolbar {
        window.setTitlebarAppearsTransparent(false);
        window.setTitleVisibility(NSWindowTitleVisibility::Visible);
        window.setTitlebarSeparatorStyle(NSTitlebarSeparatorStyle::Automatic);
    } else {
        window.setTitlebarAppearsTransparent(true);
        window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        window.setTitlebarSeparatorStyle(NSTitlebarSeparatorStyle::None);
    }
}

/// With `FullSizeContentView`, keep the system sidebar full-height under the traffic lights. If this
/// is `NO`, AppKit lays the sidebar below the titlebar and uses the “card” chrome (different radius).
fn sync_sidebar_split_layout_for_titlebar(split_vc: &NSSplitViewController) {
    let items = split_vc.splitViewItems();
    if items.count() < 1 {
        return;
    }
    let sidebar_item = items.objectAtIndex(0);
    if sidebar_item.behavior() == NSSplitViewItemBehavior::Sidebar {
        sidebar_item.setAllowsFullHeightLayout(true);
    }
}

/// Optional `<SidebarWindow sidebarMinWidth={…} sidebarPreferredWidthFraction={…} />` → `NSSplitViewItem`.
fn sync_sidebar_split_item_from_root(split_vc: &NSSplitViewController, root: &Value) {
    let Some(shell) = sidebar_window_shell_props(root) else {
        return;
    };
    let items = split_vc.splitViewItems();
    if items.count() < 1 {
        return;
    }
    let item = items.objectAtIndex(0);
    if shell
        .get("sidebarMinWidth")
        .or_else(|| shell.get("sidebarMinimumThickness"))
        .is_some()
    {
        let w = props_f64(
            &shell,
            &["sidebarMinWidth", "sidebarMinimumThickness"],
            200.0,
        );
        item.setMinimumThickness(w as CGFloat);
    }
    if shell.get("sidebarPreferredWidthFraction").is_some() {
        let f = props_f64(&shell, &["sidebarPreferredWidthFraction"], 0.15);
        item.setPreferredThicknessFraction(f as CGFloat);
    }
}

pub struct MacosSidebarHost {
    /// `NSWindow` / `NSToolbar` do not retain delegates — drop these **after** `window` / `split_vc`.
    pub window_delegate: Retained<TishWindowDelegate>,
    pub toolbar_delegate: Retained<TishToolbarDelegate>,
    /// Sidebar windows always use this `NSToolbar`; when `titlebarToolbar={false}` the item list is
    /// cleared but the bar stays attached so unified titlebar layout (and split sidebar) do not break.
    pub toolbar: Retained<NSToolbar>,
    pub window: Retained<NSWindow>,
    #[allow(dead_code)]
    pub split_vc: Retained<NSSplitViewController>,
    pub detail_root: Retained<FlippedSplitPaneRootView>,
    pub sidebar_root: Retained<FlippedSplitPaneRootView>,
    /// When the root vnode omits `toolbar` / `toolbarItems`, restore these defaults.
    toolbar_legacy: (bool, bool),
    pub ctx: BuildCtx,
    pub last_vnode: Value,
    last_quad: Cell<(f64, f64, f64, f64)>,
}

impl MacosSidebarHost {
    pub fn new(
        split_vc: Retained<NSSplitViewController>,
        window: Retained<NSWindow>,
        window_delegate: Retained<TishWindowDelegate>,
        toolbar_delegate: Retained<TishToolbarDelegate>,
        toolbar: Retained<NSToolbar>,
        toolbar_legacy: (bool, bool),
        sidebar_root: Retained<FlippedSplitPaneRootView>,
        detail_root: Retained<FlippedSplitPaneRootView>,
        ctx: BuildCtx,
    ) -> Self {
        Self {
            window_delegate,
            toolbar_delegate,
            toolbar,
            window,
            split_vc,
            detail_root,
            sidebar_root,
            toolbar_legacy,
            ctx,
            last_vnode: Value::Null,
            last_quad: Cell::new((f64::NAN, f64::NAN, f64::NAN, f64::NAN)),
        }
    }

    fn sync_toolbar_from_vnode(&self, vnode: &Value) {
        let Value::Object(o) = vnode else {
            return;
        };
        let m = &o.borrow().strings;
        let is_sidebar = matches!(
            m.get("tag"),
            Some(tishlang_core::Value::String(s)) if {
                let t = s.as_str();
                t == "sidebar_window" || t == "SidebarWindow"
            }
        );
        if !is_sidebar {
            return;
        }
        let raw = vnode_props(m);
        let eff = effective_props(&raw);
        clear_toolbar_handlers(self.ctx.root_id);
        let tb_cb = eff
            .get("onToolbarAction")
            .or_else(|| eff.get("on_toolbar_action"))
            .and_then(|v| match v {
                Value::Function(f) => {
                    let f = f.clone();
                    Some(Rc::new(move |id: String| {
                        let _ = f.call(&[Value::String(id.into())]);
                    }) as Rc<dyn Fn(String)>)
                }
                _ => None,
            });
        set_toolbar_action_callback(self.ctx.root_id, tb_cb);
        let show_titlebar = sidebar_titlebar_toolbar_enabled(&eff);
        if show_titlebar {
            if let Some(entries) = toolbar_entries_from_props(&eff, self.ctx.root_id) {
                self.toolbar_delegate.set_entries(
                    self.ctx.root_id,
                    Some(&self.ctx.router),
                    entries,
                );
            } else {
                self.toolbar_delegate.set_legacy_sidebar_flags(
                    self.ctx.root_id,
                    Some(&self.ctx.router),
                    self.toolbar_legacy.0,
                    self.toolbar_legacy.1,
                );
            }
        } else {
            self.toolbar_delegate.set_entries(
                self.ctx.root_id,
                Some(&self.ctx.router),
                vec![],
            );
        }
        // Never use `NSWindowToolbarStyle::Automatic` here: it switches to classic titlebar layout
        // and breaks `NSSplitViewItem` sidebar position / material (card vs full-height).
        self.window.setToolbar(Some(self.toolbar.as_ref()));
        self.window
            .setToolbarStyle(NSWindowToolbarStyle::Unified);
        self.toolbar_delegate
            .reload_default_items_into_toolbar(self.toolbar.as_ref());
        sync_sidebar_window_titlebar_chrome(self.window.as_ref(), show_titlebar);
        sync_sidebar_split_layout_for_titlebar(self.split_vc.as_ref());
    }

    fn sidebar_as_nsv(&self) -> &NSView {
        unsafe { &*std::ptr::from_ref(&*self.sidebar_root).cast::<NSView>() }
    }

    fn detail_as_nsv(&self) -> &NSView {
        unsafe { &*std::ptr::from_ref(&*self.detail_root).cast::<NSView>() }
    }

    /// Re-commit using current split pane bounds. Needed because `layoutSubtreeIfNeeded` during
    /// `commit_root` runs while layout notifications are suppressed, so `last_quad` can be stale
    /// and `relayout_dual` never runs with the real sizes until something else resizes the window.
    pub(super) fn sync_panes_to_laid_out_bounds(&mut self) {
        if matches!(self.last_vnode, Value::Null) {
            return;
        }
        set_suppress_layout_notify(true);
        let split_root = self.split_vc.view();
        split_root.layoutSubtreeIfNeeded();
        self.sidebar_as_nsv().layoutSubtreeIfNeeded();
        self.detail_as_nsv().layoutSubtreeIfNeeded();
        self.last_quad
            .set((f64::NAN, f64::NAN, f64::NAN, f64::NAN));
        self.relayout_dual();
        set_suppress_layout_notify(false);
    }

    fn relayout_dual(&mut self) {
        if matches!(self.last_vnode, Value::Null) {
            return;
        }
        // Match `commit_root`: floor tiny transient bounds so we still commit; when AppKit later
        // lays out real sizes, the quad changes and we run again (avoids skipping relayout entirely).
        let sw = self.sidebar_root.bounds().size.width.max(32.0) as f64;
        let sh = self.sidebar_root.bounds().size.height.max(32.0) as f64;
        let dw = self.detail_root.bounds().size.width.max(32.0) as f64;
        let dh = self.detail_root.bounds().size.height.max(32.0) as f64;
        let q = (sw, sh, dw, dh);
        let pq = self.last_quad.get();
        if !pq.0.is_nan()
            && (pq.0 - sw).abs() < 0.5
            && (pq.1 - sh).abs() < 0.5
            && (pq.2 - dw).abs() < 0.5
            && (pq.3 - dh).abs() < 0.5
        {
            return;
        }
        self.last_quad.set(q);
        let v = self.last_vnode.clone();
        let prev = if matches!(self.last_vnode, Value::Null) {
            None
        } else {
            Some(&self.last_vnode)
        };
        set_suppress_layout_notify(true);
        commit_sidebar_window_into(
            &v,
            self.sidebar_as_nsv(),
            self.detail_as_nsv(),
            sw,
            sh,
            dw,
            dh,
            &self.ctx,
            prev,
        );
        set_suppress_layout_notify(false);
    }
}

impl Host for MacosSidebarHost {
    fn commit_root(&mut self, vnode: &Value) {
        self.sync_toolbar_from_vnode(vnode);
        sync_sidebar_split_item_from_root(&self.split_vc, vnode);
        let sw = self.sidebar_root.bounds().size.width.max(32.0) as f64;
        let sh = self.sidebar_root.bounds().size.height.max(32.0) as f64;
        let dw = self.detail_root.bounds().size.width.max(32.0) as f64;
        let dh = self.detail_root.bounds().size.height.max(32.0) as f64;
        let prev = if matches!(self.last_vnode, Value::Null) {
            None
        } else {
            Some(&self.last_vnode)
        };
        set_suppress_layout_notify(true);
        commit_sidebar_window_into(
            vnode,
            self.sidebar_as_nsv(),
            self.detail_as_nsv(),
            sw,
            sh,
            dw,
            dh,
            &self.ctx,
            prev,
        );
        set_suppress_layout_notify(false);
        self.last_vnode = vnode.clone();
        let shell = window_shell_effective_props(vnode);
        self.window_delegate
            .sync_from_props(&shell, self.window.as_ref());
        if let Some(ref s) = appearance_string_from_props(&shell) {
            if !s.is_empty() {
                super::apply_window_appearance_to_window(self.window.as_ref(), Some(s.as_str()));
            }
        }
        self.sync_panes_to_laid_out_bounds();
    }

    fn content_width_changed(&mut self, _width: f64) {
        self.relayout_dual();
    }

    fn after_window_shown(&mut self) {
        self.window_delegate.fire_on_open();
        self.sync_panes_to_laid_out_bounds();
    }

    fn detach_native_actions(&mut self) {
        // Avoid `setDelegate:` on `NSWindow` / `NSToolbar` during teardown — same re-entrancy /
        // post-close invalid object issues as `MacosHost::detach_native_actions`.
        self.sidebar_root.disconnect_tish_root_routing();
        self.detail_root.disconnect_tish_root_routing();
        use super::build::detach_appkit_control_hooks_under;
        detach_appkit_control_hooks_under(self.sidebar_as_nsv());
        detach_appkit_control_hooks_under(self.detail_as_nsv());
        // Same as `MacosHost::detach_native_actions`: avoid `setContentViewController(None)` during
        // `windowWillClose:` while window close / layout animations may still reference the tree.
    }
}
