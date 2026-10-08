//! `hoverBackgroundColor` on clickable rows: the row's transparent click overlay tracks the
//! pointer and repaints the row's layer while it is inside. Only one row is hovered at a time.

use std::cell::{Cell, RefCell};

use objc2::rc::{Retained, Weak};
use objc2::runtime::AnyObject;
use objc2::{
    define_class, msg_send, AnyThread, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSButton, NSColor, NSControl, NSEvent, NSResponder, NSTrackingArea, NSTrackingAreaOptions,
    NSView,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSRect};
use tishlang_apple_common::style::props_string;
use tishlang_core::PropMap;

use super::style::resolve_ns_color;

pub struct HoverButtonIvars {
    hover: RefCell<Option<Retained<NSColor>>>,
    base: RefCell<Option<Retained<NSColor>>>,
    inside: Cell<bool>,
}

thread_local! {
    static CURRENT: RefCell<Option<Weak<HoverButton>>> = const { RefCell::new(None) };
}

define_class!(
    #[unsafe(super(NSButton, NSControl, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishHoverButton"]
    #[ivars = HoverButtonIvars]
    pub struct HoverButton;

    unsafe impl NSObjectProtocol for HoverButton {}

    impl HoverButton {
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            for area in self.trackingAreas().iter() {
                self.removeTrackingArea(&area);
            }
            // ActiveAlways: the launcher panel takes keys without its app being active.
            let options = NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::ActiveAlways
                | NSTrackingAreaOptions::InVisibleRect;
            let owner: &AnyObject = self.as_ref();
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    NSRect::ZERO,
                    options,
                    Some(owner),
                    None,
                )
            };
            self.addTrackingArea(&area);
            unsafe {
                let _: () = msg_send![super(self), updateTrackingAreas];
            }
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            if self.ivars().hover.borrow().is_none() {
                return;
            }
            let previous = CURRENT.with(|c| c.borrow_mut().replace(Weak::from(self)));
            if let Some(p) = previous.and_then(|w| w.load()) {
                if !std::ptr::eq(&*p, self) {
                    p.set_inside(false);
                }
            }
            self.set_inside(true);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.set_inside(false);
        }

        /// `[TishHoverButton clearHover]`: un-hover whatever row is hovered (e.g. when the window
        /// hides, which sends no mouseExited).
        #[unsafe(method(clearHover))]
        fn clear_hover() {
            clear_hover();
        }
    }
);

impl HoverButton {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(HoverButtonIvars {
            hover: RefCell::new(None),
            base: RefCell::new(None),
            inside: Cell::new(false),
        });
        unsafe { msg_send![super(this), init] }
    }

    /// The row's colours after a build or patch: `base` is what its `backgroundColor` painted.
    pub fn set_colors(&self, hover: Option<Retained<NSColor>>, base: Option<Retained<NSColor>>) {
        let had = self.ivars().hover.borrow().is_some();
        *self.ivars().hover.borrow_mut() = hover;
        *self.ivars().base.borrow_mut() = base;
        if had || self.ivars().hover.borrow().is_some() {
            self.paint();
        }
    }

    fn set_inside(&self, inside: bool) {
        if self.ivars().inside.replace(inside) != inside {
            self.paint();
        }
    }

    fn paint(&self) {
        let Some(row) = (unsafe { self.superview() }) else {
            return;
        };
        let inside = self.ivars().inside.get();
        let hover = self.ivars().hover.borrow().clone();
        let color = match (inside, hover) {
            (true, Some(h)) => Some(h),
            _ => self.ivars().base.borrow().clone(),
        };
        row.setWantsLayer(true);
        if let Some(layer) = row.layer() {
            let clear = NSColor::clearColor();
            let c = color.unwrap_or(clear);
            layer.setBackgroundColor(Some(&*c.CGColor()));
        }
    }
}

fn prop_color(props: &PropMap, keys: &[&str]) -> Option<Retained<NSColor>> {
    let s = props_string(props, keys)?;
    let t = s.trim().to_ascii_lowercase();
    if t == "transparent" || t == "none" || t == "clear" {
        return None;
    }
    resolve_ns_color(&s)
}

/// After a row's click overlay is built or patched: hand it the row's `hoverBackgroundColor` and
/// `backgroundColor`.
pub(super) fn sync_hover(btn: &NSButton, props: &PropMap) {
    if !btn.isKindOfClass(HoverButton::class()) {
        return;
    }
    let hb: &HoverButton = unsafe { &*(std::ptr::from_ref(btn).cast::<HoverButton>()) };
    hb.set_colors(
        prop_color(props, &["hoverBackgroundColor"]),
        prop_color(props, &["backgroundColor", "background"]),
    );
}

pub fn clear_hover() {
    if let Some(p) = CURRENT
        .with(|c| c.borrow_mut().take())
        .and_then(|w| w.load())
    {
        p.set_inside(false);
    }
}
