//! When a `ZStack` pairs **`GroupedTable`** (`NSScrollView`) with a **`VisualEffect`** header, reparent
//! the header **into** the scroll view **above** the clip view but **below** the vertical scroller.
//! Then `NSVisualEffectView` blurs scrolling list rows; the overlay scroller stays on top.

use objc2::rc::Retained;
use objc2::ClassType;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSControlSize, NSUserInterfaceItemIdentification, NSView,
    NSWindowOrderingMode, NSScroller, NSScrollerStyle, NSScrollView,
};
use objc2_core_foundation::{CGFloat, CGPoint, CGSize};
use objc2_foundation::{NSObjectProtocol, NSRect, NSString};

use super::flipped::FlippedVisualEffectView;

pub const GROUPED_TABLE_SCROLL_IDENTIFIER: &str = "tish.groupedTableScroll";

pub fn mark_grouped_table_scroll(scroll: &NSScrollView) {
    let v: &NSView = unsafe { &*std::ptr::from_ref(scroll).cast::<NSView>() };
    NSUserInterfaceItemIdentification::setIdentifier(
        v,
        Some(&NSString::from_str(GROUPED_TABLE_SCROLL_IDENTIFIER)),
    );
}

unsafe fn view_is_marked_grouped_table_scroll(v: &NSView) -> bool {
    if !v.isKindOfClass(NSScrollView::class()) {
        return false;
    }
    let Some(id) = NSUserInterfaceItemIdentification::identifier(v) else {
        return false;
    };
    id.to_string() == GROUPED_TABLE_SCROLL_IDENTIFIER
}

fn is_flipped_visual_effect(v: &NSView) -> bool {
    v.isKindOfClass(FlippedVisualEffectView::class())
}

/// Trailing width used by the vertical scroller (legacy lane) so the header can match the clip.
fn vertical_scroller_lane_width(scroll: &NSScrollView) -> f64 {
    scroll.layoutSubtreeIfNeeded();
    if let Some(vs) = scroll.verticalScroller() {
        let w = vs.frame().size.width as f64;
        if w > 0.5 {
            return w;
        }
    }
    let mtm = MainThreadMarker::new().expect("scroll chrome on main thread");
    NSScroller::scrollerWidthForControlSize_scrollerStyle(
        NSControlSize::Regular,
        NSScrollerStyle::Legacy,
        mtm,
    ) as f64
}

/// Pin the embedded header to the top; width is **scroll bounds minus scroller lane** so the bar
/// does not sit under the knob (`FlippedVisualEffectView` uses the same gutter for layout).
pub fn reposition_embedded_header_if_any(scroll: &NSScrollView) {
    let subs = scroll.subviews();
    for i in 0..subs.count() {
        let v = subs.objectAtIndex(i);
        if is_flipped_visual_effect(&*v) {
            reposition_embedded_header_in_scroll(scroll, &*v);
            return;
        }
    }
}

fn reposition_embedded_header_in_scroll(scroll: &NSScrollView, header: &NSView) {
    let lane = vertical_scroller_lane_width(scroll);
    if header.isKindOfClass(FlippedVisualEffectView::class()) {
        let fv: &FlippedVisualEffectView = unsafe { &*std::ptr::from_ref(header).cast() };
        fv.set_right_gutter(lane);
    }
    let sb = scroll.bounds();
    let sw = sb.size.width.max(0.0) as f64;
    let sh = sb.size.height.max(0.0) as f64;
    let hh = header.frame().size.height.max(1.0) as f64;
    let w = (sw - lane).max(1.0);
    let y = if scroll.isFlipped() {
        0.0
    } else {
        (sh - hh).max(0.0)
    };
    header.setFrame(NSRect::new(
        CGPoint::new(0.0, y as CGFloat),
        CGSize::new(w as CGFloat, hh as CGFloat),
    ));
    header.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMinYMargin,
    );
}

fn classify_scroll_header_pair<'a>(
    va: &'a NSView,
    vb: &'a NSView,
) -> Option<(&'a NSScrollView, &'a NSView)> {
    unsafe {
        if view_is_marked_grouped_table_scroll(va) && is_flipped_visual_effect(vb) {
            return Some((&*(std::ptr::from_ref(va).cast::<NSScrollView>()), vb));
        }
        if view_is_marked_grouped_table_scroll(vb) && is_flipped_visual_effect(va) {
            return Some((&*(std::ptr::from_ref(vb).cast::<NSScrollView>()), va));
        }
    }
    None
}

/// If `shell` is a two-child `ZStack` with a marked `GroupedTable` scroll and a `FlippedVisualEffectView`,
/// move the effect into the scroll view (above the clip, below the scroller).
pub fn zstack_try_embed_grouped_table_header(shell: &NSView) {
    let subs = shell.subviews();
    if subs.count() != 2 {
        return;
    }
    let a = subs.objectAtIndex(0);
    let b = subs.objectAtIndex(1);
    let Some((scroll, header)) = classify_scroll_header_pair(&*a, &*b) else {
        return;
    };
    let scroll_ns: *const NSView = (scroll as *const NSScrollView).cast();
    if let Some(sup) = unsafe { header.superview() } {
        let sup_ptr = Retained::as_ptr(&sup).cast::<NSView>();
        if sup_ptr == scroll_ns {
            reposition_embedded_header_in_scroll(scroll, header);
            return;
        }
    }

    header.removeFromSuperview();
    let clip = scroll.contentView();
    scroll.addSubview_positioned_relativeTo(
        header,
        NSWindowOrderingMode::Above,
        Some(&*clip),
    );
    reposition_embedded_header_in_scroll(scroll, header);
    scroll.layoutSubtreeIfNeeded();
}
