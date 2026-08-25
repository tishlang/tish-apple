//! `IosHost` — `tishlang_ui::runtime::Host` backed by a single `UIWindow`.

use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};

use block2::RcBlock;
use objc2_foundation::{NSString, NSTimer};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_ui_kit::{UIWindow, UIScreen, UIView, UIViewAutoresizing, UIViewController};
use tishlang_core::Value;
use tishlang_ui::runtime::{Host, RootId};

use super::build::{build_into, BuildCtx};
use super::router::new_router;
use super::text_view_delegate::IosTextViewDelegate;

thread_local! {
    static WINDOW: RefCell<Option<(Retained<UIWindow>, Retained<UIView>)>> = RefCell::new(None);
}

pub struct IosHost {
    window: Retained<UIWindow>,
    root: Retained<UIView>,
    ctx: BuildCtx,
    last_vnode: Value,
}

impl IosHost {
    pub fn new(mtm: MainThreadMarker, root_id: RootId) -> Self {
        let (window, root) = ensure_ios_window(mtm);
        let router = new_router(mtm);
        let text_view_delegate = IosTextViewDelegate::new(mtm);
        let ctx = BuildCtx {
            mtm,
            router,
            text_view_delegate,
            root_id,
        };
        Self {
            window,
            root,
            ctx,
            last_vnode: Value::Null,
        }
    }

    fn content_size(&self) -> (f64, f64) {
        let b = self.root.bounds();
        let mut w = b.size.width as f64;
        let mut h = b.size.height as f64;
        if w < 32.0 {
            w = 390.0;
        }
        if h < 32.0 {
            h = 844.0;
        }
        (w, h)
    }
}

impl Host for IosHost {
    fn commit_root(&mut self, vnode: &Value) {
        let (w, h) = self.content_size();
        let prev = if matches!(self.last_vnode, Value::Null) {
            None
        } else {
            Some(&self.last_vnode)
        };
        build_into(vnode, &self.root, w, h, &self.ctx, prev);
        self.last_vnode = vnode.clone();
        let _ = &self.window;
    }

    fn content_width_changed(&mut self, _width: f64) {
        if matches!(self.last_vnode, Value::Null) {
            return;
        }
        let v = self.last_vnode.clone();
        self.commit_root(&v);
    }
}

/// Root view controller for presenting alerts / sheets (after `ios.run`).
pub fn presenting_view_controller() -> Option<Retained<UIViewController>> {
    WINDOW.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|(window, _)| window.rootViewController())
    })
}

/// NSNotification posted (object: nil) whenever the user shakes the device.
/// App shells (Swift or tish) observe this for Expo-style debug menus — the
/// window subclass below is the ONLY supported shake hook; swizzling
/// `UIWindow.motionEnded` crashes because stock UIWindow never overrides it.
pub const SHAKE_NOTIFICATION: &str = "TishShakeNotification";

define_class!(
    #[unsafe(super(UIWindow))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishShakeWindow"]
    pub struct TishShakeWindow;

    impl TishShakeWindow {
        // motion: UIEventSubtype (motionShake = 1)
        #[unsafe(method(motionEnded:withEvent:))]
        fn motion_ended(&self, motion: isize, event: *mut AnyObject) {
            if motion == 1 {
                unsafe {
                    let name = NSString::from_str(SHAKE_NOTIFICATION);
                    let center: Retained<AnyObject> =
                        msg_send![class!(NSNotificationCenter), defaultCenter];
                    let nil_obj: *mut AnyObject = core::ptr::null_mut();
                    let _: () =
                        msg_send![&*center, postNotificationName: &*name, object: nil_obj];
                }
            }
            let _: () = unsafe { msg_send![super(self), motionEnded: motion, withEvent: event] };
        }
        // NOTE: no canBecomeFirstResponder override. Shake events fall back up the
        // responder chain to the window on their own, and a window that claims
        // first-responder fights WKContentView's keyboard focus — the whole app
        // window stopped compositing (black screen) the moment the iOS keyboard
        // came up while the override was present.
    }
);

/// Create (or reuse) the key `UIWindow` and content `UIView` for the legacy root.
pub fn ensure_ios_window(mtm: MainThreadMarker) -> (Retained<UIWindow>, Retained<UIView>) {
    WINDOW.with(|slot| {
        if let Some(pair) = slot.borrow().as_ref() {
            return (pair.0.clone(), pair.1.clone());
        }

        let bounds = UIScreen::mainScreen(mtm).bounds();
        let window: Retained<UIWindow> = {
            let allocated = TishShakeWindow::alloc(mtm);
            let w: Retained<TishShakeWindow> =
                unsafe { msg_send![allocated, initWithFrame: bounds] };
            Retained::into_super(w)
        };
        let vc = UIViewController::new(mtm);
        let root = UIView::new(mtm);
        root.setFrame(bounds);
        root.setAutoresizingMask(UIViewAutoresizing::FlexibleWidth | UIViewAutoresizing::FlexibleHeight);
        vc.setView(Some(&root));
        window.setRootViewController(Some(&vc));
        window.makeKeyAndVisible();

        let pair = (window, root);
        *slot.borrow_mut() = Some((pair.0.clone(), pair.1.clone()));
        pair
    })
}

/// Pump `setTimeout` / `setInterval` on the main run loop so async work can yield between UI updates.
pub fn install_timer_drain_pump() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
        tishlang_runtime::drain_timers();
    });
    let _timer = unsafe {
        NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.032, true, &*block)
    };
    drop(_timer);
}
