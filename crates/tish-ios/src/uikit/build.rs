//! Map committed vnodes to UIKit views (manual frames + autoresizing masks).

use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{sel, ClassType, MainThreadMarker};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSObjectProtocol, NSString, NSURL, NSURLRequest};
use objc2_ui_kit::{
    UIButton, UIButtonType, UIControlEvents, UIControlState, UIColor, UIFont, UIImage,
    UIImageView, UILabel, UIScrollView, UISegmentedControl, UISlider, UISwitch,
    UITextBorderStyle, UITextField, UITextInputTraits, UITextView, UIView,
    UIViewAutoresizing, UIViewContentMode,
};
use tishlang_apple_common::handlers::{
    register_bool_handler, register_click_handler, register_f64_handler,
    register_text_change_handler,
};
use tishlang_apple_common::style::{props_bool, props_f64, props_string};
use tishlang_apple_common::tag::canonical_host_tag;
use tishlang_core::{ObjectMap, PropMap, Value};
use tishlang_ui::runtime::{is_fragment_tag, RootId};

use super::router::IosControlRouter;
use super::text_view_delegate::IosTextViewDelegate;
use super::webview_bridge;
use super::wk_webview::WKWebView;

#[derive(Clone)]
pub struct BuildCtx {
    pub mtm: MainThreadMarker,
    pub router: Retained<IosControlRouter>,
    pub text_view_delegate: Retained<IosTextViewDelegate>,
    pub root_id: RootId,
}

pub(crate) fn place(view: &UIView, x: f64, y: f64, w: f64, h: f64) {
    view.setFrame(CGRect::new(
        CGPoint::new(x, y),
        CGSize::new(w.max(0.0), h.max(0.0)),
    ));
}

pub(crate) fn freeze_autoresizing(view: &UIView) {
    view.setAutoresizingMask(UIViewAutoresizing::empty());
}

pub(crate) fn fill_autoresizing(view: &UIView) {
    view.setAutoresizingMask(
        UIViewAutoresizing::FlexibleWidth | UIViewAutoresizing::FlexibleHeight,
    );
}

pub(crate) fn vnode_children(obj: &PropMap) -> Vec<Value> {
    match obj.get("children") {
        Some(Value::Array(a)) => a.borrow().clone(),
        _ => vec![],
    }
}

pub(crate) fn vnode_props(obj: &PropMap) -> PropMap {
    match obj.get("props") {
        Some(Value::Object(o)) => o.borrow().strings.clone(),
        _ => PropMap::default(),
    }
}

pub(crate) fn collect_element_vnodes(children: &[Value], out: &mut Vec<Value>) {
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

pub(crate) fn padding_insets(props: &PropMap) -> (f64, f64, f64, f64) {
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

pub(crate) fn scroll_outer_height(props: &PropMap, avail_h: Option<f64>) -> f64 {
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

pub(crate) fn text_from_children(children: &[Value]) -> String {
    children
        .iter()
        .map(|v| v.to_display_string())
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string()
}

pub fn clear_subviews(view: &UIView) {
    let subs = view.subviews();
    let n = subs.count();
    for i in (0..n).rev() {
        let sv = subs.objectAtIndex(i);
        if sv.isKindOfClass(WKWebView::class()) {
            let wv: &WKWebView = unsafe { &*std::ptr::from_ref(&*sv).cast::<WKWebView>() };
            webview_bridge::detach_bridge(wv);
        }
        sv.removeFromSuperview();
    }
}

fn propmap_to_object_map(pm: &PropMap) -> ObjectMap {
    pm.iter()
        .map(|(k, v)| (std::sync::Arc::clone(k), v.clone()))
        .collect()
}

pub(crate) fn style_label(label: &UILabel, tag: Option<&str>) {
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

pub(crate) fn measure_text_height(text: &str, width: f64, tag: Option<&str>) -> f64 {
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

fn canvas_rgba_from_value(canvas: &Value) -> Option<(usize, usize, Vec<u8>)> {
    if let Some((w, h, rgba)) = tishlang_apple_common::canvas::canvas_rgba_bytes(canvas) {
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
            let _ = f.call(&[]);
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

fn wire_text_change(props: &PropMap, tf: &UITextField, ctx: &BuildCtx) {
    if let Some(Value::Function(f)) = props.get("onChange").or_else(|| props.get("onInput")) {
        let f = f.clone();
        let idx = register_text_change_handler(
            ctx.root_id,
            Rc::new(move |s: String| {
                let _ = f.call(&[Value::String(s.into())]);
            }),
        ) as isize;
        tf.setTag(idx);
        unsafe {
            let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
            tf.addTarget_action_forControlEvents(
                Some(&*p),
                sel!(jsxTextChanged:),
                UIControlEvents::EditingChanged,
            );
        }
    }
}

fn wire_bool_change(props: &PropMap, sw: &UISwitch, ctx: &BuildCtx) {
    if let Some(Value::Function(f)) = props
        .get("onChange")
        .or_else(|| props.get("onToggle"))
    {
        let f = f.clone();
        let idx = register_bool_handler(
            ctx.root_id,
            Rc::new(move |b| {
                let _ = f.call(&[Value::Bool(b)]);
            }),
        ) as isize;
        sw.setTag(idx);
        unsafe {
            let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
            sw.addTarget_action_forControlEvents(
                Some(&*p),
                sel!(jsxBoolChanged:),
                UIControlEvents::ValueChanged,
            );
        }
    }
}

fn wire_slider_change(props: &PropMap, sl: &UISlider, ctx: &BuildCtx) {
    if let Some(Value::Function(f)) = props.get("onChange").or_else(|| props.get("onInput")) {
        let f = f.clone();
        let idx = register_f64_handler(
            ctx.root_id,
            Rc::new(move |v| {
                let _ = f.call(&[Value::Number(v)]);
            }),
        ) as isize;
        sl.setTag(idx);
        unsafe {
            let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
            sl.addTarget_action_forControlEvents(
                Some(&*p),
                sel!(jsxSliderChanged:),
                UIControlEvents::ValueChanged,
            );
        }
    }
}

fn wire_segment_change(props: &PropMap, seg: &UISegmentedControl, ctx: &BuildCtx) {
    // Always wire for show/hide; optional onChange receives selected index.
    if let Some(Value::Function(f)) = props.get("onChange").or_else(|| props.get("onSelect")) {
        let f = f.clone();
        let idx = register_f64_handler(
            ctx.root_id,
            Rc::new(move |v| {
                let _ = f.call(&[Value::Number(v)]);
            }),
        ) as isize;
        seg.setTag(idx);
    }
    unsafe {
        let p = Retained::as_ptr(&ctx.router).cast::<AnyObject>();
        seg.addTarget_action_forControlEvents(
            Some(&*p),
            sel!(jsxSegmentChanged:),
            UIControlEvents::ValueChanged,
        );
    }
}

fn wire_text_view_change(props: &PropMap, tv: &UITextView, ctx: &BuildCtx) {
    if let Some(Value::Function(f)) = props.get("onChange").or_else(|| props.get("onInput")) {
        let f = f.clone();
        let idx = register_text_change_handler(
            ctx.root_id,
            Rc::new(move |s: String| {
                let _ = f.call(&[Value::String(s.into())]);
            }),
        ) as isize;
        tv.setTag(idx);
        unsafe {
            tv.setDelegate(Some(ProtocolObject::from_ref(&*ctx.text_view_delegate)));
        }
    }
}

fn collect_tab_specs(children: &[Value]) -> Vec<(String, Vec<Value>)> {
    let mut out = Vec::new();
    let mut elems = Vec::new();
    collect_element_vnodes(children, &mut elems);
    for c in elems {
        match &c {
            Value::Object(o) => {
                let m = &o.borrow().strings;
                let is_tab = matches!(
                    m.get("tag"),
                    Some(Value::String(s)) if {
                        let t = s.as_str();
                        t == "tab" || t == "Tab"
                    }
                );
                let p = vnode_props(m);
                let lbl = props_string(&p, &["label", "title", "name"])
                    .unwrap_or_else(|| "Tab".into());
                let body = if is_tab {
                    vnode_children(m)
                } else {
                    vec![c.clone()]
                };
                out.push((lbl, body));
            }
            _ => out.push(("Tab".into(), vec![c.clone()])),
        }
    }
    out
}

fn make_text_field(mtm: MainThreadMarker, props: &PropMap, secure: bool) -> Retained<UITextField> {
    let tf = UITextField::new(mtm);
    tf.setBorderStyle(UITextBorderStyle::RoundedRect);
    let value = props_string(props, &["value", "defaultValue"]).unwrap_or_default();
    tf.setText(Some(&NSString::from_str(&value)));
    if let Some(ph) = props_string(props, &["placeholder", "hint"]) {
        tf.setPlaceholder(Some(&NSString::from_str(&ph)));
    }
    if secure {
        tf.setSecureTextEntry(true);
    }
    tf
}

fn load_ui_image(src: &str, symbol: bool) -> Option<Retained<UIImage>> {
    let name = NSString::from_str(src);
    if symbol {
        return UIImage::systemImageNamed(&name);
    }
    if src.starts_with('/') || src.contains('/') {
        return UIImage::imageWithContentsOfFile(&name);
    }
    UIImage::imageNamed(&name).or_else(|| UIImage::systemImageNamed(&name))
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
            let t = s.as_str().trim();
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
                    let gap = props_f64(&props, &["gap", "columnGap", "column_gap"], 0.0);
                    let total_gap = gap * (n.saturating_sub(1) as f64);
                    let cw = ((iw - total_gap) / n as f64).max(0.0);
                    let mut cx = ix;
                    let mut max_h = 0.0_f64;
                    for (i, c) in elems.iter().enumerate() {
                        let h = layout_vnode(c, parent, cx, iy, cw, None, ctx);
                        max_h = max_h.max(h);
                        cx += cw;
                        if i + 1 < elems.len() {
                            cx += gap;
                        }
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
                "textinput" => {
                    let tf = make_text_field(mtm, &props, false);
                    wire_text_change(&props, &tf, ctx);
                    let h = props_f64(&props, &["height", "h"], 44.0);
                    place(&tf, ix, iy, iw, h);
                    freeze_autoresizing(&tf);
                    parent.addSubview(&tf);
                    pt + h + pb
                }
                "password" => {
                    let tf = make_text_field(mtm, &props, true);
                    wire_text_change(&props, &tf, ctx);
                    let h = props_f64(&props, &["height", "h"], 44.0);
                    place(&tf, ix, iy, iw, h);
                    freeze_autoresizing(&tf);
                    parent.addSubview(&tf);
                    pt + h + pb
                }
                "toggler" | "checkbox" => {
                    let sw = UISwitch::new(mtm);
                    let on = props_bool(&props, &["checked", "value"], false);
                    sw.setOn_animated(on, false);
                    wire_bool_change(&props, &sw, ctx);
                    let h = props_f64(&props, &["height", "h"], 44.0);
                    let w = 60.0_f64.min(iw);
                    place(&sw, ix, iy + ((h - 31.0) * 0.5).max(0.0), w, 31.0);
                    freeze_autoresizing(&sw);
                    parent.addSubview(&sw);
                    pt + h + pb
                }
                "slider" => {
                    let sl = UISlider::new(mtm);
                    let minv = props_f64(&props, &["min"], 0.0) as f32;
                    let maxv = props_f64(&props, &["max"], 100.0) as f32;
                    sl.setMinimumValue(minv);
                    sl.setMaximumValue(maxv);
                    sl.setValue(props_f64(&props, &["value"], minv as f64) as f32);
                    wire_slider_change(&props, &sl, ctx);
                    let h = props_f64(&props, &["height", "h"], 44.0);
                    place(&sl, ix, iy, iw, h);
                    freeze_autoresizing(&sl);
                    parent.addSubview(&sl);
                    pt + h + pb
                }
                "text_editor" => {
                    let base_h = scroll_outer_height(&props, avail_h);
                    let min_h = props_f64(&props, &["minHeight", "min_height"], 120.0);
                    let th = base_h.max(min_h);
                    let tv = UITextView::new(mtm);
                    tv.setEditable(true);
                    let fs = props_f64(&props, &["fontSize", "font_size"], 17.0);
                    tv.setFont(Some(&UIFont::systemFontOfSize(fs)));
                    tv.setTextColor(Some(&UIColor::labelColor()));
                    tv.setBackgroundColor(Some(&UIColor::secondarySystemBackgroundColor()));
                    let initial =
                        props_string(&props, &["value", "defaultValue"]).unwrap_or_default();
                    tv.setText(Some(&NSString::from_str(&initial)));
                    wire_text_view_change(&props, &tv, ctx);
                    place(&tv, ix, iy, iw, th);
                    freeze_autoresizing(&tv);
                    parent.addSubview(&tv);
                    pt + th + pb
                }
                "tabs" => {
                    let th = props_f64(&props, &["height", "h"], avail_h.unwrap_or(200.0));
                    let tab_specs = collect_tab_specs(&children);
                    let seg_h = 32.0_f64;
                    let gap = 8.0_f64;
                    let pane_h = (th - seg_h - gap).max(40.0);
                    let outer = UIView::new(mtm);
                    place(&outer, ix, iy, iw, th);
                    freeze_autoresizing(&outer);
                    let seg = UISegmentedControl::new(mtm);
                    for (i, (label, _)) in tab_specs.iter().enumerate() {
                        seg.insertSegmentWithTitle_atIndex_animated(
                            Some(&NSString::from_str(label)),
                            i,
                            false,
                        );
                    }
                    let selected = props_f64(&props, &["selected", "value"], 0.0) as isize;
                    let max_i = (tab_specs.len().saturating_sub(1)) as isize;
                    let selected = selected.max(0).min(max_i.max(0));
                    if !tab_specs.is_empty() {
                        seg.setSelectedSegmentIndex(selected);
                    }
                    place(&seg, 0.0, 0.0, iw, seg_h);
                    freeze_autoresizing(&seg);
                    outer.addSubview(&seg);
                    wire_segment_change(&props, &seg, ctx);
                    let content = UIView::new(mtm);
                    place(&content, 0.0, seg_h + gap, iw, pane_h);
                    freeze_autoresizing(&content);
                    for (i, (_, body)) in tab_specs.iter().enumerate() {
                        let pane = UIView::new(mtm);
                        place(&pane, 0.0, 0.0, iw, pane_h);
                        freeze_autoresizing(&pane);
                        let mut y = 0.0_f64;
                        for cc in body {
                            let h = layout_vnode(cc, &pane, 0.0, y, iw, Some(pane_h), ctx);
                            y += h;
                        }
                        pane.setHidden((i as isize) != selected);
                        content.addSubview(&pane);
                    }
                    outer.addSubview(&content);
                    parent.addSubview(&outer);
                    pt + th + pb
                }
                "image" => {
                    let src = props_string(&props, &["src", "path", "url"]).unwrap_or_default();
                    let symbol = props_bool(&props, &["symbol", "sfSymbol", "sf_symbol"], false);
                    let iv = UIImageView::new(mtm);
                    iv.setContentMode(UIViewContentMode::ScaleAspectFit);
                    iv.setClipsToBounds(true);
                    if let Some(img) = load_ui_image(&src, symbol) {
                        iv.setImage(Some(&img));
                    }
                    let h = props_f64(&props, &["height", "h"], 48.0);
                    place(&iv, ix, iy, iw, h);
                    freeze_autoresizing(&iv);
                    parent.addSubview(&iv);
                    pt + h + pb
                }
                "space" => {
                    let h = props_f64(&props, &["height", "h", "size"], 12.0);
                    pt + h + pb
                }
                "rule" => {
                    let line = UIView::new(mtm);
                    line.setBackgroundColor(Some(&UIColor::grayColor()));
                    let h = props_f64(&props, &["height", "h"], 1.0).max(1.0);
                    place(&line, ix, iy, iw, h);
                    freeze_autoresizing(&line);
                    parent.addSubview(&line);
                    pt + h.max(8.0) + pb
                }
                "webview" => {
                    let th = scroll_outer_height(&props, avail_h);
                    let src = props_string(&props, &["src", "url"]).unwrap_or_default();
                    let html = props_string(&props, &["html", "htmlContent", "document"]);
                    let frame = CGRect::new(CGPoint::ZERO, CGSize::new(iw, th));
                    let wv = webview_bridge::create_webview(ctx.mtm, ctx.root_id, frame, &props);
                    load_webview_content(&wv, html.as_deref(), &src);
                    place(&*wv, ix, iy, iw, th);
                    freeze_autoresizing(&*wv);
                    parent.addSubview(&*wv);
                    pt + th + pb
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
                    let scene_obj = propmap_to_object_map(&scene_props);
                    let view = tishlang_apple_common::scene_host::create_scene_view(
                        mtm,
                        iw,
                        h,
                        Some(&scene_obj),
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

/// Load HTML into a WKWebView. Prefer `html` / `loadHTMLString` over raw `data:` URLs —
/// `NSURL` rejects unencoded `data:text/html,…` bodies (spaces/quotes → blank pane).
fn load_webview_content(wv: &WKWebView, html: Option<&str>, src: &str) {
    if let Some(doc) = html.filter(|s| !s.is_empty()) {
        unsafe {
            wv.loadHTMLString_baseURL(&NSString::from_str(doc), None);
        }
        return;
    }
    if src.is_empty() {
        return;
    }
    const PREFIX: &str = "data:text/html,";
    if let Some(rest) = src.strip_prefix(PREFIX) {
        // Accept both raw and percent-encoded bodies after the comma.
        let body = percent_decode_minimal(rest);
        unsafe {
            wv.loadHTMLString_baseURL(&NSString::from_str(&body), None);
        }
        return;
    }
    if let Some(url) = NSURL::URLWithString(&NSString::from_str(src)) {
        let req = NSURLRequest::requestWithURL(&url);
        unsafe {
            wv.loadRequest(&req);
        }
    }
}

fn percent_decode_minimal(s: &str) -> String {
    // Only needed when callers percent-encode; raw HTML passes through unchanged
    // when it contains no `%` sequences WK would have rejected as a URL anyway.
    if !s.contains('%') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h = |c: u8| -> Option<u8> {
                match c {
                    b'0'..=b'9' => Some(c - b'0'),
                    b'a'..=b'f' => Some(c - b'a' + 10),
                    b'A'..=b'F' => Some(c - b'A' + 10),
                    _ => None,
                }
            };
            if let (Some(a), Some(b)) = (h(bytes[i + 1]), h(bytes[i + 2])) {
                out.push((a << 4) | b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// Content insets for the root host view: safe-area + a small gutter.
/// Returns `(left, top, right, bottom)`.
pub(crate) fn root_content_insets(root: &UIView) -> (f64, f64, f64, f64) {
    let gutter = 8.0;
    let insets = root.safeAreaInsets();
    let mut top = insets.top as f64;
    let mut bottom = insets.bottom as f64;
    let left = insets.left as f64;
    let right = insets.right as f64;
    // Before the window finishes laying out, safe-area can be zero — keep
    // controls below the status bar / Dynamic Island on modern iPhones.
    if top < 20.0 {
        top = 59.0;
    }
    if bottom < 1.0 {
        bottom = 34.0;
    }
    (
        left + gutter,
        top + gutter,
        right + gutter,
        bottom + gutter,
    )
}

/// Commit a vnode tree into `root`. Prefers in-place patch when `prev` matches shape.
pub fn build_into(
    v: &Value,
    root: &UIView,
    width: f64,
    height: f64,
    ctx: &BuildCtx,
    prev: Option<&Value>,
) {
    if let Some(p) = prev {
        if super::patch::try_patch_vtree(p, v, root, width, height, ctx).is_some() {
            return;
        }
    }
    clear_subviews(root);
    tishlang_apple_common::handlers::clear_all_handlers_for_root(ctx.root_id);
    fill_autoresizing(root);
    place(root, 0.0, 0.0, width, height);
    root.setBackgroundColor(Some(&UIColor::systemBackgroundColor()));
    let (left, top, right, bottom) = root_content_insets(root);
    let inner_w = (width - left - right).max(0.0);
    let avail_h = (height - top - bottom).max(0.0);
    let _ = layout_vnode(v, root, left, top, inner_w, Some(avail_h), ctx);
}
