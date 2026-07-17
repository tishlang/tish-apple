//! `NSWindowDelegate`: teardown on close + optional Tish callbacks from `<Window>` / `<SidebarWindow>` props.
//!
//! - **`onOpen`**: invoked from [`super::MacosHost::after_window_shown`] / sidebar equivalent (after the window is ordered on-screen).
//! - **`onClose`**: invoked in `windowWillClose:` before unregistering Tish hooks/handlers.
//!   `detach_native_actions` runs **synchronously** here so we never walk the content view tree after
//!   AppKit has started tearing it down (that produced `objc_release` crashes in a deferred block).
//!   Only `drop_host_for_root` is **deferred** so releasing our `Retained<NSWindow>` does not race
//!   `_NSWindowTransformAnimation` (see `NS_WINDOW_CLOSE_TEARDOWN_DELAY_NS`).
//! - **`onMinimize`**: `windowDidMiniaturize:`.
//! - **`onMaximize`**: when `isZoomed` transitions to **`true`** (green zoom button / maximize).

#![allow(non_snake_case)] // Objective-C selector names

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use dispatch2::{DispatchQueue, DispatchTime};
use objc2_app_kit::{NSWindow, NSWindowDelegate};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol};
use tishlang_core::{ObjectMap, PropMap, Value};
use tishlang_ui::runtime::{
    drop_host_for_root, unregister_root_hooks_and_effects, with_host_for_root, RootId,
};

use super::handlers::unregister_root_window;

/// Delay after `windowWillClose:` before `drop_host_for_root` only. Dropping our last strong
/// `NSWindow` ref too early can race `_NSWindowTransformAnimation`; avoid synchronous
/// `setContentView(nil)` / similar — those are not part of `detach_native_actions`.
const NS_WINDOW_CLOSE_TEARDOWN_DELAY_NS: i64 = 400_000_000;

pub struct TishWindowDelegateIvars {
    root_id: Cell<RootId>,
    on_open: RefCell<Option<Value>>,
    on_close: RefCell<Option<Value>>,
    on_minimize: RefCell<Option<Value>>,
    on_maximize: RefCell<Option<Value>>,
    last_zoomed: Cell<bool>,
}

fn prop_fn(props: &PropMap, keys: &[&str]) -> Option<Value> {
    for k in keys {
        if let Some(v) = props.get(*k) {
            if matches!(v, Value::Function(_)) {
                return Some(v.clone());
            }
        }
    }
    None
}

fn invoke_cb(cell: &RefCell<Option<Value>>) {
    let f_opt = cell.borrow().clone();
    let Some(Value::Function(f)) = f_opt else {
        return;
    };
    let _ = f.call(&[]);
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishWindowDelegate"]
    #[ivars = TishWindowDelegateIvars]
    pub struct TishWindowDelegate;

    unsafe impl NSObjectProtocol for TishWindowDelegate {}

    unsafe impl NSWindowDelegate for TishWindowDelegate {
        #[unsafe(method(windowWillClose:))]
        fn windowWillClose(&self, _notification: &NSNotification) {
            let rid = self.ivars().root_id.get();
            invoke_cb(&self.ivars().on_close);
            // Rust hook state + handler maps: safe immediately (no view hierarchy mutation).
            unregister_root_hooks_and_effects(rid);
            unregister_root_window(rid);
            // Detach now while the view hierarchy is still valid; defer only dropping the host /
            // `Retained<NSWindow>` so close animation does not race teardown.
            let _ = with_host_for_root(rid, |host| host.detach_native_actions());
            let when = DispatchTime::NOW.time(NS_WINDOW_CLOSE_TEARDOWN_DELAY_NS);
            let _ = DispatchQueue::main().after(when, move || {
                drop_host_for_root(rid);
            });
        }

        #[unsafe(method(windowDidMiniaturize:))]
        fn windowDidMiniaturize(&self, _notification: &NSNotification) {
            invoke_cb(&self.ivars().on_minimize);
        }

        #[unsafe(method(windowDidResize:))]
        fn windowDidResize(&self, notification: &NSNotification) {
            let Some(obj) = notification.object() else {
                return;
            };
            let Some(win) = obj.downcast_ref::<NSWindow>() else {
                return;
            };
            let now = win.isZoomed();
            let prev = self.ivars().last_zoomed.replace(now);
            if now && !prev {
                invoke_cb(&self.ivars().on_maximize);
            }
        }
    }
);

impl TishWindowDelegate {
    pub fn new(mtm: MainThreadMarker, root_id: RootId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TishWindowDelegateIvars {
            root_id: Cell::new(root_id),
            on_open: RefCell::new(None),
            on_close: RefCell::new(None),
            on_minimize: RefCell::new(None),
            on_maximize: RefCell::new(None),
            last_zoomed: Cell::new(false),
        });
        unsafe { msg_send![super(this), init] }
    }

    /// Call after `props` change (each root commit). Resets zoom tracking from the live window.
    pub fn sync_from_props(&self, props: &PropMap, window: &NSWindow) {
        *self.ivars().on_open.borrow_mut() = prop_fn(props, &["onOpen", "on_open"]);
        *self.ivars().on_close.borrow_mut() = prop_fn(props, &["onClose", "on_close"]);
        *self.ivars().on_minimize.borrow_mut() = prop_fn(props, &["onMinimize", "on_minimize"]);
        *self.ivars().on_maximize.borrow_mut() = prop_fn(props, &["onMaximize", "on_maximize"]);
        self.ivars().last_zoomed.set(window.isZoomed());
    }

    /// After the window is ordered on-screen (`macos.show` / `openWindow` handle).
    pub fn fire_on_open(&self) {
        invoke_cb(&self.ivars().on_open);
    }
}
