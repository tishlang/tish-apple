//! Optional diagnostics: unknown vnode props (env-gated).
//!
//! Set `TISH_MACOS_WARN_UNKNOWN_PROPS=1` to log each unknown **top-level** prop key once per process
//! (after `style` merge) for selected tags. Intended for catching typos (e.g. invalid `Row` keys).

use std::collections::HashSet;
use std::sync::Mutex;

use tishlang_core::{ObjectMap, PropMap};

static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// When `true`, [`warn_unknown_props`] may emit `eprintln!` diagnostics.
pub(crate) fn macos_warn_unknown_props_enabled() -> bool {
    match std::env::var_os("TISH_MACOS_WARN_UNKNOWN_PROPS") {
        None => false,
        Some(v) => {
            let s = v.to_string_lossy();
            let t = s.trim();
            t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes")
        }
    }
}

fn prop_key_allowed(key: &str, allowed: &[&str]) -> bool {
    let kl = key.to_ascii_lowercase();
    allowed
        .iter()
        .any(|a| a.to_ascii_lowercase() == kl)
}

/// Log unknown keys in `props` for `tag` once per `(tag, key)` pair.
pub(crate) fn warn_unknown_props(tag: &str, props: &PropMap, allowed: &[&str]) {
    if !macos_warn_unknown_props_enabled() {
        return;
    }
    let Ok(mut guard) = SEEN.lock() else {
        return;
    };
    let seen = guard.get_or_insert_with(HashSet::new);
    for k in props.keys() {
        let ks = k.as_ref();
        if prop_key_allowed(ks, allowed) {
            continue;
        }
        let sig = format!("{tag}::{ks}");
        if seen.insert(sig) {
            eprintln!(
                "[tish-macos] unknown prop `{ks}` on <{tag}> (TISH_MACOS_WARN_UNKNOWN_PROPS=1)"
            );
        }
    }
}

/// Allowed keys after [`super::build::effective_props`] for `<Row>` / `<row>`.
pub(crate) const ROW_PROP_ALLOWLIST: &[&str] = &[
    "style",
    "onClick",
    "onclick",
    "columnWidths",
    "column_widths",
    "weights",
    "alignItems",
    "align_items",
    "verticalAlign",
    "vertical_align",
    "padding",
    "paddingTop",
    "paddingBottom",
    "paddingLeft",
    "paddingRight",
    "height",
    "h",
    "width",
    "w",
    "backgroundColor",
    "background",
    "borderRadius",
    "opacity",
    "borderWidth",
    "borderColor",
    "minHeight",
    "min_height",
    "maxHeight",
    "max_height",
];

/// Allowed keys for `<ScrollView>` / `scrollable`.
pub(crate) const SCROLL_PROP_ALLOWLIST: &[&str] = &[
    "style",
    "height",
    "h",
    "direction",
    "orient",
    "drawsBackground",
    "draws_background",
    "documentTopInset",
    "document_top_inset",
    "contentInsetTop",
    "content_inset_top",
    "padding",
    "paddingTop",
    "paddingBottom",
    "paddingLeft",
    "paddingRight",
    "appearance",
    "nsAppearance",
    "scrollerGutterRight",
    "scroller_gutter_right",
    "minHeight",
    "min_height",
];

/// Allowed keys for `<VisualEffect>` / `visual_effect`.
pub(crate) const VISUAL_EFFECT_PROP_ALLOWLIST: &[&str] = &[
    "style",
    "material",
    "blendingMode",
    "blending",
    "state",
    "emphasized",
    "isEmphasized",
    "is_emphasized",
    "height",
    "h",
    "appearance",
    "nsAppearance",
    "scrollerGutterRight",
    "scroller_gutter_right",
];
