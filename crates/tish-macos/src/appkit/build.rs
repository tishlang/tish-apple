//! Map committed vnodes to AppKit views (top-down layout; root content view is flipped).

use std::rc::Rc;
use std::sync::Arc;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{sel, AnyThread, ClassType, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBezelStyle, NSBorderType, NSBox, NSBoxType, NSButton,
    NSCellImagePosition, NSControl,
    NSButtonType, NSColor, NSControlStateValueOff, NSControlStateValueOn, NSFont, NSImage,
    NSImageScaling, NSImageSymbolConfiguration, NSImageSymbolScale, NSImageView,
    NSProgressIndicator, NSProgressIndicatorStyle, NSPopUpButton,
    NSScrollElasticity, NSScrollView, NSSecureTextField, NSSlider,
    NSSplitViewDividerStyle, NSSwitch, NSTabView, NSTabViewItem, NSTextField, NSTextView,
    NSToolbarFlexibleSpaceItemIdentifier, NSToolbarItemIdentifier,
    NSToolbarSidebarTrackingSeparatorItemIdentifier, NSToolbarSpaceItemIdentifier,
    NSToolbarToggleSidebarItemIdentifier, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
};
use objc2_core_foundation::{CGFloat, CGPoint, CGSize};
use objc2_foundation::{NSEdgeInsets, NSObjectProtocol, NSRect, NSString, NSURL, NSURLRequest};
use objc2_web_kit::WKWebView;
use tishlang_core::{ObjectMap, Value};
use tishlang_ui::runtime::{is_fragment_tag, RootId};

use super::flipped::{
    snap_flipped_split_panes_full_height, FlippedClipView, FlippedDocumentView, FlippedRootView,
    FlippedSplitView, FlippedVisualEffectView,
};
use super::prop_warn::{
    warn_unknown_props, ROW_PROP_ALLOWLIST, SCROLL_PROP_ALLOWLIST, VISUAL_EFFECT_PROP_ALLOWLIST,
};
use super::markdown_view::{apply_markdown_text_view_chrome, set_text_view_markdown};
use super::handlers::{
    install_text_change_tag_on_text_view, register_bool_handler, register_click_handler,
    register_f64_handler, register_pick_handler, register_text_change_handler,
    register_toolbar_action_slot,
};
use super::router::MacosControlRouter;
use super::toolbar_delegate::ToolbarEntry;
use super::style::{
    apply_layer_style_to_view, apply_nstext_view_document_background_from_props,
    apply_static_label_text_field, has_container_layer_style, resolve_ns_color,
    single_line_label_height_after_style,
};
use super::tag::canonical_host_tag;
use super::text_delegate::TextFieldDelegate;
use super::text_view_delegate::TextViewDelegate;

/// Green scroll chrome + orange document layer for `scrollable` (any host). Disable after debugging.
const DEBUG_TINT_SCROLLABLES: bool = false;

/// AppKit `NSBezelStyleShadowlessSquare` (raw 6): flat template control. objc2 deprecates the
/// `NSBezelStyle::ShadowlessSquare` alias; the runtime style is unchanged.
const BEZEL_SHADOWLESS_SQUARE: NSBezelStyle = NSBezelStyle(6);

fn debug_tint_scrollable(scroll: &NSScrollView, doc: &FlippedDocumentView) {
    if !DEBUG_TINT_SCROLLABLES {
        return;
    }
    let chrome = NSColor::colorWithSRGBRed_green_blue_alpha(0.15, 0.75, 0.2, 0.38);
    scroll.setBackgroundColor(&chrome);
    let doc_view: &NSView = unsafe { &*std::ptr::from_ref(doc).cast::<NSView>() };
    doc_view.setWantsLayer(true);
    if let Some(layer) = doc_view.layer() {
        let fill = NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 0.55, 0.05, 0.4);
        let cg = fill.CGColor();
        layer.setBackgroundColor(Some(&*cg));
    }
}

#[derive(Clone)]
pub struct BuildCtx {
    pub mtm: MainThreadMarker,
    pub router: Retained<MacosControlRouter>,
    pub text_delegate: Retained<TextFieldDelegate>,
    pub text_view_delegate: Retained<TextViewDelegate>,
    pub root_id: RootId,
}

pub fn clear_subviews(view: &NSView) {
    let subs = view.subviews();
    let n = subs.count();
    for i in (0..n).rev() {
        let sv = subs.objectAtIndex(i);
        sv.removeFromSuperview();
    }
}

/// Clear `target` / `action` on controls and text delegates so AppKit cannot message a freed
/// [`super::router::MacosControlRouter`] or [`super::text_delegate::TextFieldDelegate`] after the host drops.
pub(super) fn detach_appkit_control_hooks_under(view: &NSView) {
    unsafe {
        detach_hooks_rec(view);
    }
}

unsafe fn detach_hooks_rec(view: &NSView) {
    let subs = view.subviews();
    let n = subs.count();
    for i in 0..n {
        let sub = subs.objectAtIndex(i);
        detach_hooks_rec(&*sub);
    }
    if view.isKindOfClass(NSControl::class()) {
        let ctl: &NSControl = &*std::ptr::from_ref(view).cast::<NSControl>();
        ctl.setTarget(None);
        ctl.setAction(None);
    }
    if view.isKindOfClass(NSTextField::class()) || view.isKindOfClass(NSSecureTextField::class()) {
        let tf: &NSTextField = &*std::ptr::from_ref(view).cast::<NSTextField>();
        tf.setDelegate(None);
    }
    if view.isKindOfClass(NSTextView::class()) {
        let tv: &NSTextView = &*std::ptr::from_ref(view).cast::<NSTextView>();
        tv.setDelegate(None);
    }
    if view.isKindOfClass(WKWebView::class()) {
        let wv: &WKWebView = &*std::ptr::from_ref(view).cast::<WKWebView>();
        unsafe {
            wv.setNavigationDelegate(None);
            wv.setUIDelegate(None);
        }
    }
}

pub(super) fn vnode_children(obj: &ObjectMap) -> Vec<Value> {
    match obj.get("children") {
        Some(Value::Array(a)) => a.borrow().clone(),
        _ => vec![],
    }
}

/// Element vnodes in source order: skip whitespace-only `Value::String` (JSX gaps), flatten
/// `<Fragment>`. Used by `sidebar_window` (exactly two panes) and `Split` (first two panes).
pub(super) fn collect_element_vnodes(children: &[Value], out: &mut Vec<Value>) {
    for c in children {
        match c {
            Value::Object(o) => {
                let m = &o.borrow().strings;
                let tag = m.get("tag").unwrap_or(&Value::Null);
                if is_fragment_tag(tag) {
                    let inner = vnode_children(&m);
                    collect_element_vnodes(&inner, out);
                } else if matches!(tag, Value::String(_)) {
                    out.push(c.clone());
                }
            }
            Value::String(_) => {}
            _ => {}
        }
    }
}

/// First two element children for `Split` — avoids treating a leading JSX text node as pane 0
/// (which left an empty column and dropped the real second child).
pub(super) fn split_pane_vnodes(children: &[Value]) -> Vec<Value> {
    let mut panes = Vec::new();
    collect_element_vnodes(children, &mut panes);
    panes.truncate(2);
    panes
}

fn collect_single_content_child(children: &[Value]) -> Option<Value> {
    let mut out: Vec<Value> = Vec::new();
    collect_element_vnodes(children, &mut out);
    if out.len() == 1 {
        Some(out.into_iter().next().unwrap())
    } else {
        None
    }
}

/// Single meaningful child for `macos_window` / `Window` shell (whitespace text ignored).
pub(super) fn macos_window_content(v: &Value) -> Option<Value> {
    match v {
        Value::Object(o) => {
            let m = &o.borrow().strings;
            let ok = matches!(
                m.get("tag"),
                Some(Value::String(s)) if {
                    let t = s.as_ref();
                    t == "macos_window" || t == "Window"
                }
            );
            if !ok {
                return None;
            }
            let ch = vnode_children(&m);
            collect_single_content_child(&ch)
        }
        _ => None,
    }
}

/// Effective props on a root `<Window>` / `<macos_window>` (e.g. `onClose`, `onOpen`).
pub(super) fn macos_window_shell_props(root: &Value) -> Option<ObjectMap> {
    match root {
        Value::Object(o) => {
            let m = &o.borrow().strings;
            let ok = matches!(
                m.get("tag"),
                Some(Value::String(s)) if {
                    let t = s.as_ref();
                    t == "macos_window" || t == "Window"
                }
            );
            if !ok {
                return None;
            }
            let raw = vnode_props(&m);
            Some(effective_props(&raw))
        }
        _ => None,
    }
}

/// Effective props on a root `<SidebarWindow>` / `<sidebar_window>`.
pub(super) fn sidebar_window_shell_props(root: &Value) -> Option<ObjectMap> {
    match root {
        Value::Object(o) => {
            let m = &o.borrow().strings;
            let ok = matches!(
                m.get("tag"),
                Some(Value::String(s)) if {
                    let t = s.as_ref();
                    t == "sidebar_window" || t == "SidebarWindow"
                }
            );
            if !ok {
                return None;
            }
            let raw = vnode_props(&m);
            Some(effective_props(&raw))
        }
        _ => None,
    }
}

/// Shell props for whichever window root tag is in use, or empty (callbacks cleared).
pub(super) fn window_shell_effective_props(root: &Value) -> ObjectMap {
    macos_window_shell_props(root)
        .or_else(|| sidebar_window_shell_props(root))
        .unwrap_or_default()
}

/// First pane = sidebar, second = detail. Whitespace-only JSX text between siblings is ignored.
pub(super) fn sidebar_window_children(v: &Value) -> Option<(Value, Value)> {
    match v {
        Value::Object(o) => {
            let m = &o.borrow().strings;
            let ok = matches!(
                m.get("tag"),
                Some(Value::String(s)) if {
                    let t = s.as_ref();
                    t == "sidebar_window" || t == "SidebarWindow"
                }
            );
            if !ok {
                return None;
            }
            let ch = vnode_children(&m);
            let mut panes = Vec::new();
            collect_element_vnodes(&ch, &mut panes);
            if panes.len() != 2 {
                return None;
            }
            Some((panes[0].clone(), panes[1].clone()))
        }
        _ => None,
    }
}

pub(super) fn vnode_props(obj: &ObjectMap) -> ObjectMap {
    match obj.get("props") {
        Some(Value::Object(p)) => p.borrow().strings.clone(),
        _ => ObjectMap::default(),
    }
}

pub(super) fn props_string(props: &ObjectMap, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(Value::String(s)) = props.get(*k) {
            return Some(s.to_string());
        }
    }
    None
}

pub(super) fn props_f64(props: &ObjectMap, keys: &[&str], default: f64) -> f64 {
    for k in keys {
        if let Some(n) = props.get(*k).and_then(|v| v.as_number()) {
            return n;
        }
    }
    default
}

pub(super) fn props_bool(props: &ObjectMap, keys: &[&str]) -> bool {
    for k in keys {
        match props.get(*k) {
            Some(Value::Bool(b)) => return *b,
            Some(v) => {
                if let Some(n) = v.as_number() {
                    return n != 0.0;
                }
            }
            None => {}
        }
    }
    false
}

/// Like [`props_bool`] but returns **`None`** if no key is present (for tri-state defaults).
pub(super) fn props_opt_bool(props: &ObjectMap, keys: &[&str]) -> Option<bool> {
    for k in keys {
        if let Some(v) = props.get(*k) {
            return Some(match v {
                Value::Bool(b) => *b,
                Value::Number(n) => *n != 0.0,
                Value::Null => false,
                _ => true,
            });
        }
    }
    None
}

pub(super) fn padding_insets(props: &ObjectMap) -> (f64, f64, f64, f64) {
    let p = props_f64(props, &["padding"], 0.0);
    let pt = props_f64(props, &["paddingTop"], p);
    let pr = props_f64(props, &["paddingRight"], p);
    let pb = props_f64(props, &["paddingBottom"], p);
    let pl = props_f64(props, &["paddingLeft"], p);
    (pt, pr, pb, pl)
}

/// Shallow merge: clone `props` without `style`, then overlay keys from `props.style` (object).
pub(super) fn effective_props(props: &ObjectMap) -> ObjectMap {
    let mut out = ObjectMap::default();
    for (k, v) in props.iter() {
        if k.as_ref() == "style" {
            continue;
        }
        out.insert(Arc::clone(k), v.clone());
    }
    if let Some(Value::Object(st)) = props.get("style") {
        for (k, v) in st.borrow().strings.iter() {
            out.insert(Arc::clone(k), v.clone());
        }
    }
    out
}

fn toolbar_tag_to_item_id(tag: &str) -> Option<&'static NSToolbarItemIdentifier> {
    unsafe {
        match tag {
            "ToolbarSidebarToggle" | "toolbar_sidebar_toggle" | "sidebar_toggle" => {
                Some(NSToolbarToggleSidebarItemIdentifier)
            }
            "ToolbarSidebarSeparator" | "toolbar_sidebar_separator" | "sidebar_separator" => {
                Some(NSToolbarSidebarTrackingSeparatorItemIdentifier)
            }
            "ToolbarSpacer" | "toolbar_spacer" | "flexible_space" | "FlexibleSpace" => {
                Some(NSToolbarFlexibleSpaceItemIdentifier)
            }
            "ToolbarSpace" | "toolbar_space" | "fixed_space" | "FixedSpace" => {
                Some(NSToolbarSpaceItemIdentifier)
            }
            _ => None,
        }
    }
}

fn push_toolbar_string_token(t: &str, out: &mut Vec<ToolbarEntry>) {
    let t = t.trim();
    unsafe {
        if let Some(id) = toolbar_tag_to_item_id(t) {
            out.push(ToolbarEntry::System(id));
            return;
        }
        match t {
            "toggle_sidebar" | "ToggleSidebar" => {
                out.push(ToolbarEntry::System(NSToolbarToggleSidebarItemIdentifier));
            }
            "sidebar_separator" | "SidebarSeparator" => {
                out.push(ToolbarEntry::System(
                    NSToolbarSidebarTrackingSeparatorItemIdentifier,
                ));
            }
            "flexible_space" | "FlexibleSpace" => {
                out.push(ToolbarEntry::System(NSToolbarFlexibleSpaceItemIdentifier));
            }
            "fixed_space" | "FixedSpace" | "toolbar_space" | "ToolbarSpace" => {
                out.push(ToolbarEntry::System(NSToolbarSpaceItemIdentifier));
            }
            _ => {}
        }
    }
}

/// `SidebarWindow` / `macos_window`: optional forced appearance name (`darkAqua`, …).
pub(super) fn appearance_string_from_props(props: &ObjectMap) -> Option<String> {
    props_string(props, &["appearance", "nsAppearance"])
}

/// Declarative toolbar: `toolbar` vnodes or `toolbarItems` (strings and `{ symbol, id, label? }` objects).
pub(super) fn toolbar_entries_from_props(
    props: &ObjectMap,
    root_id: RootId,
) -> Option<Vec<ToolbarEntry>> {
    let mut out = Vec::new();
    if let Some(Value::Array(a)) = props.get("toolbar") {
        for v in a.borrow().iter() {
            if let Value::Object(o) = v {
                let m = &o.borrow().strings;
                if let Some(sym) = props_string(&m, &["symbol", "sfSymbol", "sf_symbol"]) {
                    let action_id = props_string(&m, &["id", "actionId", "action_id"])
                        .unwrap_or_else(|| "item".to_string());
                    let label =
                        props_string(&m, &["label", "toolTip", "tooltip"]).unwrap_or_default();
                    let slot = register_toolbar_action_slot(root_id, action_id);
                    let ident_s = format!("com.tish.toolbar.{root_id}.{slot}");
                    let ident = NSString::from_str(&ident_s);
                    out.push(ToolbarEntry::Custom {
                        ident,
                        symbol: sym,
                        label,
                        slot_idx: slot,
                    });
                    continue;
                }
                let tg = m.get("tag").and_then(|t| match t {
                    Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let Some(ref s) = tg {
                    if let Some(id) = toolbar_tag_to_item_id(s.as_str()) {
                        out.push(ToolbarEntry::System(id));
                    }
                }
            }
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    if let Some(Value::Array(a)) = props
        .get("toolbarItems")
        .or_else(|| props.get("toolbar_items"))
    {
        for v in a.borrow().iter() {
            if let Value::Object(o) = v {
                let m = &o.borrow().strings;
                if let Some(sym) = props_string(&m, &["symbol", "sfSymbol", "sf_symbol"]) {
                    let action_id = props_string(&m, &["id", "actionId", "action_id"])
                        .unwrap_or_else(|| "item".to_string());
                    let label =
                        props_string(&m, &["label", "toolTip", "tooltip"]).unwrap_or_default();
                    let slot = register_toolbar_action_slot(root_id, action_id);
                    let ident_s = format!("com.tish.toolbar.{root_id}.{slot}");
                    let ident = NSString::from_str(&ident_s);
                    out.push(ToolbarEntry::Custom {
                        ident,
                        symbol: sym,
                        label,
                        slot_idx: slot,
                    });
                    continue;
                }
            }
            let s = v.to_display_string();
            push_toolbar_string_token(&s, &mut out);
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    None
}

pub(super) fn split_divider_style(props: &ObjectMap) -> NSSplitViewDividerStyle {
    match props_string(props, &["dividerStyle", "divider"]).as_deref() {
        Some("thick") => NSSplitViewDividerStyle::Thick,
        Some("pane") | Some("paneSplitter") | Some("pane_splitter") => {
            NSSplitViewDividerStyle::PaneSplitter
        }
        _ => NSSplitViewDividerStyle::Thin,
    }
}

/// Matches `NSSplitView.isVertical` and the axis used by `dividerPosition` / `splitPosition`.
///
/// - **`orientation="horizontal"`** (default): two **columns**, vertical divider; position is the **leading pane width**.
/// - **`orientation="vertical"`** | **`stacked`**: two **rows**, horizontal divider; position is the **top pane height**.
pub(super) fn split_uses_vertical_divider(props: &ObjectMap) -> bool {
    !matches!(
        props_string(props, &["orientation"])
            .map(|s| s.to_ascii_lowercase())
            .as_deref(),
        Some("vertical") | Some("stacked")
    )
}

/// Pane sizes `(w0, h0, w1, h1)` and divider position along the active axis for `setPosition:ofDividerAtIndex:`.
pub(super) fn split_pane_layout(props: &ObjectMap, iw: f64, th: f64) -> (f64, f64, f64, f64, f64) {
    if split_uses_vertical_divider(props) {
        let default_half = (iw / 2.0).max(1.0);
        let pos = props_f64(props, &["dividerPosition", "splitPosition"], default_half);
        let pos = pos.clamp(32.0, (iw - 32.0).max(32.0));
        let w1 = (iw - pos).max(1.0);
        (pos, th, w1, th, pos)
    } else {
        let default_half = (th / 2.0).max(1.0);
        let pos = props_f64(props, &["dividerPosition", "splitPosition"], default_half);
        let pos = pos.clamp(32.0, (th - 32.0).max(32.0));
        let h1 = (th - pos).max(1.0);
        (iw, pos, iw, h1, pos)
    }
}

pub(super) fn visual_effect_material_from_props(props: &ObjectMap) -> NSVisualEffectMaterial {
    if let Some(v) = props.get("material") {
        if let Some(n) = v.as_number() {
            if n.is_finite() {
                return NSVisualEffectMaterial(n.round() as isize);
            }
        }
    }
    let s = props_string(props, &["material"]).unwrap_or_default();
    match s.to_ascii_lowercase().as_str() {
        "titlebar" | "title_bar" => NSVisualEffectMaterial::Titlebar,
        "selection" => NSVisualEffectMaterial::Selection,
        "menu" => NSVisualEffectMaterial::Menu,
        "popover" => NSVisualEffectMaterial::Popover,
        "sidebar" | "" => NSVisualEffectMaterial::Sidebar,
        "header" | "headerview" | "header_view" => NSVisualEffectMaterial::HeaderView,
        "sheet" => NSVisualEffectMaterial::Sheet,
        "windowbackground" | "window_background" => NSVisualEffectMaterial::WindowBackground,
        "hud" | "hudwindow" | "hud_window" => NSVisualEffectMaterial::HUDWindow,
        "fullscreenui" | "fullscreen_ui" | "full_screen_ui" => NSVisualEffectMaterial::FullScreenUI,
        "tooltip" | "tool_tip" => NSVisualEffectMaterial::ToolTip,
        "content" | "contentbackground" | "content_background" => {
            NSVisualEffectMaterial::ContentBackground
        }
        "underwindow" | "under_window" => NSVisualEffectMaterial::UnderWindowBackground,
        "underpage" | "under_page" | "underpagebackground" | "under_page_background" => {
            NSVisualEffectMaterial::UnderPageBackground
        }
        "appearancebased" | "appearance_based" => {
            #[allow(deprecated)]
            {
                NSVisualEffectMaterial::AppearanceBased
            }
        }
        "light" => {
            #[allow(deprecated)]
            {
                NSVisualEffectMaterial::Light
            }
        }
        "dark" => {
            #[allow(deprecated)]
            {
                NSVisualEffectMaterial::Dark
            }
        }
        "mediumlight" | "medium_light" => {
            #[allow(deprecated)]
            {
                NSVisualEffectMaterial::MediumLight
            }
        }
        "ultradark" | "ultra_dark" => {
            #[allow(deprecated)]
            {
                NSVisualEffectMaterial::UltraDark
            }
        }
        _ => NSVisualEffectMaterial::Sidebar,
    }
}

pub(super) fn visual_effect_blending_from_props(props: &ObjectMap) -> NSVisualEffectBlendingMode {
    let s = props_string(props, &["blendingMode", "blending"]).unwrap_or_default();
    match s.to_ascii_lowercase().as_str() {
        "withinwindow" | "within_window" => NSVisualEffectBlendingMode::WithinWindow,
        "behindwindow" | "behind_window" | "" => NSVisualEffectBlendingMode::BehindWindow,
        _ => NSVisualEffectBlendingMode::BehindWindow,
    }
}

pub(super) fn visual_effect_state_from_props(props: &ObjectMap) -> NSVisualEffectState {
    let s = props_string(props, &["state", "visualEffectState", "visual_effect_state"])
        .unwrap_or_default();
    match s.to_ascii_lowercase().as_str() {
        "active" => NSVisualEffectState::Active,
        "inactive" => NSVisualEffectState::Inactive,
        "followswindow"
        | "follows_window"
        | "followswindowactivestate"
        | "follows_window_active"
        | "" => NSVisualEffectState::FollowsWindowActiveState,
        _ => NSVisualEffectState::FollowsWindowActiveState,
    }
}

/// `VisualEffect` / `visual_effect`: material, blending, state, emphasis, layer `style`, and optional `appearance`.
pub(super) fn apply_visual_effect_view_from_props(fx: &NSVisualEffectView, props: &ObjectMap) {
    fx.setMaterial(visual_effect_material_from_props(props));
    fx.setBlendingMode(visual_effect_blending_from_props(props));
    fx.setState(visual_effect_state_from_props(props));
    fx.setEmphasized(props_bool(
        props,
        &["emphasized", "isEmphasized", "is_emphasized"],
    ));
    let fx_ns: &NSView = unsafe { &*std::ptr::from_ref(fx).cast::<NSView>() };
    apply_layer_style_to_view(fx_ns, props);
    super::apply_view_appearance_from_props(fx_ns, props);
}

pub(super) fn text_from_children(children: &[Value]) -> String {
    children
        .iter()
        .map(|v| v.to_display_string())
        .collect::<Vec<_>>()
        .join("")
}

/// Join text vnode children and trim leading/trailing Unicode whitespace. Pretty-printed JSX adds
/// newlines/indent around children; AppKit single-line controls hide titles that start with `\n`.
pub(super) fn label_text_from_children(children: &[Value]) -> String {
    text_from_children(children).trim().to_string()
}

pub(super) fn options_strings(props: &ObjectMap) -> Vec<String> {
    let Some(Value::Array(a)) = props.get("options") else {
        return vec![];
    };
    let br = a.borrow();
    let mut out = Vec::new();
    for v in br.iter() {
        match v {
            Value::String(s) => out.push(s.to_string()),
            Value::Object(o) => {
                let m = &o.borrow().strings;
                let label = m
                    .get("label")
                    .or_else(|| m.get("value"))
                    .map(|x| x.to_display_string())
                    .unwrap_or_default();
                out.push(label);
            }
            _ => out.push(v.to_display_string()),
        }
    }
    out
}

pub(super) fn place(view: &NSView, x: f64, y: f64, w: f64, h: f64) {
    view.setFrame(NSRect::new(
        CGPoint::new(x, y),
        CGSize::new(w.max(0.0), h.max(0.0)),
    ));
}

/// Lay out the flipped `FlippedDocumentView` inside a default (unflipped) `NSVisualEffectView`.
///
/// Unflipped views use a bottom-left origin: `y = 0` pins the subview to the **bottom** of the
/// effect. When `container_h > doc_h` (explicit height / `"fill"` / rounding), that leaves empty
/// space **above** the header. Use `y = container_h - doc_h` so the document meets the top edge.
pub(super) fn place_visual_effect_document(
    container: &NSView,
    doc: &NSView,
    doc_w: f64,
    doc_h: f64,
    container_h: f64,
) {
    let y = if container.isFlipped() {
        0.0
    } else {
        (container_h - doc_h).max(0.0)
    };
    place(doc, 0.0, y, doc_w, doc_h);
}

/// Stack layout assigns explicit frames; clear autoresizing so a later window resize does not
/// stretch controls (e.g. `NSTextField`) and paint over siblings below.
#[inline]
pub(super) fn freeze_autoresizing_for_manual_frames(view: &NSView) {
    view.setAutoresizingMask(NSAutoresizingMaskOptions::empty());
}

const ZERO_EDGE_INSETS: NSEdgeInsets = NSEdgeInsets {
    top: 0.0,
    left: 0.0,
    bottom: 0.0,
    right: 0.0,
};

/// Outer `scrollable` / `list` / `visual_effect` height: numeric from props, `"fill"`, or **omitted**
/// → use the parent viewport when `avail_h` is `Some` (so a pane-filling `VisualEffect` is not stuck at 200).
pub(super) fn scroll_outer_height(props: &ObjectMap, avail_h: Option<f64>) -> f64 {
    match props_string(props, &["height", "h"]).as_deref() {
        Some("fill") => avail_h.unwrap_or(200.0),
        Some(s) => s
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite() && *n >= 0.0)
            .unwrap_or_else(|| props_f64(props, &["height", "h"], 200.0)),
        None => {
            if props
                .get("height")
                .or_else(|| props.get("h"))
                .and_then(|v| v.as_number())
                .is_some()
            {
                props_f64(props, &["height", "h"], 200.0)
            } else {
                avail_h.unwrap_or(200.0)
            }
        }
    }
}

/// `VisualEffect` in a `Column` middle slot gets `avail_h: None`. Without this,
/// [`scroll_outer_height`] falls back to **200** and `max(vh, content_h)` becomes a tall shell; the
/// inner [`FlippedDocumentView`] is then bottom-aligned inside the default **unflipped**
/// `NSVisualEffectView`, leaving a large empty band above (Notes list header vs editor toolbar).
/// Which `ZStack` child receives the pane’s bounded height (`height="fill"`). Default **`0`** (first
/// child is the scroll / split surface). Use **`1`** when child **0** is a fixed-height underlay
/// (e.g. `VisualEffect`) and child **1** should fill the stack.
fn zstack_fill_child_index(props: &ObjectMap, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    let keys = [
        "fillIndex",
        "fill_index",
        "fillChild",
        "fill_child",
        "fillSlot",
        "fill_slot",
    ];
    for k in keys {
        if let Some(v) = props.get(k) {
            let idx: i64 = match v {
                Value::Number(x) => *x as i64,
                Value::String(s) => s.trim().parse::<i64>().unwrap_or(-1),
                _ => continue,
            };
            if idx >= 0 && (idx as usize) < n {
                return idx as usize;
            }
        }
    }
    0
}

pub(super) fn visual_effect_intrinsic_outer_height(avail_h: Option<f64>, props: &ObjectMap) -> bool {
    if avail_h.is_some() {
        return false;
    }
    if props
        .get("height")
        .or_else(|| props.get("h"))
        .and_then(|v| v.as_number())
        .is_some()
    {
        return false;
    }
    match props_string(props, &["height", "h"]).as_deref() {
        None | Some("") => true,
        Some(s) => {
            let t = s.trim();
            if t.eq_ignore_ascii_case("fill") {
                false
            } else if t
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .is_some()
            {
                false
            } else {
                true
            }
        }
    }
}

pub(super) fn row_wants_click_overlay(props: &ObjectMap) -> bool {
    props
        .get("onClick")
        .or_else(|| props.get("onclick"))
        .is_some_and(|v| matches!(v, Value::Function(_)))
}

/// Column widths for [`Row`]: equal split, `weights={[1,2,1]}`, or **`columnWidths`** for Notes-style sidebars.
///
/// `columnWidths={[22, null, 44]}` → fixed 22px and 44px columns; **`null`** (or `0` or `"flex"`) marks a
/// flexible column that shares the remainder using the matching entry in **`weights`** (default `1` each).
/// If fixed widths exceed `iw`, fixed columns are scaled down proportionally before flex gets the rest.
pub(super) fn row_child_widths(iw: f64, n: usize, props: &ObjectMap) -> Vec<f64> {
    if n == 0 {
        return vec![];
    }
    let iw = iw.max(0.0);
    if let Some(Value::Array(cw_arr)) = props.get("columnWidths") {
        let spec = cw_arr.borrow();
        if spec.len() == n {
            let w_src: Vec<f64> = match props.get("weights") {
                Some(Value::Array(wa)) => {
                    let ws: Vec<f64> = wa.borrow().iter().filter_map(|v| v.as_number()).collect();
                    if ws.len() == n {
                        ws
                    } else {
                        vec![1.0; n]
                    }
                }
                _ => vec![1.0; n],
            };
            let mut fixed_mask = vec![false; n];
            let mut out = vec![0.0; n];
            let mut flex_i = Vec::new();
            let mut flex_w = Vec::new();
            for i in 0..n {
                let is_flex = match spec.get(i) {
                    Some(Value::Null) => true,
                    Some(v) => {
                        if let Some(px) = v.as_number() {
                            if px > 0.0 {
                                out[i] = px;
                                fixed_mask[i] = true;
                                false
                            } else {
                                true
                            }
                        } else {
                            let s = v.to_display_string();
                            matches!(s.trim(), "flex" | "*" | "1fr")
                        }
                    }
                    None => true,
                };
                if is_flex {
                    flex_i.push(i);
                    flex_w.push(w_src.get(i).copied().unwrap_or(1.0).max(0.0));
                }
            }
            let mut fixed_sum: f64 = out.iter().sum();
            if fixed_sum > iw && fixed_sum > 1e-9 {
                let scale = iw / fixed_sum;
                for j in 0..n {
                    if fixed_mask[j] {
                        out[j] *= scale;
                    }
                }
                fixed_sum = out.iter().sum();
            }
            let rem = (iw - fixed_sum).max(0.0);
            let wtot: f64 = flex_w.iter().sum();
            if !flex_i.is_empty() {
                if wtot > 1e-9 {
                    for (k, &idx) in flex_i.iter().enumerate() {
                        out[idx] = rem * (flex_w[k] / wtot);
                    }
                } else {
                    let each = rem / flex_i.len() as f64;
                    for &idx in &flex_i {
                        out[idx] = each;
                    }
                }
            }
            return out;
        }
    }
    if let Some(Value::Array(a)) = props.get("weights") {
        let ws: Vec<f64> = a.borrow().iter().filter_map(|v| v.as_number()).collect();
        if ws.len() == n {
            let sum: f64 = ws.iter().sum();
            if sum > 1e-9 && ws.iter().all(|w| *w >= 0.0) {
                return ws.iter().map(|w| iw * (w / sum)).collect();
            }
        }
    }
    let cw = iw / n as f64;
    vec![cw; n]
}

/// Cross-axis alignment for [`Row`] children inside a flipped shell (`y` grows downward).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RowCrossAlign {
    Start,
    Center,
    End,
}

pub(super) fn row_cross_align(props: &ObjectMap) -> RowCrossAlign {
    match props_string(
        props,
        &[
            "alignItems",
            "align_items",
            "verticalAlign",
            "vertical_align",
        ],
    )
    .map(|s| s.to_ascii_lowercase())
    .as_deref()
    {
        Some("start" | "flex-start") => RowCrossAlign::Start,
        Some("center" | "middle") => RowCrossAlign::Center,
        Some("end" | "flex-end" | "bottom") => RowCrossAlign::End,
        _ => RowCrossAlign::Start,
    }
}

/// Outer height for a shell [`Row`]: at least `pt + content_h + pb`, or `height` / `h` when larger.
pub(super) fn row_shell_outer_height(pt: f64, pb: f64, content_h: f64, props: &ObjectMap) -> f64 {
    let natural = pt + content_h + pb;
    let requested = props_f64(props, &["height", "h"], 0.0);
    if requested > 0.0 {
        natural.max(requested)
    } else {
        natural
    }
}

/// Adjust `y` for the first `n_children` subviews of `shell` (after a top-aligned pass at `pt`).
pub(super) fn row_shell_reposition_children(
    shell: &NSView,
    n_children: usize,
    heights: &[f64],
    pt: f64,
    inner_h: f64,
    align: RowCrossAlign,
) {
    if matches!(align, RowCrossAlign::Start) || n_children == 0 || heights.is_empty() {
        return;
    }
    let subs = shell.subviews();
    let n = subs.count() as usize;
    for i in 0..n_children.min(n).min(heights.len()) {
        let v = subs.objectAtIndex(i);
        let r = v.frame();
        let h = heights[i];
        let dy = match align {
            RowCrossAlign::Start => 0.0,
            RowCrossAlign::Center => ((inner_h - h) * 0.5).max(0.0),
            RowCrossAlign::End => (inner_h - h).max(0.0),
        };
        place(&v, r.origin.x, pt + dy, r.size.width, r.size.height);
    }
}

pub(super) fn button_bezel_from_props(props: &ObjectMap) -> NSBezelStyle {
    let s = props_string(props, &["bezelStyle", "bezel"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    match s.as_str() {
        "toolbar" | "texturedrounded" | "textured_rounded" => NSBezelStyle::Toolbar,
        // macOS 26+ Liquid Glass (`NSBezelStyleGlass`); matches in-titlebar sidebar toggle look in content.
        "glass" | "liquidglass" | "liquid_glass" => NSBezelStyle::Glass,
        // Rounded disc — use equal width/height for a circular hit target.
        "circular" | "circle" | "round" => NSBezelStyle::Circular,
        "accessorybar" | "accessory_bar" => NSBezelStyle::AccessoryBar,
        "accessorybaraction" | "accessory_bar_action" => NSBezelStyle::AccessoryBarAction,
        // True flat / “borderless” chrome — not `AccessoryBarAction`, which draws a rounded plate.
        "borderless" | "shadowless" | "shadowlesssquare" | "shadowless_square" => {
            BEZEL_SHADOWLESS_SQUARE
        }
        "smallsquare" | "small_square" => NSBezelStyle::SmallSquare,
        _ => NSBezelStyle::Push,
    }
}

pub(super) fn apply_button_chrome(btn: &NSButton, props: &ObjectMap) {
    btn.setBezelStyle(button_bezel_from_props(props));
    let icon = props_string(props, &["icon"]);
    let src = icon.or_else(|| {
        if props_bool(props, &["symbol", "sfSymbol", "sf_symbol"]) {
            props_string(props, &["src", "path", "url"])
        } else {
            None
        }
    });
    if let Some(ref name) = src {
        if let Some(im) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(name),
            None,
        ) {
            let im = if let Some(scale_s) = props_string(props, &["symbolScale", "iconScale"])
                .map(|s| s.to_ascii_lowercase())
            {
                let sc = match scale_s.as_str() {
                    "small" => NSImageSymbolScale::Small,
                    "large" => NSImageSymbolScale::Large,
                    _ => NSImageSymbolScale::Medium,
                };
                let cfg = NSImageSymbolConfiguration::configurationWithScale(sc);
                im.imageWithSymbolConfiguration(&cfg).unwrap_or(im)
            } else {
                let bezel = button_bezel_from_props(props);
                // Untitled overlay buttons skip rescaling (full-row hit targets).
                let title_empty = btn.title().to_string().trim().is_empty();
                if title_empty
                    && matches!(bezel, NSBezelStyle::Toolbar | NSBezelStyle::Glass)
                {
                    let cfg =
                        NSImageSymbolConfiguration::configurationWithScale(NSImageSymbolScale::Small);
                    im.imageWithSymbolConfiguration(&cfg).unwrap_or(im)
                } else if !title_empty
                    && (matches!(
                        bezel,
                        NSBezelStyle::AccessoryBar | NSBezelStyle::AccessoryBarAction
                    ) || bezel == BEZEL_SHADOWLESS_SQUARE)
                {
                    let cfg =
                        NSImageSymbolConfiguration::configurationWithScale(NSImageSymbolScale::Small);
                    im.imageWithSymbolConfiguration(&cfg).unwrap_or(im)
                } else {
                    im
                }
            };
            btn.setImage(Some(&im));
            btn.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
            let has_title = !btn.title().to_string().trim().is_empty();
            btn.setImagePosition(if has_title {
                NSCellImagePosition::ImageLeading
            } else {
                NSCellImagePosition::ImageOnly
            });
            let bezel_now = button_bezel_from_props(props);
            if !has_title && matches!(bezel_now, NSBezelStyle::Toolbar | NSBezelStyle::Glass) {
                // Icon-only toolbar / glass: bordered shows the capsule or Liquid Glass bezel.
                // Built-in `NSToolbarToggleSidebarItemIdentifier` is drawn by `NSToolbar`, not `NSButton`.
                let bordered = props_opt_bool(
                    props,
                    &[
                        "bordered",
                        "buttonBordered",
                        "toolbarBordered",
                        "glassBordered",
                    ],
                )
                .unwrap_or(true);
                btn.setBordered(bordered);
            }
            if !has_title && matches!(bezel_now, NSBezelStyle::Circular) {
                let bordered = props_opt_bool(props, &["bordered", "buttonBordered"])
                    .unwrap_or(true);
                btn.setBordered(bordered);
            }
            if !has_title && bezel_now == BEZEL_SHADOWLESS_SQUARE {
                btn.setBordered(false);
                btn.setTransparent(true);
            }
            // Template symbols: default `label` tint on toolbar / glass / circular icon buttons.
            let toolbar_icon = !has_title
                && (matches!(
                    bezel_now,
                    NSBezelStyle::Toolbar | NSBezelStyle::Glass | NSBezelStyle::Circular
                ) || bezel_now == BEZEL_SHADOWLESS_SQUARE);
            match props_string(props, &["tint", "contentTint", "symbolTint", "symbol_tint"]) {
                Some(ref s) => {
                    if let Some(col) = resolve_ns_color(s) {
                        btn.setContentTintColor(Some(&col));
                    }
                }
                None if toolbar_icon => {
                    let label = NSColor::labelColor();
                    btn.setContentTintColor(Some(&label));
                }
                _ => {}
            }
        }
    } else {
        btn.setImage(None);
        btn.setImagePosition(NSCellImagePosition::NoImage);
    }
}

/// Removes the extra blank band macOS often adds above scroll content (safe-area / automatic
/// content insets on `NSScrollView` and `NSClipView`).
pub(crate) fn strip_scroll_content_insets(scroll: &NSScrollView) {
    scroll.setAutomaticallyAdjustsContentInsets(false);
    // Do **not** reset `scroll.setContentInsets` here: optional **`scrollerGutterRight`** on
    // **`ScrollView`** / **`GroupedTable`** uses a positive **`contentInsets.right`** so document
    // rows do not paint over the vertical scroller track; `sync_scroll_view_for_document` calls this
    // helper and must not wipe that inset.
    // Do **not** reset `scrollerInsets` here: `documentTopInset` / overlay headers use a positive top
    // scroller inset so the vertical knob is not clipped under the overlay; `sync_scroll_view_for_document`
    // calls this helper and must not wipe that inset.
    let clip = scroll.contentView();
    clip.setAutomaticallyAdjustsContentInsets(false);
    clip.setContentInsets(ZERO_EDGE_INSETS);
    clip.setDrawsBackground(false);
}

/// Push the vertical scroller down so its track clears a `ZStack` overlay (`documentTopInset` height).
pub(crate) fn apply_scroll_scroller_top_inset(scroll: &NSScrollView, top: f64) {
    let t = top.max(0.0);
    let ins = if t > 0.0 {
        NSEdgeInsets {
            top: t,
            left: 0.0,
            bottom: 0.0,
            right: 0.0,
        }
    } else {
        ZERO_EDGE_INSETS
    };
    scroll.setScrollerInsets(ins);
    scroll.tile();
}

/// Inset scroll **content** from the trailing edge so list/table backgrounds do not draw into the
/// vertical **`NSScrollView`** scroller lane (overlay scrollers).
pub(crate) fn apply_scroll_content_right_gutter(scroll: &NSScrollView, right: f64) {
    let r = right.max(0.0);
    scroll.setContentInsets(NSEdgeInsets {
        top: 0.0,
        left: 0.0,
        bottom: 0.0,
        right: r,
    });
    scroll.tile();
}

pub(crate) fn scroll_scroller_right_gutter_from_props(props: &ObjectMap) -> f64 {
    props_f64(
        props,
        &["scrollerGutterRight", "scroller_gutter_right"],
        0.0,
    )
    .max(0.0)
}

pub(crate) fn tune_scroll_view_chrome(scroll: &NSScrollView, has_vertical: bool, has_horizontal: bool) {
    scroll.setBorderType(NSBorderType::NoBorder);
    scroll.setAutohidesScrollers(true);
    if has_vertical && !has_horizontal {
        scroll.setHorizontalScrollElasticity(NSScrollElasticity::None);
    }
    if has_horizontal && !has_vertical {
        scroll.setVerticalScrollElasticity(NSScrollElasticity::None);
    }
}

/// Apply `value` from props without `textDidChange:` → Tish `onChange` (prevents re-entrant flush).
pub(super) fn text_view_set_string_without_delegate_notice(
    tv: &NSTextView,
    text: &str,
    ctx: &BuildCtx,
) {
    tv.setDelegate(None);
    tv.setString(&NSString::from_str(text));
    tv.setDelegate(Some(ProtocolObject::from_ref(
        &*ctx.text_view_delegate,
    )));
}

/// After layout/patch, re-tile the scroll view, restore the clip’s document offset (clamped to the
/// new document size via `constrainBoundsRect:`), and call `reflectScrolledClipView:` so
/// `NSScroller` knobs match.
///
/// **Flipped clip** ([`FlippedClipView`] + flipped document): preserve `bounds.origin` across
/// `tile()` so Tish scroll position stays stable.
///
/// **Default `NSClipView`** ([`GroupedTable`](super::grouped_table), `list`, …): do **not** reuse
/// that origin math — it assumes flipped scrolling and can park the clip over an empty region
/// (blank pane, chrome pushed to the wrong edge).
pub(super) fn sync_scroll_view_for_document(scroll: &NSScrollView, doc: &NSView) {
    let clip_before = scroll.contentView();
    let use_flipped_origin_restore = clip_before.isKindOfClass(FlippedClipView::class());
    let saved_origin = if use_flipped_origin_restore {
        Some(clip_before.bounds().origin)
    } else {
        None
    };

    scroll.layoutSubtreeIfNeeded();
    doc.layoutSubtreeIfNeeded();
    scroll.tile();
    strip_scroll_content_insets(scroll);

    let clip = scroll.contentView();
    if let Some(origin) = saved_origin {
        let b = clip.bounds();
        let proposed = NSRect::new(origin, b.size);
        let constrained = clip.constrainBoundsRect(proposed);
        scroll.scrollClipView_toPoint(&clip, constrained.origin);
        strip_scroll_content_insets(scroll);
    }
    scroll.reflectScrolledClipView(&clip);
    super::scroll_chrome_embed::reposition_embedded_header_if_any(scroll);
}

unsafe fn view_as_scroll(v: &NSView) -> Option<&NSScrollView> {
    if v.isKindOfClass(NSScrollView::class()) {
        Some(&*(std::ptr::from_ref(v).cast::<NSScrollView>()))
    } else {
        None
    }
}

/// Run after the root frame is set so nested `NSScrollView`s have finished layout (sync inside
/// `commit_vnode` can run too early and be undone by a later `layout`).
pub(super) fn resync_all_scroll_views_under(v: &NSView) {
    let subs = v.subviews();
    let n = subs.count();
    for i in 0..n {
        let sub = subs.objectAtIndex(i);
        if let Some(scroll) = unsafe { view_as_scroll(&*sub) } {
            if let Some(doc) = scroll.documentView() {
                sync_scroll_view_for_document(scroll, &doc);
            }
        }
        resync_all_scroll_views_under(&*sub);
    }
}

fn wire_on_click(props: &ObjectMap, btn: &NSButton, ctx: &BuildCtx) {
    if let Some(Value::Function(f)) = props.get("onClick").or_else(|| props.get("onclick")) {
        let f = f.clone();
        let idx = register_click_handler(ctx.root_id, Rc::new(move || {
            let _ = f(&[]);
        })) as isize;
        btn.setTag(idx);
        unsafe {
            let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
            btn.setTarget(Some(&*p));
            btn.setAction(Some(sel!(jsxClick:)));
        }
    }
}

pub fn commit_vnode(
    v: &Value,
    parent: &NSView,
    x: f64,
    y_top: f64,
    avail_w: f64,
    avail_h: Option<f64>,
    ctx: &BuildCtx,
) -> f64 {
    match v {
        Value::String(s) => {
            let t = s.as_ref().trim();
            let tf = NSTextField::labelWithString(&NSString::from_str(t), ctx.mtm);
            apply_static_label_text_field(&tf, &ObjectMap::default(), ctx.mtm);
            let h = single_line_label_height_after_style(&tf, &ObjectMap::default());
            place(&tf, x, y_top, avail_w, h);
            freeze_autoresizing_for_manual_frames(&tf);
            parent.addSubview(&tf);
            h
        }
        Value::Object(obj) => {
            let map = &obj.borrow().strings;
            let tag = map
                .get("tag")
                .and_then(|t| match t {
                    Value::String(s) => Some(s.as_ref().to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            if is_fragment_tag(map.get("tag").unwrap_or(&Value::Null)) {
                let ch = vnode_children(&map);
                let h_child = if ch.len() == 1 { avail_h } else { None };
                let mut y = y_top;
                let mut hsum = 0.0;
                for c in &ch {
                    let h = commit_vnode(c, parent, x, y, avail_w, h_child, ctx);
                    y += h;
                    hsum += h;
                }
                return hsum;
            }
            let raw_props = vnode_props(&map);
            let props = effective_props(&raw_props);
            let children = vnode_children(&map);
            let (pt, pr, pb, pl) = padding_insets(&props);
            let ix = x + pl;
            let iy = y_top + pt;
            let iw = (avail_w - pl - pr).max(0.0);

            match canonical_host_tag(tag.as_str()) {
                "space" => {
                    let wv = props_f64(&props, &["width", "w"], 8.0);
                    let hv = props_f64(&props, &["height", "h"], 8.0);
                    let v = NSView::new(ctx.mtm);
                    place(&v, ix, iy, wv, hv);
                    freeze_autoresizing_for_manual_frames(&v);
                    parent.addSubview(&v);
                    pt + hv + pb
                }
                "rule" => {
                    if props_string(&props, &["orientation"]) == Some("vertical".into()) {
                        let wv = props_f64(&props, &["width", "w"], 1.0);
                        let hv = props_f64(&props, &["height", "h"], 120.0);
                        let bx = NSBox::new(ctx.mtm);
                        bx.setBoxType(NSBoxType::Separator);
                        place(&bx, ix, iy, wv, hv);
                        freeze_autoresizing_for_manual_frames(&bx);
                        parent.addSubview(&bx);
                        pt + hv + pb
                    } else {
                        let hv = props_f64(&props, &["height", "h"], 1.0);
                        let bx = NSBox::new(ctx.mtm);
                        bx.setBoxType(NSBoxType::Separator);
                        place(&bx, ix, iy, iw, hv);
                        freeze_autoresizing_for_manual_frames(&bx);
                        parent.addSubview(&bx);
                        pt + hv + pb
                    }
                }
                "row" => {
                    warn_unknown_props("Row", &props, ROW_PROP_ALLOWLIST);
                    let n = children.len().max(1);
                    let click_overlay = row_wants_click_overlay(&props);
                    let align = row_cross_align(&props);
                    let use_shell = has_container_layer_style(&props)
                        || click_overlay
                        || align != RowCrossAlign::Start;
                    // Layer + click target: padding insets the *content* inside the shell so
                    // `backgroundColor` / `borderRadius` span the full `avail_w` (CSS-like padding).
                    // Rows without a shell keep the legacy inset (margin-like) layout.
                    let content_w = if use_shell {
                        (avail_w - pl - pr).max(0.0)
                    } else {
                        iw
                    };
                    let widths = row_child_widths(content_w, n, &props);
                    let mut max_h = 0.0_f64;
                    if use_shell {
                        let shell = FlippedDocumentView::new(
                            ctx.mtm,
                            NSRect::new(CGPoint::ZERO, CGSize::new(avail_w.max(1.0), 0.0)),
                        );
                        let shell_ns: &NSView =
                            unsafe { &*std::ptr::from_ref(&*shell).cast::<NSView>() };
                        if has_container_layer_style(&props) {
                            apply_layer_style_to_view(shell_ns, &props);
                        }
                        let mut heights: Vec<f64> = Vec::with_capacity(children.len());
                        let mut x_off = 0.0_f64;
                        for (i, c) in children.iter().enumerate() {
                            let cw = widths.get(i).copied().unwrap_or(0.0);
                            let h = commit_vnode(c, shell_ns, pl + x_off, pt, cw, None, ctx);
                            heights.push(h);
                            max_h = max_h.max(h);
                            x_off += cw;
                        }
                        let content_h = max_h.max(1.0);
                        let row_h = row_shell_outer_height(pt, pb, content_h, &props);
                        let inner_h = (row_h - pt - pb).max(0.0);
                        row_shell_reposition_children(
                            shell_ns,
                            children.len(),
                            &heights,
                            pt,
                            inner_h,
                            align,
                        );
                        if click_overlay {
                            let overlay = NSButton::new(ctx.mtm);
                            overlay.setTitle(&NSString::from_str("\u{200b}"));
                            overlay.setBezelStyle(NSBezelStyle::AccessoryBarAction);
                            overlay.setBordered(false);
                            overlay.setTransparent(true);
                            overlay.setImage(None);
                            overlay.setImagePosition(NSCellImagePosition::NoImage);
                            wire_on_click(&props, &overlay, ctx);
                            place(&overlay, 0.0, 0.0, avail_w, row_h);
                            freeze_autoresizing_for_manual_frames(&overlay);
                            shell_ns.addSubview(&overlay);
                        }
                        shell.setFrameSize(CGSize::new(avail_w, row_h));
                        place(shell_ns, x, y_top, avail_w, row_h);
                        freeze_autoresizing_for_manual_frames(shell_ns);
                        parent.addSubview(shell_ns);
                        // Must match shell height: parent layout (e.g. scroll doc) uses this, not frame alone.
                        row_h
                    } else {
                        let mut cx = ix;
                        for (i, c) in children.iter().enumerate() {
                            let cw = widths.get(i).copied().unwrap_or(0.0);
                            let h = commit_vnode(c, parent, cx, iy, cw, None, ctx);
                            max_h = max_h.max(h);
                            cx += cw;
                        }
                        pt + max_h + pb
                    }
                }
                "column" | "section" => {
                    let n = children.len();
                    if n == 0 {
                        return pt + pb;
                    }
                    let mut y = iy;
                    let mut consumed = 0.0_f64;
                    if let Some(ah) = avail_h {
                        let content_h = (ah - pt - pb).max(0.0);
                        if n >= 2 {
                            for c in &children[..n - 1] {
                                let h = commit_vnode(c, parent, ix, y, iw, None, ctx);
                                y += h;
                                consumed += h;
                            }
                            let rem = (content_h - consumed).max(1.0);
                            let h_last =
                                commit_vnode(&children[n - 1], parent, ix, y, iw, Some(rem), ctx);
                            consumed += h_last;
                        } else {
                            consumed =
                                commit_vnode(&children[0], parent, ix, y, iw, Some(content_h), ctx);
                        }
                        pt + consumed + pb
                    } else {
                        for c in &children {
                            let h = commit_vnode(c, parent, ix, y, iw, None, ctx);
                            y += h;
                            consumed += h;
                        }
                        pt + consumed + pb
                    }
                }
                "zstack" => {
                    let th = scroll_outer_height(&props, avail_h);
                    let n = children.len();
                    if n == 0 {
                        return pt + pb;
                    }
                    let fill_i = zstack_fill_child_index(&props, n);
                    let shell = FlippedDocumentView::new(
                        ctx.mtm,
                        NSRect::new(
                            CGPoint::ZERO,
                            CGSize::new(iw.max(1.0), th.max(1.0)),
                        ),
                    );
                    shell.setAutoresizingMask(
                        NSAutoresizingMaskOptions::ViewWidthSizable
                            | NSAutoresizingMaskOptions::ViewHeightSizable,
                    );
                    let shell_view: &NSView =
                        unsafe { &*std::ptr::from_ref(&*shell).cast::<NSView>() };
                    for (i, c) in children.iter().enumerate() {
                        let child_avail = if i == fill_i { Some(th) } else { None };
                        let _ = commit_vnode(c, shell_view, 0.0, 0.0, iw, child_avail, ctx);
                    }
                    place(shell_view, ix, iy, iw, th);
                    parent.addSubview(shell_view);
                    super::scroll_chrome_embed::zstack_try_embed_grouped_table_header(shell_view);
                    pt + th + pb
                }
                "scrollable" => {
                    warn_unknown_props("ScrollView", &props, SCROLL_PROP_ALLOWLIST);
                    let sh = scroll_outer_height(&props, avail_h);
                    let doc_top = props_f64(
                        &props,
                        &[
                            "documentTopInset",
                            "document_top_inset",
                            "contentInsetTop",
                            "content_inset_top",
                        ],
                        0.0,
                    )
                    .max(0.0);
                    let dir = props_string(&props, &["direction", "orient"]).unwrap_or_default();
                    let scroll = NSScrollView::new(ctx.mtm);
                    scroll.setDrawsBackground(props_bool(
                        &props,
                        &["drawsBackground", "draws_background"],
                    ));
                    scroll.setAutoresizingMask(
                        NSAutoresizingMaskOptions::ViewWidthSizable
                            | NSAutoresizingMaskOptions::ViewHeightSizable,
                    );
                    let has_v = matches!(dir.as_str(), "vertical" | "both" | "");
                    let has_h = dir == "horizontal" || dir == "both";
                    scroll.setHasVerticalScroller(has_v);
                    scroll.setHasHorizontalScroller(has_h);
                    tune_scroll_view_chrome(&scroll, has_v, has_h);
                    strip_scroll_content_insets(&scroll);
                    let clip_frame = scroll.contentView().frame();
                    let flipped_clip = FlippedClipView::new(ctx.mtm, clip_frame);
                    flipped_clip.setAutomaticallyAdjustsContentInsets(false);
                    flipped_clip.setContentInsets(ZERO_EDGE_INSETS);
                    scroll.setContentView(&flipped_clip);
                    let doc = FlippedDocumentView::new(
                        ctx.mtm,
                        NSRect::new(CGPoint::ZERO, CGSize::new(iw, 0.0)),
                    );
                    let mut doc_h = doc_top;
                    let mut dy = doc_top;
                    for c in &children {
                        let h = commit_vnode(c, &doc, 0.0, dy, iw, None, ctx);
                        dy += h;
                        doc_h += h;
                    }
                    doc.setFrameSize(CGSize::new(iw, doc_h.max(1.0)));
                    scroll.setDocumentView(Some(&doc));
                    let doc_view: &NSView = unsafe { &*std::ptr::from_ref(&*doc).cast::<NSView>() };
                    apply_layer_style_to_view(doc_view, &props);
                    debug_tint_scrollable(&scroll, &doc);
                    place(&scroll, ix, iy, iw, sh);
                    parent.addSubview(&scroll);
                    sync_scroll_view_for_document(&scroll, &doc);
                    apply_scroll_scroller_top_inset(&scroll, doc_top);
                    apply_scroll_content_right_gutter(
                        &scroll,
                        scroll_scroller_right_gutter_from_props(&props),
                    );
                    pt + sh + pb
                }
                "grouped_table" => {
                    let sh = scroll_outer_height(&props, avail_h);
                    let doc_top = props_f64(
                        &props,
                        &[
                            "documentTopInset",
                            "document_top_inset",
                            "contentInsetTop",
                            "content_inset_top",
                        ],
                        0.0,
                    )
                    .max(0.0);
                    let vibrant_sections = props_opt_bool(
                        &props,
                        &["vibrantSectionHeaders", "vibrant_section_headers"],
                    )
                    .unwrap_or(true);
                    let scroll = super::grouped_table::install_grouped_table_scroll(
                        ctx.mtm,
                        iw,
                        sh,
                        &children,
                        ctx,
                        doc_top,
                        vibrant_sections,
                    );
                    scroll.setAutoresizingMask(
                        NSAutoresizingMaskOptions::ViewWidthSizable
                            | NSAutoresizingMaskOptions::ViewHeightSizable,
                    );
                    place(&scroll, ix, iy, iw, sh);
                    parent.addSubview(&scroll);
                    if let Some(doc) = scroll.documentView() {
                        sync_scroll_view_for_document(&scroll, &doc);
                        apply_scroll_scroller_top_inset(&scroll, doc_top);
                    }
                    pt + sh + pb
                }
                "button" => {
                    let title = label_text_from_children(&children);
                    let btn = NSButton::new(ctx.mtm);
                    btn.setTitle(&NSString::from_str(&title));
                    apply_button_chrome(&btn, &props);
                    wire_on_click(&props, &btn, ctx);
                    let h = props_f64(&props, &["height", "h"], 32.0);
                    apply_layer_style_to_view(&btn, &props);
                    place(&btn, ix, iy, iw, h);
                    freeze_autoresizing_for_manual_frames(&btn);
                    parent.addSubview(&btn);
                    pt + h + pb
                }
                "text" => {
                    let text = label_text_from_children(&children);
                    let wrap = props_bool(&props, &["wrap", "wrapping"]);
                    let tf = if wrap {
                        NSTextField::wrappingLabelWithString(&NSString::from_str(&text), ctx.mtm)
                    } else {
                        NSTextField::labelWithString(&NSString::from_str(&text), ctx.mtm)
                    };
                    apply_static_label_text_field(&tf, &props, ctx.mtm);
                    let h = if wrap {
                        props_f64(&props, &["height", "h"], 44.0)
                    } else {
                        single_line_label_height_after_style(&tf, &props)
                    };
                    place(&tf, ix, iy, iw, h);
                    freeze_autoresizing_for_manual_frames(&tf);
                    parent.addSubview(&tf);
                    pt + h + pb
                }
                "textinput" => {
                    let tf = NSTextField::new(ctx.mtm);
                    tf.setStringValue(&NSString::from_str(
                        &props_string(&props, &["value", "defaultValue"]).unwrap_or_default(),
                    ));
                    tf.setBezeled(true);
                    tf.setEditable(true);
                    if let Some(Value::Function(f)) =
                        props.get("onChange").or_else(|| props.get("onInput"))
                    {
                        let f = f.clone();
                        let idx = register_text_change_handler(
                            ctx.root_id,
                            Rc::new(move |s: String| {
                                let _ = f(&[Value::String(s.into())]);
                            }),
                        ) as isize;
                        tf.setTag(idx);
                        unsafe {
                            tf.setDelegate(Some(ProtocolObject::from_ref(
                                &*ctx.text_delegate,
                            )));
                        }
                    }
                    let h = 24.0;
                    place(&tf, ix, iy, iw, h);
                    freeze_autoresizing_for_manual_frames(&tf);
                    parent.addSubview(&tf);
                    pt + h + pb
                }
                "password" => {
                    let tf = NSSecureTextField::new(ctx.mtm);
                    tf.setStringValue(&NSString::from_str(
                        &props_string(&props, &["value", "defaultValue"]).unwrap_or_default(),
                    ));
                    if let Some(Value::Function(f)) =
                        props.get("onChange").or_else(|| props.get("onInput"))
                    {
                        let f = f.clone();
                        let idx = register_text_change_handler(
                            ctx.root_id,
                            Rc::new(move |s: String| {
                                let _ = f(&[Value::String(s.into())]);
                            }),
                        ) as isize;
                        tf.setTag(idx);
                        unsafe {
                            tf.setDelegate(Some(ProtocolObject::from_ref(
                                &*ctx.text_delegate,
                            )));
                        }
                    }
                    let h = 24.0;
                    place(&tf, ix, iy, iw, h);
                    freeze_autoresizing_for_manual_frames(&tf);
                    parent.addSubview(&tf);
                    pt + h + pb
                }
                "checkbox" => {
                    let btn = NSButton::new(ctx.mtm);
                    btn.setButtonType(NSButtonType::Switch);
                    btn.setTitle(&NSString::from_str(&label_text_from_children(&children)));
                    let checked = props_bool(&props, &["checked", "value"]);
                    btn.setState(if checked {
                        NSControlStateValueOn
                    } else {
                        NSControlStateValueOff
                    });
                    if let Some(Value::Function(f)) =
                        props.get("onChange").or_else(|| props.get("onToggle"))
                    {
                        let f = f.clone();
                        let idx = register_bool_handler(ctx.root_id, Rc::new(move |b| {
                            let _ = f(&[Value::Bool(b)]);
                        })) as isize;
                        btn.setTag(idx);
                        unsafe {
                            let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
                            btn.setTarget(Some(&*p));
                            btn.setAction(Some(sel!(tishBool:)));
                        }
                    }
                    let h = 28.0;
                    place(&btn, ix, iy, iw, h);
                    freeze_autoresizing_for_manual_frames(&btn);
                    parent.addSubview(&btn);
                    pt + h + pb
                }
                "toggler" => {
                    let sw = NSSwitch::new(ctx.mtm);
                    sw.setState(if props_bool(&props, &["checked", "value"]) {
                        NSControlStateValueOn
                    } else {
                        NSControlStateValueOff
                    });
                    if let Some(Value::Function(f)) =
                        props.get("onChange").or_else(|| props.get("onToggle"))
                    {
                        let f = f.clone();
                        let idx = register_bool_handler(ctx.root_id, Rc::new(move |b| {
                            let _ = f(&[Value::Bool(b)]);
                        })) as isize;
                        sw.setTag(idx);
                        unsafe {
                            let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
                            sw.setTarget(Some(&*p));
                            sw.setAction(Some(sel!(tishBool:)));
                        }
                    }
                    let h = 28.0;
                    place(&sw, ix, iy, 60.0, h);
                    freeze_autoresizing_for_manual_frames(&sw);
                    parent.addSubview(&sw);
                    pt + h + pb
                }
                "slider" => {
                    let sl = NSSlider::new(ctx.mtm);
                    let minv = props_f64(&props, &["min"], 0.0);
                    let maxv = props_f64(&props, &["max"], 100.0);
                    sl.setMinValue(minv);
                    sl.setMaxValue(maxv);
                    sl.setDoubleValue(props_f64(&props, &["value"], minv));
                    if let Some(Value::Function(f)) =
                        props.get("onChange").or_else(|| props.get("onInput"))
                    {
                        let f = f.clone();
                        let idx = register_f64_handler(ctx.root_id, Rc::new(move |v| {
                            let _ = f(&[Value::Number(v)]);
                        })) as isize;
                        sl.setTag(idx);
                        unsafe {
                            let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
                            sl.setTarget(Some(&*p));
                            sl.setAction(Some(sel!(tishSlider:)));
                        }
                    }
                    let h = 24.0;
                    place(&sl, ix, iy, iw, h);
                    freeze_autoresizing_for_manual_frames(&sl);
                    parent.addSubview(&sl);
                    pt + h + pb
                }
                "progress_bar" => {
                    let ind = props_bool(&props, &["indeterminate"]);
                    let pi = NSProgressIndicator::new(ctx.mtm);
                    if ind {
                        pi.setStyle(NSProgressIndicatorStyle::Spinning);
                        pi.setIndeterminate(true);
                        unsafe {
                            pi.startAnimation(None);
                        }
                    } else {
                        pi.setStyle(NSProgressIndicatorStyle::Bar);
                        pi.setIndeterminate(false);
                        pi.setMinValue(0.0);
                        pi.setMaxValue(props_f64(&props, &["max"], 1.0));
                        pi.setDoubleValue(props_f64(&props, &["value"], 0.0));
                    }
                    let h = 20.0;
                    place(&pi, ix, iy, iw, h);
                    freeze_autoresizing_for_manual_frames(&pi);
                    parent.addSubview(&pi);
                    pt + h + pb
                }
                "pick_list" => {
                    let opts = options_strings(&props);
                    let popup = NSPopUpButton::new(ctx.mtm);
                    popup.removeAllItems();
                    for o in &opts {
                        popup.addItemWithTitle(&NSString::from_str(o));
                    }
                    let sel = props_f64(&props, &["selected", "value"], 0.0) as isize;
                    if !opts.is_empty() {
                        let max_i = (opts.len() - 1) as isize;
                        let si = sel.max(0).min(max_i);
                        popup.selectItemAtIndex(si);
                    }
                    if let Some(Value::Function(f)) =
                        props.get("onChange").or_else(|| props.get("onInput"))
                    {
                        let f = f.clone();
                        let idx = register_pick_handler(ctx.root_id, Rc::new(move |i| {
                            let _ = f(&[Value::Number(i as f64)]);
                        })) as isize;
                        popup.setTag(idx);
                        unsafe {
                            let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
                            popup.setTarget(Some(&*p));
                            popup.setAction(Some(sel!(tishPick:)));
                        }
                    }
                    let row_h = 28.0;
                    place(&popup, ix, iy, iw, row_h);
                    freeze_autoresizing_for_manual_frames(&popup);
                    parent.addSubview(&popup);
                    pt + row_h + pb
                }
                "radio" => {
                    let opts = options_strings(&props);
                    let cur = props_f64(&props, &["value", "selected"], 0.0) as usize;
                    let mut y = iy;
                    let mut hsum = 0.0;
                    for (i, label) in opts.iter().enumerate() {
                        let btn = NSButton::new(ctx.mtm);
                        btn.setButtonType(NSButtonType::Radio);
                        btn.setTitle(&NSString::from_str(label));
                        if i == cur {
                            btn.setState(NSControlStateValueOn);
                        }
                        if let Some(Value::Function(f)) = props.get("onChange") {
                            let f = f.clone();
                            let ii = i as f64;
                            let idx = register_bool_handler(ctx.root_id, Rc::new(move |on| {
                                if on {
                                    let _ = f(&[Value::Number(ii)]);
                                }
                            })) as isize;
                            btn.setTag(idx);
                            unsafe {
                                let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
                                btn.setTarget(Some(&*p));
                                btn.setAction(Some(sel!(tishBool:)));
                            }
                        }
                        let h = 24.0;
                        place(&btn, ix, y, iw, h);
                        freeze_autoresizing_for_manual_frames(&btn);
                        parent.addSubview(&btn);
                        y += h;
                        hsum += h;
                    }
                    pt + hsum + pb
                }
                "image" => {
                    let src = props_string(&props, &["src", "path", "url"]).unwrap_or_default();
                    let use_symbol = props_bool(&props, &["symbol", "sfSymbol", "sf_symbol"]);
                    let img = if use_symbol {
                        let sym = NSString::from_str(&src);
                        NSImage::imageWithSystemSymbolName_accessibilityDescription(&sym, None)
                    } else if src.starts_with('/') || src.contains('/') {
                        let p = NSString::from_str(&src);
                        let alloc = NSImage::alloc();
                        NSImage::initWithContentsOfFile(alloc, &p)
                    } else {
                        let name = NSString::from_str(&src);
                        NSImage::imageNamed(&name)
                    };
                    let iv = NSImageView::new(ctx.mtm);
                    if let Some(im) = img {
                        let im = if use_symbol {
                            let scale_s = props_string(&props, &["symbolScale", "iconScale"])
                                .map(|s| s.to_ascii_lowercase());
                            let sc = match scale_s.as_deref() {
                                Some("medium") => NSImageSymbolScale::Medium,
                                Some("large") => NSImageSymbolScale::Large,
                                _ => NSImageSymbolScale::Small,
                            };
                            let cfg = NSImageSymbolConfiguration::configurationWithScale(sc);
                            im.imageWithSymbolConfiguration(&cfg).unwrap_or(im)
                        } else {
                            im
                        };
                        iv.setImage(Some(&im));
                    }
                    iv.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
                    if let Some(ts) = props_string(&props, &["tint", "symbolTint", "symbol_tint"]) {
                        if let Some(col) = resolve_ns_color(&ts) {
                            iv.setContentTintColor(Some(&col));
                        }
                    }
                    let ih = props_f64(&props, &["height", "h"], 120.0);
                    place(&iv, ix, iy, iw, ih);
                    freeze_autoresizing_for_manual_frames(&iv);
                    apply_layer_style_to_view(&iv, &props);
                    parent.addSubview(&iv);
                    pt + ih + pb
                }
                "tooltip" => {
                    let tip = props_string(&props, &["title", "tooltip", "label"]).unwrap_or_default();
                    let wrap = FlippedDocumentView::new(
                        ctx.mtm,
                        NSRect::new(CGPoint::ZERO, CGSize::new(iw.max(1.0), 0.0)),
                    );
                    let wrap_ns: &NSView =
                        unsafe { &*std::ptr::from_ref(&*wrap).cast::<NSView>() };
                    if !tip.is_empty() {
                        wrap_ns.setToolTip(Some(&NSString::from_str(&tip)));
                    }
                    let h_child = if children.len() == 1 { avail_h } else { None };
                    let mut y = 0.0;
                    let mut hsum = 0.0;
                    for c in &children {
                        let h = commit_vnode(c, wrap_ns, 0.0, y, iw, h_child, ctx);
                        y += h;
                        hsum += h;
                    }
                    apply_layer_style_to_view(wrap_ns, &props);
                    place(wrap_ns, ix, iy, iw, hsum);
                    freeze_autoresizing_for_manual_frames(wrap_ns);
                    parent.addSubview(wrap_ns);
                    pt + hsum + pb
                }
                "list" => {
                    let th = scroll_outer_height(&props, avail_h);
                    let rows: Vec<String> = match props.get("rows") {
                        Some(Value::Array(a)) => a
                            .borrow()
                            .iter()
                            .map(|v| v.to_display_string())
                            .collect(),
                        _ => vec![],
                    };
                    let body = rows.join("\n");
                    let scroll = NSScrollView::new(ctx.mtm);
                    scroll.setDrawsBackground(props_bool(
                        &props,
                        &["drawsBackground", "draws_background"],
                    ));
                    scroll.setHasVerticalScroller(true);
                    tune_scroll_view_chrome(&scroll, true, false);
                    strip_scroll_content_insets(&scroll);
                    let tf =
                        NSTextField::wrappingLabelWithString(&NSString::from_str(&body), ctx.mtm);
                    apply_static_label_text_field(&tf, &props, ctx.mtm);
                    tf.setFrameSize(CGSize::new(iw.max(1.0), (th - 8.0).max(40.0)));
                    scroll.setDocumentView(Some(&tf));
                    place(&scroll, ix, iy, iw, th);
                    parent.addSubview(&scroll);
                    sync_scroll_view_for_document(&scroll, &tf);
                    apply_scroll_content_right_gutter(
                        &scroll,
                        scroll_scroller_right_gutter_from_props(&props),
                    );
                    pt + th + pb
                }
                "text_editor" => {
                    let base_h = scroll_outer_height(&props, avail_h);
                    let min_h = props_f64(&props, &["minHeight", "min_height"], 120.0);
                    let th = base_h.max(min_h);
                    let scroll = NSScrollView::new(ctx.mtm);
                    scroll.setDrawsBackground(props_bool(
                        &props,
                        &["drawsBackground", "draws_background"],
                    ));
                    scroll.setHasVerticalScroller(true);
                    tune_scroll_view_chrome(&scroll, true, false);
                    strip_scroll_content_insets(&scroll);
                    let clip_frame = scroll.contentView().frame();
                    let flipped_clip = FlippedClipView::new(ctx.mtm, clip_frame);
                    flipped_clip.setAutomaticallyAdjustsContentInsets(false);
                    flipped_clip.setContentInsets(ZERO_EDGE_INSETS);
                    scroll.setContentView(&flipped_clip);
                    let tv = NSTextView::new(ctx.mtm);
                    tv.setEditable(true);
                    tv.setRichText(false);
                    apply_nstext_view_document_background_from_props(&tv, &props);
                    let fg = NSColor::labelColor();
                    tv.setTextColor(Some(&fg));
                    tv.setInsertionPointColor(Some(&fg));
                    let fs = props_f64(&props, &["fontSize", "font_size"], 13.0);
                    let font = NSFont::systemFontOfSize(fs as CGFloat);
                    tv.setFont(Some(&font));
                    let initial =
                        props_string(&props, &["value", "defaultValue"]).unwrap_or_default();
                    let has_change = props
                        .get("onChange")
                        .or_else(|| props.get("onInput"))
                        .is_some();
                    if has_change {
                        tv.setDelegate(None);
                    }
                    tv.setString(&NSString::from_str(&initial));
                    if let Some(Value::Function(f)) =
                        props.get("onChange").or_else(|| props.get("onInput"))
                    {
                        let f = f.clone();
                        let idx = register_text_change_handler(
                            ctx.root_id,
                            Rc::new(move |s: String| {
                                let _ = f(&[Value::String(s.into())]);
                            }),
                        ) as isize;
                        install_text_change_tag_on_text_view(&tv, idx);
                    }
                    tv.setFrameSize(CGSize::new(iw.max(1.0), (th - 8.0).max(40.0)));
                    scroll.setDocumentView(Some(&tv));
                    place(&scroll, ix, iy, iw, th);
                    parent.addSubview(&scroll);
                    if has_change {
                        tv.setDelegate(Some(ProtocolObject::from_ref(
                            &*ctx.text_view_delegate,
                        )));
                    }
                    sync_scroll_view_for_document(&scroll, &tv);
                    apply_scroll_content_right_gutter(
                        &scroll,
                        scroll_scroller_right_gutter_from_props(&props),
                    );
                    pt + th + pb
                }
                "markdown_text" => {
                    let base_h = scroll_outer_height(&props, avail_h);
                    let min_h = props_f64(&props, &["minHeight", "min_height"], 120.0);
                    let th = base_h.max(min_h);
                    let scroll = NSScrollView::new(ctx.mtm);
                    scroll.setDrawsBackground(props_bool(
                        &props,
                        &["drawsBackground", "draws_background"],
                    ));
                    scroll.setHasVerticalScroller(true);
                    tune_scroll_view_chrome(&scroll, true, false);
                    strip_scroll_content_insets(&scroll);
                    let clip_frame = scroll.contentView().frame();
                    let flipped_clip = FlippedClipView::new(ctx.mtm, clip_frame);
                    flipped_clip.setAutomaticallyAdjustsContentInsets(false);
                    flipped_clip.setContentInsets(ZERO_EDGE_INSETS);
                    scroll.setContentView(&flipped_clip);
                    let tv = NSTextView::new(ctx.mtm);
                    tv.setDelegate(None);
                    let fs = props_f64(&props, &["fontSize", "font_size"], 13.0);
                    apply_markdown_text_view_chrome(&tv, fs as CGFloat);
                    apply_nstext_view_document_background_from_props(&tv, &props);
                    let md = props_string(&props, &["markdown", "value", "defaultValue"])
                        .unwrap_or_default();
                    set_text_view_markdown(&tv, &md, ctx.mtm);
                    tv.setFrameSize(CGSize::new(iw.max(1.0), (th - 8.0).max(40.0)));
                    scroll.setDocumentView(Some(&tv));
                    place(&scroll, ix, iy, iw, th);
                    parent.addSubview(&scroll);
                    sync_scroll_view_for_document(&scroll, &tv);
                    apply_scroll_content_right_gutter(
                        &scroll,
                        scroll_scroller_right_gutter_from_props(&props),
                    );
                    pt + th + pb
                }
                "tabs" => {
                    let th = props_f64(&props, &["height", "h"], 200.0);
                    let tabv = NSTabView::new(ctx.mtm);
                    for c in &children {
                        let (label, body) = match c {
                            Value::Object(o) => {
                                let m = &o.borrow().strings;
                                let is_tab = matches!(
                                    m.get("tag"),
                                    Some(Value::String(s)) if {
                                        let t = s.as_ref();
                                        t == "tab" || t == "Tab"
                                    }
                                );
                                let p = vnode_props(&m);
                                let lbl = props_string(&p, &["label", "title", "name"])
                                    .unwrap_or_else(|| "Tab".into());
                                let ch = if is_tab {
                                    vnode_children(&m)
                                } else {
                                    vec![c.clone()]
                                };
                                (lbl, ch)
                            }
                            _ => ("Tab".into(), vec![c.clone()]),
                        };
                        let item = NSTabViewItem::new();
                        item.setLabel(&NSString::from_str(&label));
                        let pane = FlippedDocumentView::new(
                            ctx.mtm,
                            NSRect::new(CGPoint::ZERO, CGSize::new(iw, (th - 28.0).max(40.0))),
                        );
                        let mut y = 0.0;
                        for cc in &body {
                            let h = commit_vnode(cc, &pane, 0.0, y, iw, None, ctx);
                            y += h;
                        }
                        item.setView(Some(&pane));
                        tabv.addTabViewItem(&item);
                    }
                    place(&tabv, ix, iy, iw, th);
                    parent.addSubview(&tabv);
                    pt + th + pb
                }
                "split" => {
                    let th = scroll_outer_height(&props, avail_h);
                    let split = FlippedSplitView::new(ctx.mtm);
                    split.setAutoresizingMask(
                        NSAutoresizingMaskOptions::ViewWidthSizable
                            | NSAutoresizingMaskOptions::ViewHeightSizable,
                    );
                    split.setVertical(split_uses_vertical_divider(&props));
                    split.setDividerStyle(split_divider_style(&props));
                    let (w0, h0, w1, h1, pos) = split_pane_layout(&props, iw, th);
                    let pane_v = split_pane_vnodes(&children);
                    for (i, c) in pane_v.iter().take(2).enumerate() {
                        let (pw, ph) = if i == 0 { (w0, h0) } else { (w1, h1) };
                        let pane = FlippedDocumentView::new(
                            ctx.mtm,
                            NSRect::new(CGPoint::ZERO, CGSize::new(pw, ph)),
                        );
                        pane.setAutoresizingMask(
                            NSAutoresizingMaskOptions::ViewWidthSizable
                                | NSAutoresizingMaskOptions::ViewHeightSizable,
                        );
                        let _ = commit_vnode(c, &pane, 0.0, 0.0, pw, Some(ph), ctx);
                        split.addSubview(&pane);
                    }
                    // Divider position uses the split's width/height; set frame first or AppKit can
                    // collapse the trailing pane when the split still has a near-zero size.
                    place(&split, ix, iy, iw, th);
                    parent.addSubview(&split);
                    split.setPosition_ofDividerAtIndex(pos, 0);
                    split.adjustSubviews();
                    snap_flipped_split_panes_full_height(&*split);
                    pt + th + pb
                }
                "visual_effect" => {
                    warn_unknown_props("VisualEffect", &props, VISUAL_EFFECT_PROP_ALLOWLIST);
                    let vh = scroll_outer_height(&props, avail_h);
                    let intrinsic_h = visual_effect_intrinsic_outer_height(avail_h, &props);
                    let child_avail = if intrinsic_h { None } else { Some(vh) };
                    let doc_seed_h = if intrinsic_h { 0.0 } else { vh };
                    let gutter = props_f64(
                        &props,
                        &["scrollerGutterRight", "scroller_gutter_right"],
                        0.0,
                    )
                    .max(0.0);
                    let inner_w = (iw - gutter).max(1.0);
                    let fx = FlippedVisualEffectView::new(
                        ctx.mtm,
                        NSRect::new(CGPoint::ZERO, CGSize::new(inner_w, 0.0)),
                    );
                    apply_visual_effect_view_from_props(&*fx, &props);
                    fx.set_right_gutter(gutter);
                    if gutter > 0.0 {
                        fx.setAutoresizingMask(
                            NSAutoresizingMaskOptions::ViewMaxXMargin
                                | NSAutoresizingMaskOptions::ViewMinYMargin
                                | NSAutoresizingMaskOptions::ViewHeightSizable,
                        );
                    } else {
                        fx.setAutoresizingMask(
                            NSAutoresizingMaskOptions::ViewWidthSizable
                                | NSAutoresizingMaskOptions::ViewHeightSizable,
                        );
                    }
                    let doc = FlippedDocumentView::new(
                        ctx.mtm,
                        NSRect::new(CGPoint::ZERO, CGSize::new(inner_w, doc_seed_h)),
                    );
                    let mut dh = 0.0;
                    let mut dy = 0.0;
                    // Pass viewport height so nested `ScrollView` / `Split` with `height="fill"` use the
                    // effect’s height, not `scroll_outer_height`’s default (200).
                    for c in &children {
                        let h = commit_vnode(c, &doc, 0.0, dy, inner_w, child_avail, ctx);
                        dy += h;
                        dh += h;
                    }
                    let content_h = dh.max(1.0);
                    let fh = if intrinsic_h {
                        content_h
                    } else {
                        content_h.max(vh)
                    };
                    let doc_view: &NSView = unsafe { &*std::ptr::from_ref(&*doc).cast::<NSView>() };
                    fx.addSubview(doc_view);
                    parent.addSubview(&fx);
                    place(&fx, ix, iy, inner_w, fh);
                    place_visual_effect_document(&*fx, doc_view, inner_w, content_h, fh);
                    freeze_autoresizing_for_manual_frames(doc_view);
                    fx.layoutSubtreeIfNeeded();
                    pt + fh + pb
                }
                "webview" => {
                    let th = scroll_outer_height(&props, avail_h);
                    let src = props_string(&props, &["src", "url"]).unwrap_or_default();
                    let frame = NSRect::new(CGPoint::ZERO, CGSize::new(iw, th));
                    let wv = unsafe { WKWebView::initWithFrame(WKWebView::alloc(ctx.mtm), frame) };
                    if let Some(url) = NSURL::URLWithString(&NSString::from_str(&src)) {
                        let req = NSURLRequest::requestWithURL(&url);
                        unsafe {
                            let _ = WKWebView::loadRequest(&*wv, &req);
                        }
                    }
                    place(&*wv, ix, iy, iw, th);
                    parent.addSubview(&*wv);
                    pt + th + pb
                }
                "macos_window" => {
                    let wrap = Value::object(map.clone());
                    if let Some(inner) = macos_window_content(&wrap) {
                        commit_vnode(&inner, parent, ix, iy, iw, avail_h, ctx)
                    } else {
                        let boxv = FlippedDocumentView::new(
                            ctx.mtm,
                            NSRect::new(CGPoint::ZERO, CGSize::new(iw.max(1.0), 0.0)),
                        );
                        let boxv_ns: &NSView =
                            unsafe { &*std::ptr::from_ref(&*boxv).cast::<NSView>() };
                        let mut y = 0.0;
                        let mut hsum = 0.0;
                        let h_child = if children.len() == 1 { avail_h } else { None };
                        for c in &children {
                            let h = commit_vnode(c, boxv_ns, 0.0, y, iw, h_child, ctx);
                            y += h;
                            hsum += h;
                        }
                        apply_layer_style_to_view(boxv_ns, &props);
                        place(boxv_ns, ix, iy, iw, hsum);
                        freeze_autoresizing_for_manual_frames(boxv_ns);
                        parent.addSubview(boxv_ns);
                        pt + hsum + pb
                    }
                }
                _ => {
                    let boxv = FlippedDocumentView::new(
                        ctx.mtm,
                        NSRect::new(CGPoint::ZERO, CGSize::new(iw.max(1.0), 0.0)),
                    );
                    let boxv_ns: &NSView =
                        unsafe { &*std::ptr::from_ref(&*boxv).cast::<NSView>() };
                    let mut y = 0.0;
                    let mut hsum = 0.0;
                    let h_child = if children.len() == 1 { avail_h } else { None };
                    for c in &children {
                        let h = commit_vnode(c, boxv_ns, 0.0, y, iw, h_child, ctx);
                        y += h;
                        hsum += h;
                    }
                    apply_layer_style_to_view(boxv_ns, &props);
                    place(boxv_ns, ix, iy, iw, hsum);
                    freeze_autoresizing_for_manual_frames(boxv_ns);
                    parent.addSubview(boxv_ns);
                    pt + hsum + pb
                }
            }
        }
        _ => {
            let tf = NSTextField::labelWithString(&NSString::from_str(&v.to_display_string()), ctx.mtm);
            apply_static_label_text_field(&tf, &ObjectMap::default(), ctx.mtm);
            let h = single_line_label_height_after_style(&tf, &ObjectMap::default());
            place(&tf, x, y_top, avail_w, h);
            freeze_autoresizing_for_manual_frames(&tf);
            parent.addSubview(&tf);
            h
        }
    }
}

fn strip_macos_window(v: &Value) -> Value {
    macos_window_content(v).unwrap_or_else(|| v.clone())
}

pub fn commit_root_into(
    v: &Value,
    root: &FlippedRootView,
    width: f64,
    viewport_h: f64,
    ctx: &BuildCtx,
    prev: Option<&Value>,
) {
    commit_root_into_with_layout(v, root, width, viewport_h, ctx, prev, true);
}

pub fn commit_root_into_with_layout(
    v: &Value,
    root: &FlippedRootView,
    width: f64,
    viewport_h: f64,
    ctx: &BuildCtx,
    prev: Option<&Value>,
    run_layout_now: bool,
) {
    let v_eff = strip_macos_window(v);
    let prev_eff = prev.map(|p| strip_macos_window(p));
    let root_ns: &NSView = unsafe { &*std::ptr::from_ref(root).cast::<NSView>() };
    commit_pane_into(
        &v_eff,
        root_ns,
        width,
        viewport_h,
        ctx,
        prev_eff.as_ref(),
        true,
        true,
        run_layout_now,
    );
}

/// Commit one vnode tree into a flipped root (`FlippedRootView` or `FlippedSplitPaneRootView`).
///
/// When `set_root_frame` is false (panes inside `NSSplitViewController`), do not call
/// `setFrameSize` on `root` — Auto Layout owns the pane geometry; forcing the frame breaks layout
/// and leaves content invisible.
pub(super) fn commit_pane_into(
    v: &Value,
    root: &NSView,
    width: f64,
    viewport_h: f64,
    ctx: &BuildCtx,
    prev: Option<&Value>,
    clear_handlers_on_full_rebuild: bool,
    set_root_frame: bool,
    run_layout_now: bool,
) {
    let root_h = viewport_h.max(300.0);
    if let Some(p) = prev {
        if super::patch::try_patch_vtree(p, v, root, width, viewport_h, ctx).is_some() {
            if set_root_frame {
                root.setFrameSize(CGSize::new(width, root_h));
            }
            if run_layout_now {
                root.layoutSubtreeIfNeeded();
                resync_all_scroll_views_under(root);
            } else {
                root.setNeedsLayout(true);
            }
            return;
        }
    }
    if clear_handlers_on_full_rebuild {
        super::handlers::clear_handlers_for_root(ctx.root_id);
    }
    clear_subviews(root);
    let _content_h = commit_vnode(v, root, 0.0, 0.0, width, Some(viewport_h), ctx);
    if set_root_frame {
        root.setFrameSize(CGSize::new(width, root_h));
    }
    if run_layout_now {
        root.layoutSubtreeIfNeeded();
        resync_all_scroll_views_under(root);
    } else {
        root.setNeedsLayout(true);
    }
}

/// Root must be `sidebar_window` with two children.
pub(super) fn commit_sidebar_window_into(
    v: &Value,
    sidebar_root: &NSView,
    detail_root: &NSView,
    sw: f64,
    sh: f64,
    dw: f64,
    dh: f64,
    ctx: &BuildCtx,
    prev: Option<&Value>,
) {
    let Some((sidebar_v, detail_v)) = sidebar_window_children(v) else {
        return;
    };
    let prev_s = prev.and_then(|p| sidebar_window_children(p).map(|(a, _)| a));
    let prev_d = prev.and_then(|p| sidebar_window_children(p).map(|(_, b)| b));

    if let Some(p) = prev {
        if super::patch::try_patch_sidebar_vtree(
            p, v, sidebar_root, detail_root, sw, sh, dw, dh, ctx,
        )
        .is_some()
        {
            sidebar_root.layoutSubtreeIfNeeded();
            detail_root.layoutSubtreeIfNeeded();
            resync_all_scroll_views_under(sidebar_root);
            resync_all_scroll_views_under(detail_root);
            return;
        }
    }
    super::handlers::clear_handlers_for_root(ctx.root_id);
    commit_pane_into(
        &sidebar_v,
        sidebar_root,
        sw,
        sh,
        ctx,
        prev_s.as_ref(),
        false,
        false,
        true,
    );
    commit_pane_into(
        &detail_v,
        detail_root,
        dw,
        dh,
        ctx,
        prev_d.as_ref(),
        false,
        false,
        true,
    );
}

#[cfg(test)]
mod sidebar_window_children_tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use tishlang_core::{ObjectMap, Value};

    use super::sidebar_window_children;

    fn vnode(tag: &str, children: Vec<Value>) -> Value {
        let mut m = ObjectMap::default();
        m.insert(Arc::from("tag"), Value::String(tag.into()));
        m.insert(
            Arc::from("children"),
            Value::array(children),
        );
        m.insert(Arc::from("props"), Value::Null);
        m.insert(Arc::from("_el"), Value::Null);
        Value::object(m)
    }

    #[test]
    fn ignores_jsx_whitespace_string_siblings() {
        let root = vnode(
            "sidebar_window",
            vec![
                vnode("scrollable", vec![]),
                Value::String("\n      ".into()),
                vnode("scrollable", vec![]),
            ],
        );
        let (a, b) = sidebar_window_children(&root).expect("two pane vnodes");
        let tag = |v: &Value| match v {
            Value::Object(o) => o
                .borrow()
                .get("tag")
                .map(|t| t.to_display_string())
                .unwrap_or_default(),
            _ => String::new(),
        };
        assert_eq!(tag(&a), "scrollable");
        assert_eq!(tag(&b), "scrollable");
    }
}

#[cfg(test)]
mod split_pane_vnodes_tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use tishlang_core::{ObjectMap, Value};

    use super::split_pane_vnodes;

    fn vnode(tag: &str) -> Value {
        let mut m = ObjectMap::default();
        m.insert(Arc::from("tag"), Value::String(tag.into()));
        m.insert(
            Arc::from("children"),
            Value::array(vec![]),
        );
        m.insert(Arc::from("props"), Value::Null);
        m.insert(Arc::from("_el"), Value::Null);
        Value::object(m)
    }

    fn tag(v: &Value) -> String {
        match v {
            Value::Object(o) => o
                .borrow()
                .get("tag")
                .map(|t| t.to_display_string())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    #[test]
    fn skips_leading_jsx_text_so_second_pane_is_not_dropped() {
        let ch = vec![
            Value::String("\n      ".into()),
            vnode("scroll_view"),
            vnode("text_editor"),
        ];
        let panes = split_pane_vnodes(&ch);
        assert_eq!(panes.len(), 2);
        assert_eq!(tag(&panes[0]), "scroll_view");
        assert_eq!(tag(&panes[1]), "text_editor");
    }
}

#[cfg(test)]
mod effective_props_tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use tishlang_core::{ObjectMap, Value};

    use super::effective_props;

    fn style_obj(pairs: &[(&str, Value)]) -> Value {
        let mut m = ObjectMap::default();
        for (k, v) in pairs {
            m.insert(Arc::from(*k), v.clone());
        }
        Value::object(m)
    }

    #[test]
    fn style_overrides_top_level_for_same_key() {
        let mut raw = ObjectMap::default();
        raw.insert(Arc::from("padding"), Value::Number(1.0));
        raw.insert(
            Arc::from("style"),
            style_obj(&[("padding", Value::Number(8.0))]),
        );
        let e = effective_props(&raw);
        assert_eq!(
            e.get("padding").and_then(|v| v.as_number()),
            Some(8.0)
        );
    }

    #[test]
    fn top_level_kept_when_not_in_style() {
        let mut raw = ObjectMap::default();
        raw.insert(Arc::from("height"), Value::String("fill".into()));
        raw.insert(
            Arc::from("style"),
            style_obj(&[("backgroundColor", Value::String("#ff0000".into()))]),
        );
        let e = effective_props(&raw);
        match e.get("height") {
            Some(Value::String(s)) => assert_eq!(s.as_ref(), "fill"),
            o => panic!("expected height fill, got {:?}", o),
        }
        match e.get("backgroundColor") {
            Some(Value::String(s)) => assert_eq!(s.as_ref(), "#ff0000"),
            o => panic!("expected backgroundColor, got {:?}", o),
        }
    }
}
