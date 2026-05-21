//! `IosHost` — `tishlang_ui::runtime::Host` backed by a single `UIWindow`.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_ui_kit::{UIWindow, UIScreen, UIView, UIViewAutoresizing, UIViewController};
use tishlang_core::Value;
use tishlang_ui::runtime::{Host, RootId};

use super::build::{build_into, BuildCtx};
use super::router::new_router;

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
        let ctx = BuildCtx {
            mtm,
            router,
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
        build_into(vnode, &self.root, w, h, &self.ctx);
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

/// Create (or reuse) the key `UIWindow` and content `UIView` for the legacy root.
pub fn ensure_ios_window(mtm: MainThreadMarker) -> (Retained<UIWindow>, Retained<UIView>) {
    WINDOW.with(|slot| {
        if let Some(pair) = slot.borrow().as_ref() {
            return (pair.0.clone(), pair.1.clone());
        }

        let bounds = UIScreen::mainScreen(mtm).bounds();
        let window = {
            let allocated = UIWindow::alloc(mtm);
            UIWindow::initWithFrame(allocated, bounds)
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
