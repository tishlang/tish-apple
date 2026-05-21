//! In-place AppKit updates when vnode shape matches the previous commit.

use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{sel, AnyThread, ClassType};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBox, NSButton, NSColor, NSControlStateValueOff,
    NSControlStateValueOn, NSFont, NSImage, NSImageScaling, NSImageSymbolConfiguration,
    NSImageSymbolScale, NSImageView, NSProgressIndicator,
    NSPopUpButton, NSScrollView, NSSecureTextField, NSSlider,
    NSSplitView, NSSwitch, NSTabView, NSTextField, NSTextView, NSVisualEffectView, NSView,
};
use objc2_core_foundation::{CGFloat, CGSize};
use objc2_foundation::{NSObjectProtocol, NSString, NSURL, NSURLRequest};
use objc2_web_kit::WKWebView;
use tishlang_core::{ObjectMap, Value};
use tishlang_ui::runtime::is_fragment_tag;

use super::build::{
    apply_button_chrome, effective_props, freeze_autoresizing_for_manual_frames, options_strings,
    padding_insets, place, place_visual_effect_document, props_bool, props_f64, props_string,
    row_child_widths, row_cross_align,
    row_shell_outer_height, row_shell_reposition_children, row_wants_click_overlay, RowCrossAlign,
    scroll_outer_height, split_divider_style, split_pane_layout, split_pane_vnodes,
    split_uses_vertical_divider, visual_effect_intrinsic_outer_height,
    apply_scroll_content_right_gutter, apply_scroll_scroller_top_inset,
    label_text_from_children, scroll_scroller_right_gutter_from_props, sync_scroll_view_for_document,
    apply_visual_effect_view_from_props, text_view_set_string_without_delegate_notice,
    vnode_children, vnode_props, BuildCtx,
};
use super::flipped::{snap_flipped_split_panes_full_height, FlippedVisualEffectView};
use super::style::{
    apply_layer_style_to_view, apply_nstext_view_document_background_from_props,
    apply_static_label_text_field, has_container_layer_style, resolve_ns_color,
    single_line_label_height_after_style,
};
use super::markdown_view::{apply_markdown_text_view_chrome, set_text_view_markdown};
use super::tag::canonical_host_tag;
use super::handlers::{
    decode_control_tag, install_text_change_tag_on_text_view, register_bool_handler,
    register_click_handler, register_f64_handler, register_pick_handler,
    register_text_change_handler, text_change_tag_from_text_view, update_bool_handler,
    update_click_handler, update_f64_handler, update_pick_handler, update_text_change_handler,
};
fn wire_on_click_patch(props: &ObjectMap, btn: &NSButton, ctx: &BuildCtx, existing_tag: isize) {
    if let Some(Value::Function(f)) = props.get("onClick").or_else(|| props.get("onclick")) {
        let f = f.clone();
        let idx = if existing_tag >= 0 {
            let (rid, slot) = decode_control_tag(existing_tag);
            update_click_handler(
                rid,
                slot,
                Rc::new(move || {
                    let _ = f(&[]);
                }),
            )
        } else {
            register_click_handler(ctx.root_id, Rc::new(move || {
                let _ = f(&[]);
            }))
        };
        btn.setTag(idx);
        unsafe {
            let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
            btn.setTarget(Some(&*p));
            btn.setAction(Some(sel!(jsxClick:)));
        }
    }
}

fn tab_label_and_children(tab: &Value) -> (String, Vec<Value>) {
    match tab {
        Value::Object(o) => {
            let m = &o.borrow().strings;
            let is_tab = matches!(
                m.get("tag"),
                Some(Value::String(s)) if canonical_host_tag(s.as_ref()) == "tab"
            );
            let p = vnode_props(&m);
            let lbl = props_string(&p, &["label", "title", "name"]).unwrap_or_else(|| "Tab".into());
            let ch = if is_tab {
                vnode_children(&m)
            } else {
                vec![tab.clone()]
            };
            (lbl, ch)
        }
        _ => ("Tab".into(), vec![tab.clone()]),
    }
}

pub fn vnode_same_shape(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(_), Value::String(_)) => true,
        (Value::Object(oa), Value::Object(ob)) => {
            let ma = &oa.borrow().strings;
            let mb = &ob.borrow().strings;
            let ta = ma.get("tag").unwrap_or(&Value::Null);
            if is_fragment_tag(ta) {
                let ca = vnode_children(&ma);
                let cb = vnode_children(&mb);
                return ca.len() == cb.len()
                    && ca
                        .iter()
                        .zip(cb.iter())
                        .all(|(x, y)| vnode_same_shape(x, y));
            }
            let sa = ma
                .get("tag")
                .and_then(|t| match t {
                    Value::String(s) => Some(s.as_ref().to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            let sb = mb
                .get("tag")
                .and_then(|t| match t {
                    Value::String(s) => Some(s.as_ref().to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            let ca = canonical_host_tag(sa.as_str());
            let cb = canonical_host_tag(sb.as_str());
            if ca != cb {
                return false;
            }
            if ca == "row" {
                let po = effective_props(&vnode_props(&ma));
                let pn = effective_props(&vnode_props(&mb));
                let shell_o =
                    has_container_layer_style(&po) || row_wants_click_overlay(&po);
                let shell_n =
                    has_container_layer_style(&pn) || row_wants_click_overlay(&pn);
                if shell_o != shell_n {
                    return false;
                }
            }
            let ca = vnode_children(&ma);
            let cb = vnode_children(&mb);
            ca.len() == cb.len()
                && ca
                    .iter()
                    .zip(cb.iter())
                    .all(|(x, y)| vnode_same_shape(x, y))
        }
        _ => false,
    }
}

fn subview(parent: &NSView, i: usize) -> Option<Retained<NSView>> {
    let subs = parent.subviews();
    let n = subs.count() as usize;
    if i >= n {
        return None;
    }
    Some(subs.objectAtIndex(i))
}

unsafe fn as_scroll(v: &NSView) -> Option<&NSScrollView> {
    if v.isKindOfClass(NSScrollView::class()) {
        Some(&*(std::ptr::from_ref(v).cast::<NSScrollView>()))
    } else {
        None
    }
}

unsafe fn as_slider(v: &NSView) -> Option<&NSSlider> {
    if v.isKindOfClass(NSSlider::class()) {
        Some(&*(std::ptr::from_ref(v).cast::<NSSlider>()))
    } else {
        None
    }
}

unsafe fn as_switch(v: &NSView) -> Option<&NSSwitch> {
    if v.isKindOfClass(NSSwitch::class()) {
        Some(&*(std::ptr::from_ref(v).cast::<NSSwitch>()))
    } else {
        None
    }
}

unsafe fn as_popup(v: &NSView) -> Option<&NSPopUpButton> {
    if v.isKindOfClass(NSPopUpButton::class()) {
        Some(&*(std::ptr::from_ref(v).cast::<NSPopUpButton>()))
    } else {
        None
    }
}

unsafe fn as_progress(v: &NSView) -> Option<&NSProgressIndicator> {
    if v.isKindOfClass(NSProgressIndicator::class()) {
        Some(&*(std::ptr::from_ref(v).cast::<NSProgressIndicator>()))
    } else {
        None
    }
}

unsafe fn as_image_view(v: &NSView) -> Option<&NSImageView> {
    if v.isKindOfClass(NSImageView::class()) {
        Some(&*(std::ptr::from_ref(v).cast::<NSImageView>()))
    } else {
        None
    }
}

unsafe fn as_webview(v: &NSView) -> Option<&WKWebView> {
    if v.isKindOfClass(WKWebView::class()) {
        Some(&*(std::ptr::from_ref(v).cast::<WKWebView>()))
    } else {
        None
    }
}

unsafe fn as_tabview(v: &NSView) -> Option<&NSTabView> {
    if v.isKindOfClass(NSTabView::class()) {
        Some(&*(std::ptr::from_ref(v).cast::<NSTabView>()))
    } else {
        None
    }
}

unsafe fn as_split(v: &NSView) -> Option<&NSSplitView> {
    if v.isKindOfClass(NSSplitView::class()) {
        Some(&*(std::ptr::from_ref(v).cast::<NSSplitView>()))
    } else {
        None
    }
}

unsafe fn as_visual_effect(v: &NSView) -> Option<&NSVisualEffectView> {
    if v.isKindOfClass(NSVisualEffectView::class()) {
        Some(&*(std::ptr::from_ref(v).cast::<NSVisualEffectView>()))
    } else {
        None
    }
}

pub fn try_patch_vtree(
    old: &Value,
    new: &Value,
    root: &NSView,
    width: f64,
    viewport_h: f64,
    ctx: &BuildCtx,
) -> Option<f64> {
    if !vnode_same_shape(old, new) {
        return None;
    }
    let mut slot = 0usize;
    let h = patch_vnode(
        old,
        new,
        root,
        &mut slot,
        0.0,
        0.0,
        width,
        Some(viewport_h),
        ctx,
    )
    .ok()?;
    let n = root.subviews().count() as usize;
    if slot != n {
        return None;
    }
    Some(h)
}

/// Patch both panes of a `sidebar_window` root; `prev` / `new` must be `sidebar_window` vnodes.
pub fn try_patch_sidebar_vtree(
    old: &Value,
    new: &Value,
    sidebar_root: &NSView,
    detail_root: &NSView,
    sw: f64,
    sh: f64,
    dw: f64,
    dh: f64,
    ctx: &BuildCtx,
) -> Option<()> {
    if !vnode_same_shape(old, new) {
        return None;
    }
    let (old_s, old_d) = super::build::sidebar_window_children(old)?;
    let (new_s, new_d) = super::build::sidebar_window_children(new)?;
    try_patch_vtree(&old_s, &new_s, sidebar_root, sw, sh, ctx)?;
    try_patch_vtree(&old_d, &new_d, detail_root, dw, dh, ctx)?;
    Some(())
}

fn patch_vnode(
    old: &Value,
    new: &Value,
    parent: &NSView,
    slot: &mut usize,
    x: f64,
    y_top: f64,
    avail_w: f64,
    avail_h: Option<f64>,
    ctx: &BuildCtx,
) -> Result<f64, ()> {
    match (old, new) {
        (Value::String(sa), Value::String(sb)) => {
            let v = subview(parent, *slot).ok_or(())?;
            if !v.isKindOfClass(NSTextField::class()) {
                return Err(());
            }
            let tf: &NSTextField = unsafe { &*(std::ptr::from_ref(&*v).cast()) };
            if sa.as_ref().trim() != sb.as_ref().trim() {
                tf.setStringValue(&NSString::from_str(sb.as_ref().trim()));
            }
            apply_static_label_text_field(tf, &ObjectMap::default(), ctx.mtm);
            let h = single_line_label_height_after_style(tf, &ObjectMap::default());
            place(&*v, x, y_top, avail_w, h);
            freeze_autoresizing_for_manual_frames(&*v);
            *slot += 1;
            Ok(h)
        }
        (Value::Object(oa), Value::Object(ob)) => {
            let om = &oa.borrow().strings;
            if is_fragment_tag(om.get("tag").unwrap_or(&Value::Null)) {
                let nm = &ob.borrow().strings;
                let ch_old = vnode_children(&om);
                let ch_new = vnode_children(&nm);
                if ch_old.len() != ch_new.len() {
                    return Err(());
                }
                let h_pass = if ch_old.len() == 1 { avail_h } else { None };
                let mut y = y_top;
                let mut hsum = 0.0;
                for (co, cn) in ch_old.iter().zip(ch_new.iter()) {
                    let h = patch_vnode(co, cn, parent, slot, x, y, avail_w, h_pass, ctx)?;
                    y += h;
                    hsum += h;
                }
                return Ok(hsum);
            }
            let tag_o = om
                .get("tag")
                .and_then(|t| match t {
                    Value::String(s) => Some(s.as_ref().to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            let nm = &ob.borrow().strings;
            let tag_n = nm
                .get("tag")
                .and_then(|t| match t {
                    Value::String(s) => Some(s.as_ref().to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            if canonical_host_tag(tag_o.as_str()) != canonical_host_tag(tag_n.as_str()) {
                return Err(());
            }
            let tag = canonical_host_tag(tag_n.as_str());
            let props = effective_props(&vnode_props(&nm));
            let children = vnode_children(&nm);
            let (pt, pr, pb, pl) = padding_insets(&props);
            let ix = x + pl;
            let iy = y_top + pt;
            let iw = (avail_w - pl - pr).max(0.0);

            match tag {
                "space" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    let wv = props_f64(&props, &["width", "w"], 8.0);
                    let hv = props_f64(&props, &["height", "h"], 8.0);
                    place(&*v, ix, iy, wv, hv);
                    freeze_autoresizing_for_manual_frames(&*v);
                    *slot += 1;
                    Ok(pt + hv + pb)
                }
                "rule" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    if !v.isKindOfClass(NSBox::class()) {
                        return Err(());
                    }
                    let bx: &NSBox = unsafe { &*(std::ptr::from_ref(&*v).cast()) };
                    let out = if props_string(&props, &["orientation"]) == Some("vertical".into()) {
                        let wv = props_f64(&props, &["width", "w"], 1.0);
                        let hv = props_f64(&props, &["height", "h"], 120.0);
                        place(bx, ix, iy, wv, hv);
                        freeze_autoresizing_for_manual_frames(bx);
                        pt + hv + pb
                    } else {
                        let hv = props_f64(&props, &["height", "h"], 1.0);
                        place(bx, ix, iy, iw, hv);
                        freeze_autoresizing_for_manual_frames(bx);
                        pt + hv + pb
                    };
                    *slot += 1;
                    Ok(out)
                }
                "row" => {
                    let och = vnode_children(&om);
                    let n = children.len().max(1);
                    let click_overlay = row_wants_click_overlay(&props);
                    let align = row_cross_align(&props);
                    let use_shell = has_container_layer_style(&props)
                        || click_overlay
                        || align != RowCrossAlign::Start;
                    let content_w = if use_shell {
                        (avail_w - pl - pr).max(0.0)
                    } else {
                        iw
                    };
                    let widths = row_child_widths(content_w, n, &props);
                    let mut max_h = 0.0_f64;
                    if use_shell {
                        let shell = subview(parent, *slot).ok_or(())?;
                        if has_container_layer_style(&props) {
                            apply_layer_style_to_view(&*shell, &props);
                        }
                        let mut inner = 0usize;
                        let mut x_off = 0.0_f64;
                        let mut heights: Vec<f64> = Vec::with_capacity(children.len());
                        for (i, (co, cn)) in och.iter().zip(children.iter()).enumerate() {
                            let cw = widths.get(i).copied().unwrap_or(0.0);
                            let h = patch_vnode(
                                co,
                                cn,
                                &*shell,
                                &mut inner,
                                pl + x_off,
                                pt,
                                cw,
                                None,
                                ctx,
                            )?;
                            heights.push(h);
                            max_h = max_h.max(h);
                            x_off += cw;
                        }
                        let expected = children.len() + if click_overlay { 1 } else { 0 };
                        if inner != children.len()
                            || shell.subviews().count() as usize != expected
                        {
                            return Err(());
                        }
                        if click_overlay {
                            let ov = shell.subviews().objectAtIndex(children.len());
                            if !ov.isKindOfClass(NSButton::class()) {
                                return Err(());
                            }
                            let btn: &NSButton =
                                unsafe { &*(std::ptr::from_ref(&*ov).cast::<NSButton>()) };
                            wire_on_click_patch(&props, btn, ctx, btn.tag());
                        }
                        let content_h = max_h.max(1.0);
                        let row_h = row_shell_outer_height(pt, pb, content_h, &props);
                        let inner_h = (row_h - pt - pb).max(0.0);
                        row_shell_reposition_children(
                            &*shell,
                            children.len(),
                            &heights,
                            pt,
                            inner_h,
                            align,
                        );
                        shell.setFrameSize(CGSize::new(avail_w, row_h));
                        place(&*shell, x, y_top, avail_w, row_h);
                        freeze_autoresizing_for_manual_frames(&*shell);
                        *slot += 1;
                        Ok(row_h)
                    } else {
                        let mut cx = ix;
                        for (i, (co, cn)) in och.iter().zip(children.iter()).enumerate() {
                            let cw = widths.get(i).copied().unwrap_or(0.0);
                            let h = patch_vnode(co, cn, parent, slot, cx, iy, cw, None, ctx)?;
                            max_h = max_h.max(h);
                            cx += cw;
                        }
                        Ok(pt + max_h + pb)
                    }
                }
                "column" | "section" => {
                    let och = vnode_children(&om);
                    if och.len() != children.len() {
                        return Err(());
                    }
                    let n = children.len();
                    if n == 0 {
                        return Ok(pt + pb);
                    }
                    if let Some(ah) = avail_h {
                        let content_h = (ah - pt - pb).max(0.0);
                        if n >= 2 {
                            let mut y = iy;
                            let mut used = 0.0_f64;
                            for (co, cn) in och.iter().zip(children.iter()).take(n - 1) {
                                let h = patch_vnode(co, cn, parent, slot, ix, y, iw, None, ctx)?;
                                y += h;
                                used += h;
                            }
                            let rem = (content_h - used).max(1.0);
                            let h_last = patch_vnode(
                                &och[n - 1],
                                &children[n - 1],
                                parent,
                                slot,
                                ix,
                                y,
                                iw,
                                Some(rem),
                                ctx,
                            )?;
                            Ok(pt + used + h_last + pb)
                        } else {
                            let h = patch_vnode(
                                &och[0],
                                &children[0],
                                parent,
                                slot,
                                ix,
                                iy,
                                iw,
                                Some(content_h),
                                ctx,
                            )?;
                            Ok(pt + h + pb)
                        }
                    } else {
                        let mut y = iy;
                        let mut total = 0.0_f64;
                        for (co, cn) in och.iter().zip(children.iter()) {
                            let h = patch_vnode(co, cn, parent, slot, ix, y, iw, None, ctx)?;
                            y += h;
                            total += h;
                        }
                        Ok(pt + total + pb)
                    }
                }
                "grouped_table" => Err(()),
                "zstack" => Err(()),
                "scrollable" => {
                    let scroll_view = subview(parent, *slot).ok_or(())?;
                    let scroll = unsafe { as_scroll(&*scroll_view).ok_or(())? };
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
                    scroll.setDrawsBackground(props_bool(
                        &props,
                        &["drawsBackground", "draws_background"],
                    ));
                    place(scroll, ix, iy, iw, sh);
                    let doc = scroll.documentView().ok_or(())?;
                    if !doc.isFlipped() {
                        return Err(());
                    }
                    let och = vnode_children(&om);
                    let mut inner = 0usize;
                    let mut doc_h = doc_top;
                    let mut dy = doc_top;
                    for (co, cn) in och.iter().zip(children.iter()) {
                        let h = patch_vnode(co, cn, &doc, &mut inner, 0.0, dy, iw, None, ctx)?;
                        dy += h;
                        doc_h += h;
                    }
                    if inner != doc.subviews().count() as usize {
                        return Err(());
                    }
                    doc.setFrameSize(CGSize::new(iw, doc_h.max(1.0)));
                    apply_layer_style_to_view(&*doc, &props);
                    sync_scroll_view_for_document(scroll, &*doc);
                    apply_scroll_scroller_top_inset(scroll, doc_top);
                    apply_scroll_content_right_gutter(
                        scroll,
                        scroll_scroller_right_gutter_from_props(&props),
                    );
                    *slot += 1;
                    Ok(pt + sh + pb)
                }
                "button" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    if !v.isKindOfClass(NSButton::class()) {
                        return Err(());
                    }
                    let btn: &NSButton = unsafe { &*(std::ptr::from_ref(&*v).cast()) };
                    let title = label_text_from_children(&children);
                    btn.setTitle(&NSString::from_str(&title));
                    apply_button_chrome(btn, &props);
                    wire_on_click_patch(&props, btn, ctx, btn.tag());
                    let h = props_f64(&props, &["height", "h"], 32.0);
                    apply_layer_style_to_view(btn, &props);
                    place(btn, ix, iy, iw, h);
                    freeze_autoresizing_for_manual_frames(btn);
                    *slot += 1;
                    Ok(pt + h + pb)
                }
                "text" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    if !v.isKindOfClass(NSTextField::class())
                        || v.isKindOfClass(NSSecureTextField::class())
                    {
                        return Err(());
                    }
                    let tf: &NSTextField = unsafe { &*(std::ptr::from_ref(&*v).cast()) };
                    let text = label_text_from_children(&children);
                    let wrap = props_bool(&props, &["wrap", "wrapping"]);
                    let cur = tf.stringValue().to_string();
                    if cur != text {
                        tf.setStringValue(&NSString::from_str(&text));
                    }
                    apply_static_label_text_field(tf, &props, ctx.mtm);
                    let h = if wrap {
                        props_f64(&props, &["height", "h"], 44.0)
                    } else {
                        single_line_label_height_after_style(tf, &props)
                    };
                    place(tf, ix, iy, iw, h);
                    freeze_autoresizing_for_manual_frames(tf);
                    *slot += 1;
                    Ok(pt + h + pb)
                }
                "textinput" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    if !v.isKindOfClass(NSTextField::class())
                        || v.isKindOfClass(NSSecureTextField::class())
                    {
                        return Err(());
                    }
                    let tf: &NSTextField = unsafe { &*(std::ptr::from_ref(&*v).cast()) };
                    let want = props_string(&props, &["value", "defaultValue"]).unwrap_or_default();
                    let cur = tf.stringValue().to_string();
                    if cur != want {
                        tf.setStringValue(&NSString::from_str(&want));
                    }
                    if let Some(Value::Function(f)) =
                        props.get("onChange").or_else(|| props.get("onInput"))
                    {
                        let f = f.clone();
                        let ttag = tf.tag();
                        let idx = if ttag >= 0 {
                            let (rid, slot) = decode_control_tag(ttag);
                            update_text_change_handler(
                                rid,
                                slot,
                                Rc::new(move |s: String| {
                                    let _ = f(&[Value::String(s.into())]);
                                }),
                            )
                        } else {
                            register_text_change_handler(ctx.root_id, Rc::new(move |s: String| {
                                let _ = f(&[Value::String(s.into())]);
                            }))
                        };
                        tf.setTag(idx);
                        unsafe {
                            tf.setDelegate(Some(ProtocolObject::from_ref(
                                &*ctx.text_delegate,
                            )));
                        }
                    }
                    let h = 24.0;
                    place(tf, ix, iy, iw, h);
                    freeze_autoresizing_for_manual_frames(tf);
                    *slot += 1;
                    Ok(pt + h + pb)
                }
                "password" | "secure" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    if !v.isKindOfClass(NSSecureTextField::class()) {
                        return Err(());
                    }
                    let tf: &NSSecureTextField = unsafe { &*(std::ptr::from_ref(&*v).cast()) };
                    let want = props_string(&props, &["value", "defaultValue"]).unwrap_or_default();
                    let cur = tf.stringValue().to_string();
                    if cur != want {
                        tf.setStringValue(&NSString::from_str(&want));
                    }
                    if let Some(Value::Function(f)) =
                        props.get("onChange").or_else(|| props.get("onInput"))
                    {
                        let f = f.clone();
                        let ttag = tf.tag();
                        let idx = if ttag >= 0 {
                            let (rid, slot) = decode_control_tag(ttag);
                            update_text_change_handler(
                                rid,
                                slot,
                                Rc::new(move |s: String| {
                                    let _ = f(&[Value::String(s.into())]);
                                }),
                            )
                        } else {
                            register_text_change_handler(ctx.root_id, Rc::new(move |s: String| {
                                let _ = f(&[Value::String(s.into())]);
                            }))
                        };
                        tf.setTag(idx);
                        unsafe {
                            tf.setDelegate(Some(ProtocolObject::from_ref(
                                &*ctx.text_delegate,
                            )));
                        }
                    }
                    let h = 24.0;
                    place(tf, ix, iy, iw, h);
                    freeze_autoresizing_for_manual_frames(tf);
                    *slot += 1;
                    Ok(pt + h + pb)
                }
                "checkbox" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    if !v.isKindOfClass(NSButton::class()) {
                        return Err(());
                    }
                    let btn: &NSButton = unsafe { &*(std::ptr::from_ref(&*v).cast()) };
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
                        let t = btn.tag();
                        let idx = if t >= 0 {
                            let (rid, slot) = decode_control_tag(t);
                            update_bool_handler(
                                rid,
                                slot,
                                Rc::new(move |b| {
                                    let _ = f(&[Value::Bool(b)]);
                                }),
                            )
                        } else {
                            register_bool_handler(ctx.root_id, Rc::new(move |b| {
                                let _ = f(&[Value::Bool(b)]);
                            }))
                        };
                        btn.setTag(idx);
                        unsafe {
                            let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
                            btn.setTarget(Some(&*p));
                            btn.setAction(Some(sel!(tishBool:)));
                        }
                    }
                    let h = 28.0;
                    place(btn, ix, iy, iw, h);
                    freeze_autoresizing_for_manual_frames(btn);
                    *slot += 1;
                    Ok(pt + h + pb)
                }
                "toggler" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    let sw = unsafe { as_switch(&*v).ok_or(())? };
                    sw.setState(if props_bool(&props, &["checked", "value"]) {
                        NSControlStateValueOn
                    } else {
                        NSControlStateValueOff
                    });
                    if let Some(Value::Function(f)) =
                        props.get("onChange").or_else(|| props.get("onToggle"))
                    {
                        let f = f.clone();
                        let t = sw.tag();
                        let idx = if t >= 0 {
                            let (rid, slot) = decode_control_tag(t);
                            update_bool_handler(
                                rid,
                                slot,
                                Rc::new(move |b| {
                                    let _ = f(&[Value::Bool(b)]);
                                }),
                            )
                        } else {
                            register_bool_handler(ctx.root_id, Rc::new(move |b| {
                                let _ = f(&[Value::Bool(b)]);
                            }))
                        };
                        sw.setTag(idx);
                        unsafe {
                            let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
                            sw.setTarget(Some(&*p));
                            sw.setAction(Some(sel!(tishBool:)));
                        }
                    }
                    let h = 28.0;
                    place(sw, ix, iy, 60.0, h);
                    freeze_autoresizing_for_manual_frames(sw);
                    *slot += 1;
                    Ok(pt + h + pb)
                }
                "slider" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    let sl = unsafe { as_slider(&*v).ok_or(())? };
                    let minv = props_f64(&props, &["min"], 0.0);
                    let maxv = props_f64(&props, &["max"], 100.0);
                    sl.setMinValue(minv);
                    sl.setMaxValue(maxv);
                    sl.setDoubleValue(props_f64(&props, &["value"], minv));
                    if let Some(Value::Function(f)) =
                        props.get("onChange").or_else(|| props.get("onInput"))
                    {
                        let f = f.clone();
                        let t = sl.tag();
                        let idx = if t >= 0 {
                            let (rid, slot) = decode_control_tag(t);
                            update_f64_handler(
                                rid,
                                slot,
                                Rc::new(move |v| {
                                    let _ = f(&[Value::Number(v)]);
                                }),
                            )
                        } else {
                            register_f64_handler(ctx.root_id, Rc::new(move |v| {
                                let _ = f(&[Value::Number(v)]);
                            }))
                        };
                        sl.setTag(idx);
                        unsafe {
                            let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
                            sl.setTarget(Some(&*p));
                            sl.setAction(Some(sel!(tishSlider:)));
                        }
                    }
                    let h = 24.0;
                    place(sl, ix, iy, iw, h);
                    freeze_autoresizing_for_manual_frames(sl);
                    *slot += 1;
                    Ok(pt + h + pb)
                }
                "progress_bar" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    let pi = unsafe { as_progress(&*v).ok_or(())? };
                    let ind = props_bool(&props, &["indeterminate"]);
                    if ind != pi.isIndeterminate() {
                        return Err(());
                    }
                    if ind {
                        place(pi, ix, iy, iw, 20.0);
                        freeze_autoresizing_for_manual_frames(pi);
                        *slot += 1;
                        return Ok(pt + 20.0 + pb);
                    }
                    pi.setMaxValue(props_f64(&props, &["max"], 1.0));
                    pi.setDoubleValue(props_f64(&props, &["value"], 0.0));
                    place(pi, ix, iy, iw, 20.0);
                    freeze_autoresizing_for_manual_frames(pi);
                    *slot += 1;
                    Ok(pt + 20.0 + pb)
                }
                "pick_list" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    let popup = unsafe { as_popup(&*v).ok_or(())? };
                    let opts = options_strings(&props);
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
                        let t = popup.tag();
                        let idx = if t >= 0 {
                            let (rid, slot) = decode_control_tag(t);
                            update_pick_handler(
                                rid,
                                slot,
                                Rc::new(move |i| {
                                    let _ = f(&[Value::Number(i as f64)]);
                                }),
                            )
                        } else {
                            register_pick_handler(ctx.root_id, Rc::new(move |i| {
                                let _ = f(&[Value::Number(i as f64)]);
                            }))
                        };
                        popup.setTag(idx);
                        unsafe {
                            let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
                            popup.setTarget(Some(&*p));
                            popup.setAction(Some(sel!(tishPick:)));
                        }
                    }
                    let row_h = 28.0;
                    place(popup, ix, iy, iw, row_h);
                    freeze_autoresizing_for_manual_frames(popup);
                    *slot += 1;
                    Ok(pt + row_h + pb)
                }
                "radio" => {
                    let opts = options_strings(&props);
                    let cur = props_f64(&props, &["value", "selected"], 0.0) as usize;
                    let mut y = iy;
                    let mut hsum = 0.0;
                    for (i, label) in opts.iter().enumerate() {
                        let rv = subview(parent, *slot).ok_or(())?;
                        if !rv.isKindOfClass(NSButton::class()) {
                            return Err(());
                        }
                        let btn: &NSButton = unsafe { &*(std::ptr::from_ref(&*rv).cast()) };
                        btn.setTitle(&NSString::from_str(label));
                        btn.setState(if i == cur {
                            NSControlStateValueOn
                        } else {
                            NSControlStateValueOff
                        });
                        if let Some(Value::Function(f)) = props.get("onChange") {
                            let f = f.clone();
                            let ii = i as f64;
                            let t = btn.tag();
                            let idx = if t >= 0 {
                                let (rid, slot) = decode_control_tag(t);
                                update_bool_handler(
                                    rid,
                                    slot,
                                    Rc::new(move |on| {
                                        if on {
                                            let _ = f(&[Value::Number(ii)]);
                                        }
                                    }),
                                )
                            } else {
                                register_bool_handler(ctx.root_id, Rc::new(move |on| {
                                    if on {
                                        let _ = f(&[Value::Number(ii)]);
                                    }
                                }))
                            };
                            btn.setTag(idx);
                            unsafe {
                                let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
                                btn.setTarget(Some(&*p));
                                btn.setAction(Some(sel!(tishBool:)));
                            }
                        }
                        let h = 24.0;
                        place(btn, ix, y, iw, h);
                        freeze_autoresizing_for_manual_frames(btn);
                        y += h;
                        hsum += h;
                        *slot += 1;
                    }
                    Ok(pt + hsum + pb)
                }
                "image" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    let iv = unsafe { as_image_view(&*v).ok_or(())? };
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
                    } else {
                        iv.setImage(None);
                    }
                    iv.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
                    if let Some(ref ts) =
                        props_string(&props, &["tint", "symbolTint", "symbol_tint"])
                    {
                        if let Some(col) = resolve_ns_color(ts) {
                            iv.setContentTintColor(Some(&col));
                        }
                    } else {
                        iv.setContentTintColor(None);
                    }
                    let ih = props_f64(&props, &["height", "h"], 120.0);
                    place(iv, ix, iy, iw, ih);
                    freeze_autoresizing_for_manual_frames(iv);
                    apply_layer_style_to_view(&*iv, &props);
                    *slot += 1;
                    Ok(pt + ih + pb)
                }
                "tooltip" => {
                    let wrap_v = subview(parent, *slot).ok_or(())?;
                    let tip =
                        props_string(&props, &["title", "tooltip", "label"]).unwrap_or_default();
                    if !tip.is_empty() {
                        wrap_v.setToolTip(Some(&NSString::from_str(&tip)));
                    } else {
                        wrap_v.setToolTip(None);
                    }
                    let och = vnode_children(&om);
                    let h_pass = if och.len() == 1 { avail_h } else { None };
                    let mut inner = 0usize;
                    let mut y = 0.0;
                    let mut hsum = 0.0;
                    for (co, cn) in och.iter().zip(children.iter()) {
                        let h = patch_vnode(co, cn, &*wrap_v, &mut inner, 0.0, y, iw, h_pass, ctx)?;
                        y += h;
                        hsum += h;
                    }
                    if inner != wrap_v.subviews().count() as usize {
                        return Err(());
                    }
                    apply_layer_style_to_view(&*wrap_v, &props);
                    place(&*wrap_v, ix, iy, iw, hsum);
                    freeze_autoresizing_for_manual_frames(&*wrap_v);
                    *slot += 1;
                    Ok(pt + hsum + pb)
                }
                "list" => {
                    let scroll_view = subview(parent, *slot).ok_or(())?;
                    let scroll = unsafe { as_scroll(&*scroll_view).ok_or(())? };
                    let th = scroll_outer_height(&props, avail_h);
                    scroll.setDrawsBackground(props_bool(
                        &props,
                        &["drawsBackground", "draws_background"],
                    ));
                    place(scroll, ix, iy, iw, th);
                    let doc = scroll.documentView().ok_or(())?;
                    if !doc.isKindOfClass(NSTextField::class()) {
                        return Err(());
                    }
                    let tf: &NSTextField = unsafe { &*(std::ptr::from_ref(&*doc).cast()) };
                    let rows: Vec<String> = match props.get("rows") {
                        Some(Value::Array(a)) => a
                            .borrow()
                            .iter()
                            .map(|v| v.to_display_string())
                            .collect(),
                        _ => vec![],
                    };
                    let body = rows.join("\n");
                    tf.setStringValue(&NSString::from_str(&body));
                    apply_static_label_text_field(tf, &props, ctx.mtm);
                    tf.setFrameSize(CGSize::new(iw.max(1.0), (th - 8.0).max(40.0)));
                    sync_scroll_view_for_document(scroll, &*doc);
                    apply_scroll_content_right_gutter(
                        scroll,
                        scroll_scroller_right_gutter_from_props(&props),
                    );
                    *slot += 1;
                    Ok(pt + th + pb)
                }
                "text_editor" => {
                    let scroll_view = subview(parent, *slot).ok_or(())?;
                    let scroll = unsafe { as_scroll(&*scroll_view).ok_or(())? };
                    let base_h = scroll_outer_height(&props, avail_h);
                    let min_h = props_f64(&props, &["minHeight", "min_height"], 120.0);
                    let th = base_h.max(min_h);
                    scroll.setDrawsBackground(props_bool(
                        &props,
                        &["drawsBackground", "draws_background"],
                    ));
                    place(scroll, ix, iy, iw, th);
                    let doc = scroll.documentView().ok_or(())?;
                    if !doc.isKindOfClass(NSTextView::class()) {
                        return Err(());
                    }
                    let tv: &NSTextView = unsafe { &*(std::ptr::from_ref(&*doc).cast()) };
                    let want = props_string(&props, &["value", "defaultValue"]).unwrap_or_default();
                    let cur = tv.string().to_string();
                    if cur != want {
                        if props
                            .get("onChange")
                            .or_else(|| props.get("onInput"))
                            .is_some()
                        {
                            text_view_set_string_without_delegate_notice(tv, &want, ctx);
                        } else {
                            tv.setString(&NSString::from_str(&want));
                        }
                    }
                    if let Some(Value::Function(f)) =
                        props.get("onChange").or_else(|| props.get("onInput"))
                    {
                        let f = f.clone();
                        let idx = if let Some(encoded) = text_change_tag_from_text_view(tv) {
                            let (rid, slot_i) = decode_control_tag(encoded);
                            update_text_change_handler(
                                rid,
                                slot_i,
                                Rc::new(move |s: String| {
                                    let _ = f(&[Value::String(s.into())]);
                                }),
                            )
                        } else {
                            register_text_change_handler(ctx.root_id, Rc::new(move |s: String| {
                                let _ = f(&[Value::String(s.into())]);
                            }))
                        };
                        install_text_change_tag_on_text_view(tv, idx);
                        tv.setDelegate(Some(ProtocolObject::from_ref(
                            &*ctx.text_view_delegate,
                        )));
                    }
                    let fs = props_f64(&props, &["fontSize", "font_size"], 13.0);
                    let font = NSFont::systemFontOfSize(fs as CGFloat);
                    tv.setFont(Some(&font));
                    apply_nstext_view_document_background_from_props(tv, &props);
                    let fg = NSColor::labelColor();
                    tv.setTextColor(Some(&fg));
                    tv.setInsertionPointColor(Some(&fg));
                    tv.setFrameSize(CGSize::new(iw.max(1.0), (th - 8.0).max(40.0)));
                    sync_scroll_view_for_document(scroll, &*doc);
                    apply_scroll_content_right_gutter(
                        scroll,
                        scroll_scroller_right_gutter_from_props(&props),
                    );
                    *slot += 1;
                    Ok(pt + th + pb)
                }
                "markdown_text" => {
                    let scroll_view = subview(parent, *slot).ok_or(())?;
                    let scroll = unsafe { as_scroll(&*scroll_view).ok_or(())? };
                    let base_h = scroll_outer_height(&props, avail_h);
                    let min_h = props_f64(&props, &["minHeight", "min_height"], 120.0);
                    let th = base_h.max(min_h);
                    scroll.setDrawsBackground(props_bool(
                        &props,
                        &["drawsBackground", "draws_background"],
                    ));
                    place(scroll, ix, iy, iw, th);
                    let doc = scroll.documentView().ok_or(())?;
                    if !doc.isKindOfClass(NSTextView::class()) {
                        return Err(());
                    }
                    let tv: &NSTextView = unsafe { &*(std::ptr::from_ref(&*doc).cast()) };
                    let raw_old = vnode_props(&om);
                    let old_props = effective_props(&raw_old);
                    let old_md = props_string(&old_props, &["markdown", "value", "defaultValue"])
                        .unwrap_or_default();
                    let want = props_string(&props, &["markdown", "value", "defaultValue"])
                        .unwrap_or_default();
                    if old_md != want {
                        set_text_view_markdown(tv, &want, ctx.mtm);
                    }
                    let fs = props_f64(&props, &["fontSize", "font_size"], 13.0);
                    apply_markdown_text_view_chrome(tv, fs as CGFloat);
                    apply_nstext_view_document_background_from_props(tv, &props);
                    tv.setFrameSize(CGSize::new(iw.max(1.0), (th - 8.0).max(40.0)));
                    sync_scroll_view_for_document(scroll, &*doc);
                    apply_scroll_content_right_gutter(
                        scroll,
                        scroll_scroller_right_gutter_from_props(&props),
                    );
                    *slot += 1;
                    Ok(pt + th + pb)
                }
                "tabs" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    let tabv = unsafe { as_tabview(&*v).ok_or(())? };
                    let th = props_f64(&props, &["height", "h"], 200.0);
                    let n = tabv.numberOfTabViewItems() as usize;
                    if n != children.len() {
                        return Err(());
                    }
                    let old_tabs = vnode_children(&om);
                    for ti in 0..n {
                        let old_tab = old_tabs.get(ti).ok_or(())?;
                        let new_tab = children.get(ti).ok_or(())?;
                        let (lbl, body) = tab_label_and_children(new_tab);
                        let (_old_lbl, old_body) = tab_label_and_children(old_tab);
                        if old_body.len() != body.len()
                            || !old_body
                                .iter()
                                .zip(body.iter())
                                .all(|(a, b)| vnode_same_shape(a, b))
                        {
                            return Err(());
                        }
                        let item = tabv.tabViewItemAtIndex(ti as isize);
                        item.setLabel(&NSString::from_str(&lbl));
                        let pane = item.view(ctx.mtm).ok_or(())?;
                        let mut inner = 0usize;
                        let mut y = 0.0;
                        for (co, cn) in old_body.iter().zip(body.iter()) {
                            let h = patch_vnode(co, cn, &pane, &mut inner, 0.0, y, iw, None, ctx)?;
                            y += h;
                        }
                        if inner != pane.subviews().count() as usize {
                            return Err(());
                        }
                    }
                    place(tabv, ix, iy, iw, th);
                    *slot += 1;
                    Ok(pt + th + pb)
                }
                "split" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    let split = unsafe { as_split(&*v).ok_or(())? };
                    let th = scroll_outer_height(&props, avail_h);
                    split.setAutoresizingMask(
                        NSAutoresizingMaskOptions::ViewWidthSizable
                            | NSAutoresizingMaskOptions::ViewHeightSizable,
                    );
                    split.setVertical(split_uses_vertical_divider(&props));
                    split.setDividerStyle(split_divider_style(&props));
                    let (w0, h0, w1, h1, pos) = split_pane_layout(&props, iw, th);
                    let och = vnode_children(&om);
                    let o_panes = split_pane_vnodes(&och);
                    let n_panes = split_pane_vnodes(&children);
                    if o_panes.len() < 2 || n_panes.len() < 2 {
                        return Err(());
                    }
                    let subs = split.subviews();
                    if subs.count() < 2 {
                        return Err(());
                    }
                    for i in 0..2 {
                        let pane = subs.objectAtIndex(i);
                        pane.setAutoresizingMask(
                            NSAutoresizingMaskOptions::ViewWidthSizable
                                | NSAutoresizingMaskOptions::ViewHeightSizable,
                        );
                        let mut inner = 0usize;
                        let (pw, ph) = if i == 0 { (w0, h0) } else { (w1, h1) };
                        patch_vnode(
                            &o_panes[i],
                            &n_panes[i],
                            &pane,
                            &mut inner,
                            0.0,
                            0.0,
                            pw,
                            Some(ph),
                            ctx,
                        )?;
                        if inner != pane.subviews().count() as usize {
                            return Err(());
                        }
                    }
                    place(split, ix, iy, iw, th);
                    split.setPosition_ofDividerAtIndex(pos, 0);
                    split.adjustSubviews();
                    snap_flipped_split_panes_full_height(split);
                    *slot += 1;
                    Ok(pt + th + pb)
                }
                "visual_effect" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    let fx = unsafe { as_visual_effect(&*v).ok_or(())? };
                    apply_visual_effect_view_from_props(fx, &props);
                    let gutter = props_f64(
                        &props,
                        &["scrollerGutterRight", "scroller_gutter_right"],
                        0.0,
                    )
                    .max(0.0);
                    if fx.isKindOfClass(FlippedVisualEffectView::class()) {
                        let fv: &FlippedVisualEffectView =
                            unsafe { &*(std::ptr::from_ref(fx).cast::<FlippedVisualEffectView>()) };
                        fv.set_right_gutter(gutter);
                    }
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
                    let vh = scroll_outer_height(&props, avail_h);
                    let intrinsic_h = visual_effect_intrinsic_outer_height(avail_h, &props);
                    let child_avail = if intrinsic_h { None } else { Some(vh) };
                    let inner_w = (iw - gutter).max(1.0);
                    let subs = fx.subviews();
                    if subs.count() < 1 {
                        return Err(());
                    }
                    let doc = subs.objectAtIndex(0);
                    if !doc.isFlipped() {
                        return Err(());
                    }
                    let och = vnode_children(&om);
                    let mut inner = 0usize;
                    let mut doc_h = 0.0;
                    let mut dy = 0.0;
                    for (co, cn) in och.iter().zip(children.iter()) {
                        let h = patch_vnode(
                            co,
                            cn,
                            &doc,
                            &mut inner,
                            0.0,
                            dy,
                            inner_w,
                            child_avail,
                            ctx,
                        )?;
                        dy += h;
                        doc_h += h;
                    }
                    if inner != doc.subviews().count() as usize {
                        return Err(());
                    }
                    let content_h = doc_h.max(1.0);
                    let fh = if intrinsic_h {
                        content_h
                    } else {
                        content_h.max(vh)
                    };
                    place_visual_effect_document(&*fx, &*doc, inner_w, content_h, fh);
                    place(fx, ix, iy, inner_w, fh);
                    fx.layoutSubtreeIfNeeded();
                    *slot += 1;
                    Ok(pt + fh + pb)
                }
                "webview" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    let wv = unsafe { as_webview(&*v).ok_or(())? };
                    let th = scroll_outer_height(&props, avail_h);
                    let src = props_string(&props, &["src", "url"]).unwrap_or_default();
                    if let Some(url) = NSURL::URLWithString(&NSString::from_str(&src)) {
                        let req = NSURLRequest::requestWithURL(&url);
                        unsafe {
                            let _ = wv.loadRequest(&req);
                        }
                    }
                    place(wv, ix, iy, iw, th);
                    *slot += 1;
                    Ok(pt + th + pb)
                }
                _ => {
                    let boxv = subview(parent, *slot).ok_or(())?;
                    let och = vnode_children(&om);
                    let h_pass = if och.len() == 1 { avail_h } else { None };
                    let mut inner = 0usize;
                    let mut y = 0.0;
                    let mut hsum = 0.0;
                    for (co, cn) in och.iter().zip(children.iter()) {
                        let h = patch_vnode(co, cn, &*boxv, &mut inner, 0.0, y, iw, h_pass, ctx)?;
                        y += h;
                        hsum += h;
                    }
                    if inner != boxv.subviews().count() as usize {
                        return Err(());
                    }
                    apply_layer_style_to_view(&*boxv, &props);
                    place(&*boxv, ix, iy, iw, hsum);
                    freeze_autoresizing_for_manual_frames(&*boxv);
                    *slot += 1;
                    Ok(pt + hsum + pb)
                }
            }
        }
        _ => Err(()),
    }
}
