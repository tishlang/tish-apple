//! Flipped geometry for JSX-style top-left layout.
//! [`FlippedRootView`] is the window content view (relayout on resize); inner panes use
//! [`FlippedDocumentView`] so nested layouts do not trigger host relayout notifications.

use std::cell::Cell;

use objc2::rc::Retained;
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSClipView, NSSplitView, NSView, NSVisualEffectView};
use objc2_core_foundation::{CGPoint, CGSize};
use objc2_foundation::{NSObjectProtocol, NSRect};
use tishlang_ui::runtime::RootId;

use super::{notify_root_layout_changed, notify_split_pane_layout_changed};

/// `layout` skips [`notify_root_layout_changed`] once the host is tearing down (`0` is never installed).
pub const DISCONNECTED_ROOT_ID: RootId = 0;

/// Per-view Tish root id (layout notifications route to the correct host).
pub struct FlippedRootViewIvars {
    pub root_id: Cell<RootId>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishFlippedRootView"]
    #[ivars = FlippedRootViewIvars]
    pub struct FlippedRootView;

    unsafe impl NSObjectProtocol for FlippedRootView {}

    impl FlippedRootView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(layout))]
        fn layout(&self) {
            unsafe {
                let _: () = msg_send![super(self), layout];
            }
            let rid = self.ivars().root_id.get();
            if rid == DISCONNECTED_ROOT_ID {
                return;
            }
            let s = self.bounds().size;
            notify_root_layout_changed(rid, s.width as f64, s.height as f64);
        }
    }
);

pub struct FlippedSplitPaneRootViewIvars {
    pub root_id: Cell<RootId>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishFlippedSplitPaneRootView"]
    #[ivars = FlippedSplitPaneRootViewIvars]
    pub struct FlippedSplitPaneRootView;

    unsafe impl NSObjectProtocol for FlippedSplitPaneRootView {}

    impl FlippedSplitPaneRootView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(layout))]
        fn layout(&self) {
            unsafe {
                let _: () = msg_send![super(self), layout];
            }
            let rid = self.ivars().root_id.get();
            if rid == DISCONNECTED_ROOT_ID {
                return;
            }
            let _ = self.bounds().size;
            notify_split_pane_layout_changed(rid);
        }
    }
);

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishFlippedDocumentView"]
    pub struct FlippedDocumentView;

    unsafe impl NSObjectProtocol for FlippedDocumentView {}

    impl FlippedDocumentView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

/// Right-edge gutter (pts) reserved for a sibling `NSScrollView` scroller so `ViewWidthSizable`
/// does not grow the effect over the knob after layout.
pub struct FlippedVisualEffectViewIvars {
    pub right_gutter: Cell<f64>,
}

// Flipped `NSVisualEffectView`: matches split-pane `FlippedDocumentView` coords; an unflipped effect
// under a flipped superview mis-anchors the inner flipped document (large empty band above header).
define_class!(
    #[unsafe(super(NSVisualEffectView))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishFlippedVisualEffectView"]
    #[ivars = FlippedVisualEffectViewIvars]
    pub struct FlippedVisualEffectView;

    unsafe impl NSObjectProtocol for FlippedVisualEffectView {}

    impl FlippedVisualEffectView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(layout))]
        fn layout(&self) {
            unsafe {
                let _: () = msg_send![super(self), layout];
            }
            let g = self.ivars().right_gutter.get();
            if g <= 0.0 {
                return;
            }
            let Some(sup) = (unsafe { self.superview() }) else {
                return;
            };
            let sw = sup.bounds().size.width.max(0.0);
            let nw = (sw - g).max(1.0);
            let mut f = self.frame();
            if (f.size.width - nw).abs() > 0.25 {
                f.size.width = nw;
                f.origin.x = 0.0;
                self.setFrame(f);
            }
            let subs = self.subviews();
            if subs.count() < 1 {
                return;
            }
            let doc = subs.objectAtIndex(0);
            let dh = doc.bounds().size.height.max(1.0);
            let y = 0.0;
            doc.setFrame(NSRect::new(
                CGPoint::new(0.0, y),
                CGSize::new(nw, dh),
            ));
        }
    }
);

// Flipped clip view: default NSClipView is unflipped, which breaks scroll origin for flipped docs.
define_class!(
    #[unsafe(super(NSClipView))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishFlippedClipView"]
    pub struct FlippedClipView;

    unsafe impl NSObjectProtocol for FlippedClipView {}

    impl FlippedClipView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

/// After `adjustSubviews` / `layout`, pin flipped pane roots to the top and full split height.
/// Even with a flipped `NSSplitView`, AppKit can leave a tall empty band above flipped children.
pub(super) fn snap_flipped_split_panes_full_height(split: &NSSplitView) {
    let vertical = split.isVertical();
    if !vertical {
        return;
    }
    let b = split.bounds();
    let h = b.size.height.max(0.0);
    if h < 1.0 {
        return;
    }
    let subs = split.subviews();
    for i in 0..subs.count() {
        let v = subs.objectAtIndex(i);
        if !v.isFlipped() {
            continue;
        }
        let f = v.frame();
        let x = f.origin.x;
        let w = f.size.width.max(0.0);
        v.setFrame(NSRect::new(
            CGPoint::new(x, 0.0),
            CGSize::new(w, h),
        ));
    }
}

// Flipped `NSSplitView`: default split is unflipped while Tish panes are `FlippedDocumentView`,
// so AppKit bottom-anchors panes and leaves a large empty band at the top (notes column chrome).
define_class!(
    #[unsafe(super(NSSplitView))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishFlippedSplitView"]
    pub struct FlippedSplitView;

    unsafe impl NSObjectProtocol for FlippedSplitView {}

    impl FlippedSplitView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(layout))]
        fn layout(&self) {
            unsafe {
                let _: () = msg_send![super(self), layout];
            }
            let sv: &NSSplitView = unsafe { &*std::ptr::from_ref(self).cast() };
            snap_flipped_split_panes_full_height(sv);
        }
    }
);

impl FlippedRootView {
    /// Stop routing `layout` into Tish (window close / host teardown).
    pub fn disconnect_tish_root_routing(&self) {
        self.ivars().root_id.set(DISCONNECTED_ROOT_ID);
    }

    pub fn new(mtm: MainThreadMarker, frame: NSRect, root_id: RootId) -> Retained<Self> {
        let ivars = FlippedRootViewIvars {
            root_id: Cell::new(root_id),
        };
        let this = Self::alloc(mtm).set_ivars(ivars);
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

impl FlippedSplitPaneRootView {
    pub fn disconnect_tish_root_routing(&self) {
        self.ivars().root_id.set(DISCONNECTED_ROOT_ID);
    }

    pub fn new(mtm: MainThreadMarker, frame: NSRect, root_id: RootId) -> Retained<Self> {
        let ivars = FlippedSplitPaneRootViewIvars {
            root_id: Cell::new(root_id),
        };
        let this = Self::alloc(mtm).set_ivars(ivars);
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

impl FlippedDocumentView {
    pub fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        unsafe { msg_send![super(mtm.alloc().set_ivars(())), initWithFrame: frame] }
    }
}

impl FlippedClipView {
    pub fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        unsafe { msg_send![super(mtm.alloc().set_ivars(())), initWithFrame: frame] }
    }
}

impl FlippedVisualEffectView {
    pub fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let ivars = FlippedVisualEffectViewIvars {
            right_gutter: Cell::new(0.0),
        };
        let this = Self::alloc(mtm).set_ivars(ivars);
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    /// When > 0, [`layout`](FlippedVisualEffectView::layout) keeps the frame width at
    /// `superview.bounds.width - gutter` so a `ZStack` list scroller stays visible.
    pub fn set_right_gutter(&self, gutter: f64) {
        let g = gutter.max(0.0);
        self.ivars().right_gutter.set(g);
        // `NSVisualEffectView` can composite slightly outside its frame; the header sits as a
        // `ZStack` sibling *above* the full-width `NSScrollView`, so blur would otherwise cover the
        // vertical scroller in the gutter column.
        let v: &NSView = unsafe { &*std::ptr::from_ref(self).cast::<NSView>() };
        v.setClipsToBounds(g > 0.0);
        self.setNeedsLayout(true);
    }
}

impl FlippedSplitView {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let frame = NSRect::new(CGPoint::ZERO, CGSize::ZERO);
        unsafe { msg_send![super(mtm.alloc().set_ivars(())), initWithFrame: frame] }
    }
}
