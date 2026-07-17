//! Map committed vnodes to UIKit views (manual frames + autoresizing masks).

use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{sel, MainThreadMarker};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::NSString;
use objc2_ui_kit::{
    UIButton, UIButtonType, UIControlEvents, UIControlState, UIColor, UIFont, UIImage,
    UIImageView, UILabel, UIScrollView, UIView, UIViewAutoresizing, UIViewContentMode,
};
use tish_apple_common::handlers::register_click_handler;
use tish_apple_common::style::props_f64;
use tish_apple_common::tag::canonical_host_tag;
use tishlang_core::{ObjectMap, PropMap, Value};
use tishlang_ui::runtime::{is_fragment_tag, RootId};

use super::router::IosControlRouter;

#[derive(Clone)]
pub struct BuildCtx {
    pub mtm: MainThreadMarker,
    pub router: Retained<IosControlRouter>,
    pub root_id: RootId,
}

pub fn clear_subviews(view: &UIView) {
    let subs = view.subviews();
    let n = subs.count();
    for i in (0..n).rev() {
        subs.objectAtIndex(i).removeFromSuperview();
    }
}

fn vnode_children(obj: &PropMap) -> Vec<Value> {
    match obj.get("children") {
        Some(Value::Array(a)) => a.borrow().clone(),
        _ => vec![],
    }
}

fn vnode_props(obj: &PropMap) -> PropMap {
    match obj.get("props") {
        Some(Value::Object(o)) => o.borrow().strings.clone(),
        _ => ObjectMap::default(),
    }
}

fn collect_element_vnodes(children: &[Value], out: &mut Vec<Value>) {
    for c in children {
        match c {
            Value::Object(o) => {
                let m = &o.borrow().strings;
                let tag = m.get("tag").unwrap_or(&Value::Null);
                if is_fragment_tag(tag) {
                    let inner = vnode_children(m);
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

fn text_from_children(children: &[Value]) -> String {
    children
        .iter()
        .map(|v| v.to_display_string())
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string()
}

fn style_label(label: &UILabel, tag: Option<&str>) {
    unsafe {
        label.setTextColor(Some(&UIColor::labelColor()));
        label.setNumberOfLines(0);
        label.setLineBreakMode(objc2_ui_kit::NSLineBreakMode::ByWordWrapping);
        let font = match tag {
            Some("h1") => UIFont::boldSystemFontOfSize(28.0),
            Some("h2") => UIFont::boldSystemFontOfSize(24.0),
            Some("h3") => UIFont::boldSystemFontOfSize(20.0),
            Some("p") => UIFont::systemFontOfSize(17.0),
            _ => UIFont::systemFontOfSize(17.0),
        };
        label.setFont(Some(&font));
    }
}

fn default_text_height(tag: Option<&str>) -> f64 {
    match tag {
        Some("h1") => 28.0,
        Some("h2") => 24.0,
        Some("h3") => 22.0,
        Some("p") => 18.0,
        _ => 18.0,
    }
}

fn measure_text_height(text: &str, width: f64, tag: Option<&str>) -> f64 {
    let min = default_text_height(tag);
    if text.is_empty() {
        return min;
    }
    let line_h = match tag {
        Some("h1") => 28.0,
        _ => 18.0,
    };
    let chars_per_line = (width / 8.0).max(24.0);
    let lines = (text.len() as f64 / chars_per_line).ceil().max(1.0);
    (lines * line_h).max(min)
}

fn place(view: &UIView, x: f64, y: f64, w: f64, h: f64) {
    view.setFrame(CGRect::new(
        CGPoint::new(x, y),
        CGSize::new(w.max(0.0), h.max(0.0)),
    ));
}

fn freeze_autoresizing(view: &UIView) {
    view.setAutoresizingMask(UIViewAutoresizing::empty());
}

fn fill_autoresizing(view: &UIView) {
    view.setAutoresizingMask(
        UIViewAutoresizing::FlexibleWidth | UIViewAutoresizing::FlexibleHeight,
    );
}

fn canvas_rgba_from_value(canvas: &Value) -> Option<(usize, usize, Vec<u8>)> {
    if let Some((w, h, rgba)) = tish_apple_common::canvas::canvas_rgba_bytes(canvas) {
        return Some((w as usize, h as usize, rgba));
    }
    let Value::Object(obj) = canvas else {
        return None;
    };
    let m = &obj.borrow().strings;
    let w = m.get("width")?.as_number()? as usize;
    let h = m.get("height")?.as_number()? as usize;
    let Value::Array(arr) = m.get("__pixels")? else {
        return None;
    };
    let rgba: Vec<u8> = arr
        .borrow()
        .iter()
        .filter_map(|v| v.as_number().map(|n| n.round().clamp(0.0, 255.0) as u8))
        .collect();
    if rgba.len() < w * h * 4 {
        return None;
    }
    Some((w, h, rgba))
}

fn ui_image_from_rgba(w: usize, h: usize, rgba: &[u8]) -> Option<Retained<UIImage>> {
    use objc2_core_foundation::CFData;
    use objc2_core_graphics::{
        CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo,
        CGImageByteOrderInfo, CGBitmapInfo,
    };

    let data = CFData::from_bytes(rgba);
    let provider = CGDataProvider::with_cf_data(Some(&data))?;
    let space = CGColorSpace::new_device_rgb()?;
    let bitmap_info = CGBitmapInfo(CGImageAlphaInfo::Last.0 | CGImageByteOrderInfo::Order32Big.0);
    let cg = unsafe {
        CGImage::new(
            w,
            h,
            8,
            32,
            w * 4,
            Some(&space),
            bitmap_info,
            Some(&provider),
            std::ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }?;
    Some(UIImage::imageWithCGImage(&cg))
}

fn wire_on_click(props: &PropMap, btn: &UIButton, ctx: &BuildCtx) {
    if let Some(Value::Function(f)) = props.get("onClick").or_else(|| props.get("onclick")) {
        let f = f.clone();
        let idx = register_click_handler(ctx.root_id, Rc::new(move || {
            let _ = f(&[]);
        })) as isize;
        btn.setTag(idx);
        unsafe {
            let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
            btn.addTarget_action_forControlEvents(
                Some(&*p),
                sel!(jsxClick:),
                UIControlEvents::TouchUpInside,
            );
        }
    }
}

fn layout_vnode(
    v: &Value,
    parent: &UIView,
    x: f64,
    y: f64,
    avail_w: f64,
    avail_h: Option<f64>,
    ctx: &BuildCtx,
) -> f64 {
    let mtm = ctx.mtm;
    match v {
        Value::String(s) => {
            let t = s.as_ref().trim();
            if t.is_empty() {
                return 0.0;
            }
            let label = UILabel::new(mtm);
            label.setText(Some(&NSString::from_str(t)));
            style_label(&label, None);
            let h = measure_text_height(t, avail_w, None);
            place(&label, x, y, avail_w, h);
            freeze_autoresizing(&label);
            parent.addSubview(&label);
            h
        }
        Value::Object(obj) => {
            let map = &obj.borrow().strings;
            let tag = match map.get("tag") {
                Some(Value::String(s)) => s.to_string(),
                _ => return 0.0,
            };
            let props = vnode_props(map);
            let children = vnode_children(map);
            let (pt, pr, pb, pl) = padding_insets(&props);
            let ix = x + pl;
            let iy = y + pt;
            let iw = (avail_w - pl - pr).max(0.0);

            let raw_tag = tag.as_str();
            match canonical_host_tag(raw_tag) {
                "column" | "section" | "div" => {
                    let mut elems = Vec::new();
                    collect_element_vnodes(&children, &mut elems);
                    let gap = props_f64(&props, &["gap", "rowGap", "row_gap"], 0.0);
                    let mut cy = iy;
                    let mut total = 0.0_f64;
                    for c in &elems {
                        let h = layout_vnode(c, parent, ix, cy, iw, None, ctx);
                        cy += h + gap;
                        total += h + gap;
                    }
                    if !elems.is_empty() && gap > 0.0 {
                        total -= gap;
                    }
                    pt + total + pb
                }
                "row" => {
                    let mut elems = Vec::new();
                    collect_element_vnodes(&children, &mut elems);
                    let n = elems.len().max(1);
                    let cw = iw / n as f64;
                    let mut cx = ix;
                    let mut max_h = 0.0_f64;
                    for c in &elems {
                        let h = layout_vnode(c, parent, cx, iy, cw, None, ctx);
                        max_h = max_h.max(h);
                        cx += cw;
                    }
                    pt + max_h.max(44.0) + pb
                }
                "scrollable" => {
                    let outer_h = scroll_outer_height(&props, avail_h);
                    let scroll = UIScrollView::new(mtm);
                    fill_autoresizing(&scroll);
                    place(&scroll, ix, iy, iw, outer_h);
                    let content = UIView::new(mtm);
                    let mut doc_h = 0.0_f64;
                    let mut dy = 0.0_f64;
                    let mut elems = Vec::new();
                    collect_element_vnodes(&children, &mut elems);
                    for c in &elems {
                        let h = layout_vnode(c, &content, 0.0, dy, iw, None, ctx);
                        dy += h;
                        doc_h += h;
                    }
                    doc_h = doc_h.max(1.0);
                    place(&content, 0.0, 0.0, iw, doc_h);
                    freeze_autoresizing(&content);
                    scroll.addSubview(&content);
                    scroll.setContentSize(CGSize::new(iw, doc_h));
                    parent.addSubview(&scroll);
                    pt + outer_h + pb
                }
                "button" => {
                    let title = text_from_children(&children);
                    let btn = UIButton::buttonWithType(UIButtonType::System, mtm);
                    btn.setTitle_forState(Some(&NSString::from_str(&title)), UIControlState::Normal);
                    wire_on_click(&props, &btn, ctx);
                    let h = props_f64(&props, &["height", "h"], 44.0);
                    place(&btn, ix, iy, iw, h);
                    freeze_autoresizing(&btn);
                    parent.addSubview(&btn);
                    pt + h + pb
                }
                "text" => {
                    let text = text_from_children(&children);
                    let label = UILabel::new(mtm);
                    label.setText(Some(&NSString::from_str(&text)));
                    style_label(&label, Some(raw_tag));
                    let h = props_f64(&props, &["height", "h"], measure_text_height(&text, iw, Some(raw_tag)));
                    place(&label, ix, iy, iw, h);
                    freeze_autoresizing(&label);
                    parent.addSubview(&label);
                    pt + h + pb
                }
                "scene_view" => {
                    let h = props_f64(&props, &["height", "h"], avail_h.unwrap_or(320.0));
                    let mut scene_props = props.clone();
                    scene_props.insert(
                        std::sync::Arc::from("rootId"),
                        Value::Number(ctx.root_id as f64),
                    );
                    let view = tish_apple_common::scene_host::create_scene_view(
                        mtm,
                        iw,
                        h,
                        Some(&scene_props),
                    );
                    place(&view, ix, iy, iw, h);
                    freeze_autoresizing(&view);
                    parent.addSubview(&view);
                    pt + h + pb
                }
                "card_art" => {
                    let card_h = props_f64(&props, &["height", "h"], 100.0);
                    let iv = UIImageView::new(mtm);
                    iv.setContentMode(UIViewContentMode::ScaleAspectFit);
                    iv.setClipsToBounds(true);
                    if let Some(Value::Object(art)) = props.get("art") {
                        if let Some(canvas) = art.borrow().strings.get("canvas") {
                            if let Some((w, h, rgba)) = canvas_rgba_from_value(canvas) {
                                if let Some(img) = ui_image_from_rgba(w, h, &rgba) {
                                    iv.setImage(Some(&img));
                                }
                            }
                        }
                    }
                    place(&iv, ix, iy, iw, card_h);
                    fill_autoresizing(&iv);
                    parent.addSubview(&iv);
                    pt + card_h + pb
                }
                _ => {
                    let mut elems = Vec::new();
                    collect_element_vnodes(&children, &mut elems);
                    if elems.is_empty() {
                        pt + pb
                    } else {
                        let mut cy = iy;
                        let mut total = 0.0_f64;
                        for c in &elems {
                            let h = layout_vnode(c, parent, ix, cy, iw, None, ctx);
                            cy += h;
                            total += h;
                        }
                        pt + total + pb
                    }
                }
            }
        }
        _ => 0.0,
    }
}

fn padding_insets(props: &PropMap) -> (f64, f64, f64, f64) {
    let pt = props_f64(props, &["paddingTop", "padding_top", "pt"], 0.0);
    let pr = props_f64(props, &["paddingRight", "padding_right", "pr"], 0.0);
    let pb = props_f64(props, &["paddingBottom", "padding_bottom", "pb"], 0.0);
    let pl = props_f64(props, &["paddingLeft", "padding_left", "pl"], 0.0);
    let all = props_f64(props, &["padding", "p"], 0.0);
    let pt = if pt > 0.0 { pt } else { all };
    let pr = if pr > 0.0 { pr } else { all };
    let pb = if pb > 0.0 { pb } else { all };
    let pl = if pl > 0.0 { pl } else { all };
    (pt, pr, pb, pl)
}

fn scroll_outer_height(props: &ObjectMap, avail_h: Option<f64>) -> f64 {
    if let Some(h) = props.get("height").and_then(|v| v.as_number()) {
        if h.is_finite() && h > 0.0 {
            return h;
        }
    }
    if let Some(h) = avail_h {
        return h.max(120.0);
    }
    props_f64(props, &["height", "h"], 200.0)
}

/// Commit a vnode tree into `root` (clears existing subviews).
pub fn build_into(v: &Value, root: &UIView, width: f64, height: f64, ctx: &BuildCtx) {
    clear_subviews(root);
    tish_apple_common::handlers::clear_click_handlers_for_root(ctx.root_id);
    fill_autoresizing(root);
    place(root, 0.0, 0.0, width, height);
    root.setBackgroundColor(Some(&UIColor::systemBackgroundColor()));
    let pad = 8.0;
    let top = 8.0;
    let inner_w = (width - pad * 2.0).max(0.0);
    let _ = layout_vnode(v, root, pad, top, inner_w, Some(height - top - pad), ctx);
}
