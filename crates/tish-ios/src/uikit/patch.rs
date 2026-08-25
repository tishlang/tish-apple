//! In-place vnode patch for UIKit — preserves `UITextField` first-responder across re-renders.

use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{sel, ClassType};
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSObjectProtocol, NSString};
use objc2::runtime::ProtocolObject;
use objc2_ui_kit::{
    UIButton, UIControlEvents, UIControlState, UILabel, UIScrollView, UISegmentedControl,
    UISlider, UISwitch, UITextField, UITextView, UIView,
};
use tishlang_apple_common::handlers::{
    decode_control_tag, register_bool_handler, register_click_handler, register_f64_handler,
    register_text_change_handler, update_bool_handler, update_click_handler, update_f64_handler,
    update_text_change_handler,
};
use tishlang_apple_common::style::{props_bool, props_f64, props_string};
use tishlang_apple_common::tag::canonical_host_tag;
use tishlang_core::{PropMap, Value};
use tishlang_ui::runtime::is_fragment_tag;

use super::build::{
    collect_element_vnodes, freeze_autoresizing, measure_label_height, padding_insets, place,
    root_content_insets, scroll_outer_height, style_label, text_from_children, vnode_children,
    vnode_props, BuildCtx,
};
use super::wk_webview::WKWebView;

pub fn vnode_same_shape(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(_), Value::String(_)) => true,
        (Value::Object(oa), Value::Object(ob)) => {
            let ma = &oa.borrow().strings;
            let mb = &ob.borrow().strings;
            let ta = ma.get("tag").unwrap_or(&Value::Null);
            if is_fragment_tag(ta) {
                let ca = vnode_children(ma);
                let cb = vnode_children(mb);
                return ca.len() == cb.len()
                    && ca
                        .iter()
                        .zip(cb.iter())
                        .all(|(x, y)| vnode_same_shape(x, y));
            }
            let sa = ma
                .get("tag")
                .and_then(|t| match t {
                    Value::String(s) => Some(s.as_str().to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            let sb = mb
                .get("tag")
                .and_then(|t| match t {
                    Value::String(s) => Some(s.as_str().to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            if canonical_host_tag(sa.as_str()) != canonical_host_tag(sb.as_str()) {
                return false;
            }
            let ca = vnode_children(ma);
            let cb = vnode_children(mb);
            ca.len() == cb.len()
                && ca
                    .iter()
                    .zip(cb.iter())
                    .all(|(x, y)| vnode_same_shape(x, y))
        }
        _ => false,
    }
}

fn subview(parent: &UIView, i: usize) -> Option<Retained<UIView>> {
    let subs = parent.subviews();
    let n = subs.count() as usize;
    if i >= n {
        return None;
    }
    Some(subs.objectAtIndex(i))
}

fn has_control_tag(tag: isize) -> bool {
    // `UIView.tag` defaults to 0; registered handlers always encode a non-zero tag
    // (or at least a valid slot once a control has been wired once).
    tag != 0
}

fn wire_on_click_patch(props: &PropMap, btn: &UIButton, ctx: &BuildCtx) {
    if let Some(Value::Function(f)) = props.get("onClick").or_else(|| props.get("onclick")) {
        let f = f.clone();
        let existing = btn.tag();
        let idx = if has_control_tag(existing) {
            let (rid, slot) = decode_control_tag(existing);
            update_click_handler(
                rid,
                slot,
                Rc::new(move || {
                    let _ = f.call(&[]);
                }),
            )
        } else {
            let idx = register_click_handler(
                ctx.root_id,
                Rc::new(move || {
                    let _ = f.call(&[]);
                }),
            );
            unsafe {
                let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
                btn.addTarget_action_forControlEvents(
                    Some(&*p),
                    sel!(jsxClick:),
                    objc2_ui_kit::UIControlEvents::TouchUpInside,
                );
            }
            idx
        };
        btn.setTag(idx);
    }
}

fn wire_text_change_patch(props: &PropMap, tf: &UITextField, ctx: &BuildCtx) {
    if let Some(Value::Function(f)) = props.get("onChange").or_else(|| props.get("onInput")) {
        let f = f.clone();
        let existing = tf.tag();
        let idx = if has_control_tag(existing) {
            let (rid, slot) = decode_control_tag(existing);
            update_text_change_handler(
                rid,
                slot,
                Rc::new(move |s: String| {
                    let _ = f.call(&[Value::String(s.into())]);
                }),
            )
        } else {
            let idx = register_text_change_handler(
                ctx.root_id,
                Rc::new(move |s: String| {
                    let _ = f.call(&[Value::String(s.into())]);
                }),
            );
            unsafe {
                let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
                tf.addTarget_action_forControlEvents(
                    Some(&*p),
                    sel!(jsxTextChanged:),
                    objc2_ui_kit::UIControlEvents::EditingChanged,
                );
            }
            idx
        };
        tf.setTag(idx);
    }
}

fn wire_bool_change_patch(props: &PropMap, sw: &UISwitch, ctx: &BuildCtx) {
    if let Some(Value::Function(f)) = props.get("onChange").or_else(|| props.get("onToggle")) {
        let f = f.clone();
        let existing = sw.tag();
        let idx = if has_control_tag(existing) {
            let (rid, slot) = decode_control_tag(existing);
            update_bool_handler(
                rid,
                slot,
                Rc::new(move |b| {
                    let _ = f.call(&[Value::Bool(b)]);
                }),
            )
        } else {
            let idx = register_bool_handler(
                ctx.root_id,
                Rc::new(move |b| {
                    let _ = f.call(&[Value::Bool(b)]);
                }),
            );
            unsafe {
                let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
                sw.addTarget_action_forControlEvents(
                    Some(&*p),
                    sel!(jsxBoolChanged:),
                    UIControlEvents::ValueChanged,
                );
            }
            idx
        };
        sw.setTag(idx);
    }
}

fn wire_slider_change_patch(props: &PropMap, sl: &UISlider, ctx: &BuildCtx) {
    if let Some(Value::Function(f)) = props.get("onChange").or_else(|| props.get("onInput")) {
        let f = f.clone();
        let existing = sl.tag();
        let idx = if has_control_tag(existing) {
            let (rid, slot) = decode_control_tag(existing);
            update_f64_handler(
                rid,
                slot,
                Rc::new(move |v| {
                    let _ = f.call(&[Value::Number(v)]);
                }),
            )
        } else {
            let idx = register_f64_handler(
                ctx.root_id,
                Rc::new(move |v| {
                    let _ = f.call(&[Value::Number(v)]);
                }),
            );
            unsafe {
                let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
                sl.addTarget_action_forControlEvents(
                    Some(&*p),
                    sel!(jsxSliderChanged:),
                    UIControlEvents::ValueChanged,
                );
            }
            idx
        };
        sl.setTag(idx);
    }
}

fn wire_segment_change_patch(props: &PropMap, seg: &UISegmentedControl, ctx: &BuildCtx) {
    if let Some(Value::Function(f)) = props.get("onChange").or_else(|| props.get("onSelect")) {
        let f = f.clone();
        let existing = seg.tag();
        let idx = if has_control_tag(existing) {
            let (rid, slot) = decode_control_tag(existing);
            update_f64_handler(
                rid,
                slot,
                Rc::new(move |v| {
                    let _ = f.call(&[Value::Number(v)]);
                }),
            )
        } else {
            let idx = register_f64_handler(
                ctx.root_id,
                Rc::new(move |v| {
                    let _ = f.call(&[Value::Number(v)]);
                }),
            );
            unsafe {
                let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
                seg.addTarget_action_forControlEvents(
                    Some(&*p),
                    sel!(jsxSegmentChanged:),
                    UIControlEvents::ValueChanged,
                );
            }
            idx
        };
        seg.setTag(idx);
    }
}

fn wire_text_view_change_patch(props: &PropMap, tv: &UITextView, ctx: &BuildCtx) {
    if let Some(Value::Function(f)) = props.get("onChange").or_else(|| props.get("onInput")) {
        let f = f.clone();
        let existing = tv.tag();
        let idx = if has_control_tag(existing) {
            let (rid, slot) = decode_control_tag(existing);
            update_text_change_handler(
                rid,
                slot,
                Rc::new(move |s: String| {
                    let _ = f.call(&[Value::String(s.into())]);
                }),
            )
        } else {
            let idx = register_text_change_handler(
                ctx.root_id,
                Rc::new(move |s: String| {
                    let _ = f.call(&[Value::String(s.into())]);
                }),
            );
            unsafe {
                tv.setDelegate(Some(ProtocolObject::from_ref(&*ctx.text_view_delegate)));
            }
            idx
        };
        tv.setTag(idx);
    }
}

fn patch_text_field(tf: &UITextField, props: &PropMap, ctx: &BuildCtx) {
    let want = props_string(props, &["value", "defaultValue"]).unwrap_or_default();
    let cur = tf
        .text()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let focused = tf.isFirstResponder();
    if cur != want && !focused {
        tf.setText(Some(&NSString::from_str(&want)));
    }
    if let Some(ph) = props_string(props, &["placeholder", "hint"]) {
        let cur_ph = tf
            .placeholder()
            .map(|s| s.to_string())
            .unwrap_or_default();
        if cur_ph != ph {
            tf.setPlaceholder(Some(&NSString::from_str(&ph)));
        }
    }
    wire_text_change_patch(props, tf, ctx);
}

/// Patch when shapes match; returns laid-out height, or `None` to force full rebuild.
pub fn try_patch_vtree(
    old: &Value,
    new: &Value,
    root: &UIView,
    width: f64,
    height: f64,
    ctx: &BuildCtx,
) -> Option<f64> {
    if !vnode_same_shape(old, new) {
        return None;
    }
    let (left, top, right, bottom) = root_content_insets(root);
    let inner_w = (width - left - right).max(0.0);
    let mut slot = 0usize;
    let h = patch_vnode(
        old,
        new,
        root,
        &mut slot,
        left,
        top,
        inner_w,
        Some((height - top - bottom).max(0.0)),
        ctx,
    )
    .ok()?;
    let n = root.subviews().count() as usize;
    if slot != n {
        return None;
    }
    Some(h)
}

fn patch_vnode(
    old: &Value,
    new: &Value,
    parent: &UIView,
    slot: &mut usize,
    x: f64,
    y: f64,
    avail_w: f64,
    avail_h: Option<f64>,
    ctx: &BuildCtx,
) -> Result<f64, ()> {
    match (old, new) {
        (Value::String(_sa), Value::String(sb)) => {
            let v = subview(parent, *slot).ok_or(())?;
            if !v.isKindOfClass(UILabel::class()) {
                return Err(());
            }
            let label: &UILabel = unsafe { &*(std::ptr::from_ref(&*v).cast()) };
            let t = sb.as_str().trim();
            let cur = label
                .text()
                .map(|s| s.to_string())
                .unwrap_or_default();
            if cur != t {
                label.setText(Some(&NSString::from_str(t)));
            }
            style_label(label, None);
            let h = measure_label_height(label, avail_w, None);
            place(&*v, x, y, avail_w, h);
            freeze_autoresizing(&*v);
            *slot += 1;
            Ok(h)
        }
        (Value::Object(oa), Value::Object(ob)) => {
            let om = &oa.borrow().strings;
            if is_fragment_tag(om.get("tag").unwrap_or(&Value::Null)) {
                let nm = &ob.borrow().strings;
                let ch_old = vnode_children(om);
                let ch_new = vnode_children(nm);
                if ch_old.len() != ch_new.len() {
                    return Err(());
                }
                let mut cy = y;
                let mut hsum = 0.0;
                for (co, cn) in ch_old.iter().zip(ch_new.iter()) {
                    let h = patch_vnode(co, cn, parent, slot, x, cy, avail_w, avail_h, ctx)?;
                    cy += h;
                    hsum += h;
                }
                return Ok(hsum);
            }
            let tag_o = om
                .get("tag")
                .and_then(|t| match t {
                    Value::String(s) => Some(s.as_str().to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            let nm = &ob.borrow().strings;
            let tag_n = nm
                .get("tag")
                .and_then(|t| match t {
                    Value::String(s) => Some(s.as_str().to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            if canonical_host_tag(tag_o.as_str()) != canonical_host_tag(tag_n.as_str()) {
                return Err(());
            }
            let raw_tag = tag_n.as_str();
            let tag = canonical_host_tag(raw_tag);
            let props = vnode_props(nm);
            let children = vnode_children(nm);
            let (pt, pr, pb, pl) = padding_insets(&props);
            let ix = x + pl;
            let iy = y + pt;
            let iw = (avail_w - pl - pr).max(0.0);

            match tag {
                "column" | "section" | "div" => {
                    let mut old_elems = Vec::new();
                    let mut new_elems = Vec::new();
                    collect_element_vnodes(&vnode_children(om), &mut old_elems);
                    collect_element_vnodes(&children, &mut new_elems);
                    if old_elems.len() != new_elems.len() {
                        return Err(());
                    }
                    let gap = props_f64(&props, &["gap", "rowGap", "row_gap"], 0.0);
                    let mut cy = iy;
                    let mut total = 0.0_f64;
                    for (co, cn) in old_elems.iter().zip(new_elems.iter()) {
                        let h = patch_vnode(co, cn, parent, slot, ix, cy, iw, None, ctx)?;
                        cy += h + gap;
                        total += h + gap;
                    }
                    if !old_elems.is_empty() && gap > 0.0 {
                        total -= gap;
                    }
                    Ok(pt + total + pb)
                }
                "row" => {
                    let mut old_elems = Vec::new();
                    let mut new_elems = Vec::new();
                    collect_element_vnodes(&vnode_children(om), &mut old_elems);
                    collect_element_vnodes(&children, &mut new_elems);
                    if old_elems.len() != new_elems.len() {
                        return Err(());
                    }
                    let n = old_elems.len().max(1);
                    let gap = props_f64(&props, &["gap", "columnGap", "column_gap"], 0.0);
                    let total_gap = gap * (n.saturating_sub(1) as f64);
                    let cw = ((iw - total_gap) / n as f64).max(0.0);
                    let mut cx = ix;
                    let mut max_h = 0.0_f64;
                    for (i, (co, cn)) in old_elems.iter().zip(new_elems.iter()).enumerate() {
                        let h = patch_vnode(co, cn, parent, slot, cx, iy, cw, None, ctx)?;
                        max_h = max_h.max(h);
                        cx += cw;
                        if i + 1 < old_elems.len() {
                            cx += gap;
                        }
                    }
                    Ok(pt + max_h.max(44.0) + pb)
                }
                "scrollable" => {
                    let scroll_view = subview(parent, *slot).ok_or(())?;
                    if !scroll_view.isKindOfClass(UIScrollView::class()) {
                        return Err(());
                    }
                    let scroll: &UIScrollView =
                        unsafe { &*(std::ptr::from_ref(&*scroll_view).cast()) };
                    let outer_h = scroll_outer_height(&props, avail_h);
                    place(scroll, ix, iy, iw, outer_h);
                    let content = subview(scroll, 0).ok_or(())?;
                    let mut old_elems = Vec::new();
                    let mut new_elems = Vec::new();
                    collect_element_vnodes(&vnode_children(om), &mut old_elems);
                    collect_element_vnodes(&children, &mut new_elems);
                    if old_elems.len() != new_elems.len() {
                        return Err(());
                    }
                    let mut inner = 0usize;
                    let mut doc_h = 0.0_f64;
                    let mut dy = 0.0_f64;
                    for (co, cn) in old_elems.iter().zip(new_elems.iter()) {
                        let h = patch_vnode(co, cn, &content, &mut inner, 0.0, dy, iw, None, ctx)?;
                        dy += h;
                        doc_h += h;
                    }
                    if inner != content.subviews().count() as usize {
                        return Err(());
                    }
                    doc_h = doc_h.max(1.0);
                    place(&content, 0.0, 0.0, iw, doc_h);
                    freeze_autoresizing(&content);
                    scroll.setContentSize(CGSize::new(iw, doc_h));
                    *slot += 1;
                    Ok(pt + outer_h + pb)
                }
                "button" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    if !v.isKindOfClass(UIButton::class()) {
                        return Err(());
                    }
                    let btn: &UIButton = unsafe { &*(std::ptr::from_ref(&*v).cast()) };
                    let title = text_from_children(&children);
                    btn.setTitle_forState(Some(&NSString::from_str(&title)), UIControlState::Normal);
                    wire_on_click_patch(&props, btn, ctx);
                    let h = props_f64(&props, &["height", "h"], 44.0);
                    place(btn, ix, iy, iw, h);
                    freeze_autoresizing(btn);
                    *slot += 1;
                    Ok(pt + h + pb)
                }
                "textinput" | "password" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    if !v.isKindOfClass(UITextField::class()) {
                        return Err(());
                    }
                    let tf: &UITextField = unsafe { &*(std::ptr::from_ref(&*v).cast()) };
                    patch_text_field(tf, &props, ctx);
                    let h = props_f64(&props, &["height", "h"], 44.0);
                    place(tf, ix, iy, iw, h);
                    freeze_autoresizing(tf);
                    *slot += 1;
                    Ok(pt + h + pb)
                }
                "toggler" | "checkbox" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    if !v.isKindOfClass(UISwitch::class()) {
                        return Err(());
                    }
                    let sw: &UISwitch = unsafe { &*(std::ptr::from_ref(&*v).cast()) };
                    let on = props_bool(&props, &["checked", "value"], false);
                    if sw.isOn() != on {
                        sw.setOn_animated(on, false);
                    }
                    wire_bool_change_patch(&props, sw, ctx);
                    let h = props_f64(&props, &["height", "h"], 44.0);
                    let w = 60.0_f64.min(iw);
                    place(sw, ix, iy + ((h - 31.0) * 0.5).max(0.0), w, 31.0);
                    freeze_autoresizing(sw);
                    *slot += 1;
                    Ok(pt + h + pb)
                }
                "slider" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    if !v.isKindOfClass(UISlider::class()) {
                        return Err(());
                    }
                    let sl: &UISlider = unsafe { &*(std::ptr::from_ref(&*v).cast()) };
                    let minv = props_f64(&props, &["min"], 0.0) as f32;
                    let maxv = props_f64(&props, &["max"], 100.0) as f32;
                    sl.setMinimumValue(minv);
                    sl.setMaximumValue(maxv);
                    let want = props_f64(&props, &["value"], minv as f64) as f32;
                    if (sl.value() - want).abs() > 0.000_1 {
                        sl.setValue(want);
                    }
                    wire_slider_change_patch(&props, sl, ctx);
                    let h = props_f64(&props, &["height", "h"], 44.0);
                    place(sl, ix, iy, iw, h);
                    freeze_autoresizing(sl);
                    *slot += 1;
                    Ok(pt + h + pb)
                }
                "text_editor" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    if !v.isKindOfClass(UITextView::class()) {
                        return Err(());
                    }
                    let tv: &UITextView = unsafe { &*(std::ptr::from_ref(&*v).cast()) };
                    let want = props_string(&props, &["value", "defaultValue"]).unwrap_or_default();
                    let cur = tv.text().to_string();
                    let focused = tv.isFirstResponder();
                    if cur != want && !focused {
                        tv.setText(Some(&NSString::from_str(&want)));
                    }
                    wire_text_view_change_patch(&props, tv, ctx);
                    let base_h = scroll_outer_height(&props, avail_h);
                    let min_h = props_f64(&props, &["minHeight", "min_height"], 120.0);
                    let th = base_h.max(min_h);
                    place(tv, ix, iy, iw, th);
                    freeze_autoresizing(tv);
                    *slot += 1;
                    Ok(pt + th + pb)
                }
                "tabs" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    let th = props_f64(&props, &["height", "h"], avail_h.unwrap_or(200.0));
                    place(&*v, ix, iy, iw, th);
                    freeze_autoresizing(&*v);
                    let outer_subs = v.subviews();
                    if outer_subs.count() < 2 {
                        return Err(());
                    }
                    let seg_v = outer_subs.objectAtIndex(0);
                    if !seg_v.isKindOfClass(UISegmentedControl::class()) {
                        return Err(());
                    }
                    let seg: &UISegmentedControl =
                        unsafe { &*(std::ptr::from_ref(&*seg_v).cast()) };
                    let selected = props_f64(&props, &["selected", "value"], 0.0) as isize;
                    let content = outer_subs.objectAtIndex(1);
                    let panes = content.subviews();
                    let n = panes.count() as isize;
                    let selected = selected.max(0).min((n - 1).max(0));
                    if n > 0 && seg.selectedSegmentIndex() != selected {
                        seg.setSelectedSegmentIndex(selected);
                    }
                    for i in 0..panes.count() {
                        let pane = panes.objectAtIndex(i);
                        pane.setHidden((i as isize) != selected);
                    }
                    wire_segment_change_patch(&props, seg, ctx);
                    // Patch children inside each pane when tab bodies match 1:1.
                    let mut old_tabs = Vec::new();
                    let mut new_tabs = Vec::new();
                    collect_element_vnodes(&vnode_children(om), &mut old_tabs);
                    collect_element_vnodes(&children, &mut new_tabs);
                    if old_tabs.len() != new_tabs.len() || old_tabs.len() as isize != n {
                        return Err(());
                    }
                    let seg_h = 32.0_f64;
                    let gap = 8.0_f64;
                    let pane_h = (th - seg_h - gap).max(40.0);
                    for i in 0..old_tabs.len() {
                        let pane = panes.objectAtIndex(i);
                        let mut pane_slot = 0usize;
                        let mut old_body = Vec::new();
                        let mut new_body = Vec::new();
                        // Prefer <tab> children; otherwise treat child as body.
                        let om_tab = match &old_tabs[i] {
                            Value::Object(o) => o.borrow().strings.clone(),
                            _ => return Err(()),
                        };
                        let nm_tab = match &new_tabs[i] {
                            Value::Object(o) => o.borrow().strings.clone(),
                            _ => return Err(()),
                        };
                        let old_is_tab = matches!(
                            om_tab.get("tag"),
                            Some(Value::String(s)) if s.as_str() == "tab" || s.as_str() == "Tab"
                        );
                        let new_is_tab = matches!(
                            nm_tab.get("tag"),
                            Some(Value::String(s)) if s.as_str() == "tab" || s.as_str() == "Tab"
                        );
                        if old_is_tab {
                            collect_element_vnodes(&vnode_children(&om_tab), &mut old_body);
                        } else {
                            old_body.push(old_tabs[i].clone());
                        }
                        if new_is_tab {
                            collect_element_vnodes(&vnode_children(&nm_tab), &mut new_body);
                        } else {
                            new_body.push(new_tabs[i].clone());
                        }
                        if old_body.len() != new_body.len() {
                            return Err(());
                        }
                        let mut y = 0.0_f64;
                        for (co, cn) in old_body.iter().zip(new_body.iter()) {
                            let h = patch_vnode(
                                co,
                                cn,
                                &pane,
                                &mut pane_slot,
                                0.0,
                                y,
                                iw,
                                Some(pane_h),
                                ctx,
                            )?;
                            y += h;
                        }
                        if pane_slot != pane.subviews().count() as usize {
                            return Err(());
                        }
                    }
                    *slot += 1;
                    Ok(pt + th + pb)
                }
                "image" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    if !v.isKindOfClass(objc2_ui_kit::UIImageView::class()) {
                        return Err(());
                    }
                    let h = props_f64(&props, &["height", "h"], 48.0);
                    place(&*v, ix, iy, iw, h);
                    freeze_autoresizing(&*v);
                    *slot += 1;
                    Ok(pt + h + pb)
                }
                "space" => Ok(pt + props_f64(&props, &["height", "h", "size"], 12.0) + pb),
                "rule" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    let h = props_f64(&props, &["height", "h"], 1.0).max(1.0);
                    place(&*v, ix, iy, iw, h);
                    freeze_autoresizing(&*v);
                    *slot += 1;
                    Ok(pt + h.max(8.0) + pb)
                }
                "webview" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    if !v.isKindOfClass(WKWebView::class()) {
                        return Err(());
                    }
                    let th = scroll_outer_height(&props, avail_h);
                    // shell: pinned edge-to-edge at build time — a keyboard toggle
                    // (or any state change) re-runs this patch, and re-placing the
                    // webview through the inset layout flow yanked it out of its
                    // full-bleed frame (screen went black behind it).
                    if !props_bool(&props, &["shell"], false) {
                        place(&*v, ix, iy, iw, th);
                        freeze_autoresizing(&*v);
                    }
                    *slot += 1;
                    Ok(pt + th + pb)
                }
                "text" => {
                    let v = subview(parent, *slot).ok_or(())?;
                    if !v.isKindOfClass(UILabel::class()) {
                        return Err(());
                    }
                    let label: &UILabel = unsafe { &*(std::ptr::from_ref(&*v).cast()) };
                    let text = text_from_children(&children);
                    let cur = label
                        .text()
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    if cur != text {
                        label.setText(Some(&NSString::from_str(&text)));
                    }
                    style_label(label, Some(raw_tag));
                    let h = props_f64(
                        &props,
                        &["height", "h"],
                        measure_label_height(label, iw, Some(raw_tag)),
                    );
                    place(label, ix, iy, iw, h);
                    freeze_autoresizing(label);
                    *slot += 1;
                    Ok(pt + h + pb)
                }
                _ => {
                    let mut old_elems = Vec::new();
                    let mut new_elems = Vec::new();
                    collect_element_vnodes(&vnode_children(om), &mut old_elems);
                    collect_element_vnodes(&children, &mut new_elems);
                    if old_elems.len() != new_elems.len() {
                        return Err(());
                    }
                    if old_elems.is_empty() {
                        return Ok(pt + pb);
                    }
                    let mut cy = iy;
                    let mut total = 0.0_f64;
                    for (co, cn) in old_elems.iter().zip(new_elems.iter()) {
                        let h = patch_vnode(co, cn, parent, slot, ix, cy, iw, None, ctx)?;
                        cy += h;
                        total += h;
                    }
                    Ok(pt + total + pb)
                }
            }
        }
        _ => Err(()),
    }
}
