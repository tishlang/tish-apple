//! `GroupedTable`: grouped list with optional vibrancy section headers in a flipped scroll surface.
//!
//! JSX: `<GroupedTable><section title="…">…rows…</section></GroupedTable>`.
//! Standalone `<section>` is laid out like a [`Column`](super::build::commit_vnode) elsewhere.
//!
//! Implementation matches [`ScrollView`](super::build::commit_vnode) (`FlippedClipView` +
//! `FlippedDocumentView`). Section headers scroll with content; they are not AppKit floating group
//! rows (`NSTableView` + `floatsGroupRows` was unreliable inside Tish’s flipped split/window layout).

#![allow(non_snake_case)]

use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSColor, NSControlSize, NSFont, NSFontWeightSemibold, NSScroller, NSScrollerStyle, NSScrollView,
    NSTextField, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState,
};
use objc2_core_foundation::{CGPoint, CGSize};
use objc2_foundation::{NSEdgeInsets, NSRect, NSString};
use tishlang_core::{ObjectMap, PropMap};
use tishlang_core::Value;

use super::build::{
    collect_element_vnodes, commit_vnode, freeze_autoresizing_for_manual_frames,
    strip_scroll_content_insets, tune_scroll_view_chrome, vnode_children, vnode_props, BuildCtx,
};
use super::flipped::{FlippedClipView, FlippedDocumentView, FlippedVisualEffectView};
use super::scroll_chrome_embed::mark_grouped_table_scroll;
use super::style::apply_static_label_text_field;
use super::canonical_host_tag;

/// Width reserved for a **legacy** vertical scroller (`NSScrollerStyle::Legacy`) so document rows
/// match the clip view and do not sit under the knob.
fn legacy_vertical_scroller_lane(mtm: MainThreadMarker) -> f64 {
    NSScroller::scrollerWidthForControlSize_scrollerStyle(
        NSControlSize::Regular,
        NSScrollerStyle::Legacy,
        mtm,
    ) as f64
}

/// One logical row: vibrancy group header or a Tish-built content view.
pub(super) enum GroupedTableRow {
    Group {
        view: Retained<NSView>,
        height: f64,
    },
    Item {
        view: Retained<NSView>,
        height: f64,
    },
}

const ZERO_EDGE_INSETS: NSEdgeInsets = NSEdgeInsets {
    top: 0.0,
    left: 0.0,
    bottom: 0.0,
    right: 0.0,
};

/// Section title row without `NSVisualEffectView` (lets scrolling list show through under overlays).
fn group_header_plain_view(mtm: MainThreadMarker, title: &str, width: f64) -> Retained<NSView> {
    let h = 28.0_f64;
    let holder = FlippedDocumentView::new(
        mtm,
        NSRect::new(CGPoint::ZERO, CGSize::new(width.max(1.0), h)),
    );
    let holder_ns: &NSView = unsafe { &*std::ptr::from_ref(&*holder).cast::<NSView>() };
    let tf = NSTextField::labelWithString(&NSString::from_str(title), mtm);
    apply_static_label_text_field(&tf, &PropMap::default(), mtm);
    let font = NSFont::systemFontOfSize_weight(11.0, unsafe { NSFontWeightSemibold });
    tf.setFont(Some(&font));
    tf.setTextColor(Some(&NSColor::secondaryLabelColor()));
    tf.setFrame(NSRect::new(
        CGPoint::new(12.0, 4.0),
        CGSize::new((width - 24.0).max(1.0), 20.0),
    ));
    holder_ns.addSubview(&tf);
    unsafe { Retained::cast_unchecked(holder) }
}

fn group_header_material_view(mtm: MainThreadMarker, title: &str, width: f64) -> Retained<NSView> {
    let h = 28.0_f64;
    let fx = FlippedVisualEffectView::new(
        mtm,
        NSRect::new(CGPoint::ZERO, CGSize::new(width.max(1.0), h)),
    );
    fx.setMaterial(NSVisualEffectMaterial::UnderWindowBackground);
    fx.setBlendingMode(NSVisualEffectBlendingMode::WithinWindow);
    fx.setState(NSVisualEffectState::FollowsWindowActiveState);
    fx.setFrame(NSRect::new(CGPoint::ZERO, CGSize::new(width.max(1.0), h)));
    let tf = NSTextField::labelWithString(&NSString::from_str(title), mtm);
    apply_static_label_text_field(&tf, &PropMap::default(), mtm);
    let font = NSFont::systemFontOfSize_weight(11.0, unsafe { NSFontWeightSemibold });
    tf.setFont(Some(&font));
    tf.setTextColor(Some(&NSColor::labelColor()));
    tf.setFrame(NSRect::new(
        CGPoint::new(12.0, 4.0),
        CGSize::new((width - 24.0).max(1.0), 20.0),
    ));
    let fx_ns: &NSView = unsafe { &*std::ptr::from_ref(&*fx).cast::<NSView>() };
    fx_ns.addSubview(&tf);
    let v: Retained<NSView> = unsafe { Retained::cast_unchecked(fx) };
    v
}

/// Build flat row list from `<section title>` groups and their element children.
pub(super) fn build_grouped_table_rows(
    grouped_children: &[Value],
    content_w: f64,
    ctx: &BuildCtx,
    vibrant_section_headers: bool,
) -> Vec<GroupedTableRow> {
    let mut sections: Vec<Value> = Vec::new();
    collect_element_vnodes(grouped_children, &mut sections);
    let mut rows = Vec::new();
    for sec in sections {
        let Value::Object(oref) = sec else {
            continue;
        };
        let sm = &oref.borrow().strings;
        let tag = sm
            .get("tag")
            .and_then(|t| match t {
                Value::String(s) => Some(s.as_str().to_string()),
                _ => None,
            })
            .unwrap_or_default();
        if canonical_host_tag(tag.as_str()) != "section" {
            continue;
        }
        let raw = vnode_props(sm);
        let title = raw
            .get("title")
            .or_else(|| raw.get("label"))
            .map(|v| v.to_display_string())
            .unwrap_or_default();
        let gh = if vibrant_section_headers {
            group_header_material_view(ctx.mtm, &title, content_w)
        } else {
            group_header_plain_view(ctx.mtm, &title, content_w)
        };
        rows.push(GroupedTableRow::Group {
            view: gh,
            height: 28.0,
        });
        let mut row_vnodes = Vec::new();
        collect_element_vnodes(&vnode_children(sm), &mut row_vnodes);
        for rv in row_vnodes {
            let holder = FlippedDocumentView::new(
                ctx.mtm,
                NSRect::new(CGPoint::ZERO, CGSize::new(content_w.max(1.0), 1.0)),
            );
            let h = commit_vnode(&rv, &*holder, 0.0, 0.0, content_w, None, ctx);
            let h = h.max(44.0);
            holder.setFrameSize(CGSize::new(content_w.max(1.0), h));
            let v: Retained<NSView> = unsafe { Retained::cast_unchecked(holder) };
            rows.push(GroupedTableRow::Item { view: v, height: h });
        }
    }
    rows
}

fn grouped_rows_total_height(rows: &[GroupedTableRow]) -> f64 {
    rows
        .iter()
        .map(|r| match r {
            GroupedTableRow::Group { height, .. } | GroupedTableRow::Item { height, .. } => *height,
        })
        .sum()
}

/// `NSScrollView` + flipped document: same stack as `ScrollView`, with section chrome + rows.
///
/// `document_top_inset` reserves space at the top of the document (e.g. under a `ZStack` overlay
/// header) so the first row is not hidden; that inset scrolls away with the content.
pub(super) fn install_grouped_table_scroll(
    mtm: MainThreadMarker,
    content_w: f64,
    _scroll_h: f64,
    grouped_children: &[Value],
    ctx: &BuildCtx,
    document_top_inset: f64,
    vibrant_section_headers: bool,
) -> Retained<NSScrollView> {
    let inset = document_top_inset.max(0.0);
    let scroll = NSScrollView::new(mtm);
    scroll.setDrawsBackground(false);
    scroll.setHasVerticalScroller(true);
    scroll.setHasHorizontalScroller(false);
    tune_scroll_view_chrome(&scroll, true, false);
    // Reserve a real trailing lane for the vertical scroller so list rows shrink with the pane
    // instead of living under an overlay scroller.
    scroll.setScrollerStyle(NSScrollerStyle::Legacy);
    strip_scroll_content_insets(&scroll);

    let lane = legacy_vertical_scroller_lane(mtm);
    let cw = (content_w - lane).max(1.0);
    let rows_vec = build_grouped_table_rows(
        grouped_children,
        cw,
        ctx,
        vibrant_section_headers,
    );
    let doc_h = (grouped_rows_total_height(&rows_vec) + inset).max(1.0);

    let clip_frame = scroll.contentView().frame();
    let flipped_clip = FlippedClipView::new(mtm, clip_frame);
    flipped_clip.setAutomaticallyAdjustsContentInsets(false);
    flipped_clip.setContentInsets(ZERO_EDGE_INSETS);
    scroll.setContentView(&flipped_clip);
    strip_scroll_content_insets(&scroll);

    let doc = FlippedDocumentView::new(
        mtm,
        NSRect::new(CGPoint::ZERO, CGSize::new(cw, 0.0)),
    );
    let mut dy = inset;
    for row in rows_vec {
        let (view, h) = match row {
            GroupedTableRow::Group { view, height } => (view, height),
            GroupedTableRow::Item { view, height } => (view, height),
        };
        view.setFrame(NSRect::new(
            CGPoint::new(0.0, dy),
            CGSize::new(cw, h),
        ));
        freeze_autoresizing_for_manual_frames(&*view);
        doc.addSubview(&*view);
        dy += h;
    }
    doc.setFrameSize(CGSize::new(cw, doc_h));
    scroll.setDocumentView(Some(&*doc));
    mark_grouped_table_scroll(&scroll);
    scroll
}
