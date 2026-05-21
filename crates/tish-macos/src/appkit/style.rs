//! React-style `style` object helpers: CSS-like subset mapped to AppKit.

use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSColor, NSFont, NSTextAlignment, NSTextField, NSTextView, NSView};
use objc2_core_foundation::CGFloat;
use tishlang_core::{ObjectMap, Value};

/// Returns true if layer-backed view styling from merged props should apply.
pub(super) fn has_container_layer_style(props: &ObjectMap) -> bool {
    let bg_significant = match props_string(props, &["backgroundColor", "background"]) {
        Some(ref s) => {
            let t = s.trim().to_ascii_lowercase();
            t != "transparent" && t != "none" && t != "clear"
        }
        None => false,
    };
    bg_significant
        || props_f64(props, &["opacity"], 1.0) != 1.0
        || props_f64(props, &["borderRadius"], 0.0) > 0.0
        || props_f64(props, &["borderWidth"], 0.0) > 0.0
        || props_string(props, &["borderColor"]).is_some()
}

fn props_string(props: &ObjectMap, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(Value::String(s)) = props.get(*k) {
            return Some(s.to_string());
        }
    }
    None
}

fn props_f64(props: &ObjectMap, keys: &[&str], default: f64) -> f64 {
    for k in keys {
        if let Some(n) = props.get(*k).and_then(|v| v.as_number()) {
            return n;
        }
    }
    default
}

/// Parse `#RGB`, `#RRGGBB`, `#RRGGBBAA` into sRGB components (0..1) and alpha.
pub(super) fn parse_hex_color(s: &str) -> Option<(f64, f64, f64, f64)> {
    let t = s.trim();
    let hex = t.strip_prefix('#')?;
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect();
    match bytes.len() {
        3 => Some((
            bytes[0] as f64 / 255.0,
            bytes[1] as f64 / 255.0,
            bytes[2] as f64 / 255.0,
            1.0,
        )),
        4 => Some((
            bytes[0] as f64 / 255.0,
            bytes[1] as f64 / 255.0,
            bytes[2] as f64 / 255.0,
            bytes[3] as f64 / 255.0,
        )),
        _ => None,
    }
}

/// Hex, basic keywords, and dynamic AppKit semantic colors (`label`, `controlAccent`, …).
pub(super) fn resolve_ns_color(s: &str) -> Option<Retained<NSColor>> {
    let t = s.trim();
    if let Some(rgba) = parse_hex_color(t) {
        return Some(NSColor::colorWithSRGBRed_green_blue_alpha(
            rgba.0, rgba.1, rgba.2, rgba.3,
        ));
    }
    let lower = t.to_ascii_lowercase();
    match lower.as_str() {
        "white" => Some(NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 1.0, 1.0, 1.0)),
        "black" => Some(NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 0.0, 0.0, 1.0)),
        "transparent" => Some(NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 0.0, 0.0, 0.0)),
        "label" => Some(NSColor::labelColor()),
        "secondarylabel" | "secondary_label" => Some(NSColor::secondaryLabelColor()),
        "tertiarylabel" | "tertiary_label" => Some(NSColor::tertiaryLabelColor()),
        "quaternarylabel" | "quaternary_label" => Some(NSColor::quaternaryLabelColor()),
        "separator" | "separatorcolor" | "separator_color" => Some(NSColor::separatorColor()),
        "grid" | "gridcolor" | "grid_color" => Some(NSColor::gridColor()),
        "controlaccent" | "control_accent" | "accent" => Some(NSColor::controlAccentColor()),
        "windowbackground" | "window_background" => Some(NSColor::windowBackgroundColor()),
        "textbackground" | "text_background" => Some(NSColor::textBackgroundColor()),
        _ => None,
    }
}

pub(super) fn apply_layer_style_to_view(view: &NSView, props: &ObjectMap) {
    let bg = props_string(props, &["backgroundColor", "background"]);
    let bg_paint = bg.as_deref().and_then(|s| {
        let t = s.trim().to_ascii_lowercase();
        if t == "transparent" || t == "none" || t == "clear" {
            None
        } else {
            Some(s)
        }
    });
    let opacity = props_f64(props, &["opacity"], 1.0);
    let radius = props_f64(props, &["borderRadius"], 0.0);
    let bw = props_f64(props, &["borderWidth"], 0.0);
    let border = props_string(props, &["borderColor"]);

    let need_layer = bg_paint.is_some()
        || opacity < 1.0 - 1e-6
        || opacity > 1.0 + 1e-6
        || radius > 0.0
        || bw > 0.0
        || border.is_some();
    if !need_layer {
        return;
    }

    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
        use std::ops::Deref;
        let l = layer.deref();
        if let Some(s) = bg_paint {
            if let Some(c) = resolve_ns_color(s) {
                let cg = c.CGColor();
                l.setBackgroundColor(Some(&*cg));
            }
        }
        if opacity < 1.0 - 1e-6 || opacity > 1.0 + 1e-6 {
            l.setOpacity(opacity as f32);
        }
        if radius > 0.0 {
            l.setCornerRadius(radius as CGFloat);
            l.setMasksToBounds(true);
        }
        if bw > 0.0 {
            l.setBorderWidth(bw as CGFloat);
            if let Some(ref bs) = border {
                if let Some(bc) = resolve_ns_color(bs) {
                    let cg = bc.CGColor();
                    l.setBorderColor(Some(&*cg));
                }
            }
        }
    }
}

/// `NSTextView` document fill: default is transparent (`drawsBackground` false) so `NSVisualEffectView`
/// shows through; opt in with `drawsBackground` + optional `backgroundColor` / `background`.
pub(super) fn apply_nstext_view_document_background_from_props(tv: &NSTextView, props: &ObjectMap) {
    let draw = props_bool(props, &["drawsBackground", "draws_background"]);
    tv.setDrawsBackground(draw);
    if draw {
        if let Some(ref s) = props_string(props, &["backgroundColor", "background"]) {
            if let Some(c) = resolve_ns_color(s) {
                tv.setBackgroundColor(&c);
            }
        }
    }
}

fn props_bool(props: &ObjectMap, keys: &[&str]) -> bool {
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

fn font_weight_from_props(props: &ObjectMap) -> f64 {
    match props.get("fontWeight") {
        Some(Value::Number(n)) => *n,
        Some(Value::String(s)) => {
            let x = s.to_ascii_lowercase();
            match x.as_str() {
                "bold" | "700" => 0.6,
                "semibold" | "600" => 0.45,
                "medium" | "500" => 0.35,
                "regular" | "normal" | "400" => 0.0,
                _ => 0.0,
            }
        }
        _ => 0.0,
    }
}

/// Non-editable label fields: no bezel, no background fill, adapts to dark mode via `labelColor` when
/// `style.color` is omitted.
pub(super) fn apply_static_label_text_field(tf: &NSTextField, props: &ObjectMap, mtm: MainThreadMarker) {
    tf.setBezeled(false);
    tf.setEditable(false);
    tf.setDrawsBackground(false);
    apply_text_style(tf, props, mtm);
    if props_string(props, &["color"]).is_none() {
        let c = NSColor::labelColor();
        tf.setTextColor(Some(&c));
    }
}

pub(super) fn apply_text_style(tf: &NSTextField, props: &ObjectMap, mtm: MainThreadMarker) {
    let size = props_f64(props, &["fontSize"], 0.0);
    let w = font_weight_from_props(props);
    if size > 0.0 {
        let font = NSFont::systemFontOfSize_weight(size as CGFloat, w as CGFloat);
        tf.setFont(Some(&font));
    } else if w > 0.01 {
        let base = tf.font().map(|f| f.pointSize() as f64).unwrap_or(13.0);
        let font = NSFont::systemFontOfSize_weight(base as CGFloat, w as CGFloat);
        tf.setFont(Some(&font));
    }
    if let Some(ref c) = props_string(props, &["color"]) {
        if let Some(col) = resolve_ns_color(c) {
            tf.setTextColor(Some(&col));
        }
    }
    if let Some(ref a) = props_string(props, &["textAlign", "align", "textAlignment"]) {
        let al = match a.to_ascii_lowercase().as_str() {
            "left" => NSTextAlignment::Left,
            "center" => NSTextAlignment(1),
            "right" => NSTextAlignment(2),
            _ => NSTextAlignment::Natural,
        };
        tf.setAlignment(al);
    }
    let _ = mtm;
}

/// Typographic line height (ascender − descender + leading). Matches a single text line’s bounds.
pub(super) fn ns_font_line_height(font: &NSFont) -> f64 {
    let asc = font.ascender() as f64;
    let desc = font.descender() as f64;
    let lead = font.leading() as f64;
    (asc - desc + lead).ceil().max(1.0)
}

fn explicit_label_height_from_props(props: &ObjectMap) -> Option<f64> {
    for k in ["height", "h"] {
        if let Some(n) = props.get(k).and_then(|v| v.as_number()) {
            if n > 0.0 {
                return Some(n);
            }
        }
    }
    None
}

/// Frame height for a non-wrapping static label after [`apply_static_label_text_field`].
/// Uses font metrics so `Row` `alignItems: "center"` lines up with compact views (e.g. SF Symbol rows).
pub(super) fn single_line_label_height_after_style(tf: &NSTextField, props: &ObjectMap) -> f64 {
    if let Some(h) = explicit_label_height_from_props(props) {
        return h;
    }
    match tf.font() {
        Some(f) => ns_font_line_height(&f),
        None => 16.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors() {
        let (r, g, b, a) = parse_hex_color("#ff00ff").unwrap();
        assert!((r - 1.0).abs() < 1e-6 && (g - 0.0).abs() < 1e-6 && (b - 1.0).abs() < 1e-6);
        assert!((a - 1.0).abs() < 1e-6);
        let (_, _, _, a2) = parse_hex_color("#11223344").unwrap();
        assert!(a2 > 0.0 && a2 < 1.0);
    }
}
