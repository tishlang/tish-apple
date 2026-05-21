//! AppKit target/action router(s). Clone handlers before invoking Tish (avoid RefCell re-entrancy).

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSButton, NSControlStateValueOn, NSPopUpButton, NSSlider, NSSwitch, NSToolbarItem,
};
use objc2_foundation::{NSObject, NSObjectProtocol};
use tishlang_ui::runtime::run_with_current_root;

use super::handlers::{
    decode_control_tag, decode_toolbar_tag, invoke_toolbar_action, BOOL_HANDLERS, CLICK_HANDLERS,
    F64_HANDLERS, PICK_HANDLERS,
};

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishMacosControlRouter"]
    pub struct MacosControlRouter;

    unsafe impl NSObjectProtocol for MacosControlRouter {}

    impl MacosControlRouter {
        #[unsafe(method(jsxClick:))]
        fn jsx_click(&self, sender: Option<&AnyObject>) {
            let Some(s) = sender else { return };
            let Some(btn) = s.downcast_ref::<NSButton>() else {
                return;
            };
            if decode_toolbar_tag(btn.tag()).is_some() {
                return;
            }
            let (root_id, idx) = decode_control_tag(btn.tag());
            let handler = CLICK_HANDLERS.with(|c| {
                c.borrow()
                    .get(&root_id)
                    .and_then(|v| v.get(idx))
                    .and_then(|slot| slot.as_ref().map(|h| h.clone()))
            });
            if let Some(f) = handler {
                run_with_current_root(root_id, || f());
            }
        }

        #[unsafe(method(tishBool:))]
        fn tish_bool(&self, sender: Option<&AnyObject>) {
            let Some(s) = sender else { return };
            if let Some(sw) = s.downcast_ref::<NSSwitch>() {
                let (root_id, idx) = decode_control_tag(sw.tag());
                let on = sw.state() == NSControlStateValueOn;
                let handler = BOOL_HANDLERS.with(|c| {
                    c.borrow()
                        .get(&root_id)
                        .and_then(|v| v.get(idx))
                        .and_then(|slot| slot.as_ref().map(|h| h.clone()))
                });
                if let Some(f) = handler {
                    run_with_current_root(root_id, || f(on));
                }
                return;
            }
            let Some(btn) = s.downcast_ref::<NSButton>() else {
                return;
            };
            let (root_id, idx) = decode_control_tag(btn.tag());
            let on = btn.state() == NSControlStateValueOn;
            let handler = BOOL_HANDLERS.with(|c| {
                c.borrow()
                    .get(&root_id)
                    .and_then(|v| v.get(idx))
                    .and_then(|slot| slot.as_ref().map(|h| h.clone()))
            });
            if let Some(f) = handler {
                run_with_current_root(root_id, || f(on));
            }
        }

        #[unsafe(method(tishSlider:))]
        fn tish_slider(&self, sender: Option<&AnyObject>) {
            let Some(s) = sender else { return };
            let Some(sl) = s.downcast_ref::<NSSlider>() else {
                return;
            };
            let (root_id, idx) = decode_control_tag(sl.tag());
            let v = sl.doubleValue();
            let handler = F64_HANDLERS.with(|c| {
                c.borrow()
                    .get(&root_id)
                    .and_then(|vec| vec.get(idx))
                    .and_then(|slot| slot.as_ref().map(|h| h.clone()))
            });
            if let Some(f) = handler {
                run_with_current_root(root_id, || f(v));
            }
        }

        #[unsafe(method(tishPick:))]
        fn tish_pick(&self, sender: Option<&AnyObject>) {
            let Some(s) = sender else { return };
            let Some(pb) = s.downcast_ref::<NSPopUpButton>() else {
                return;
            };
            let (root_id, idx) = decode_control_tag(pb.tag());
            let sel_i = pb.indexOfSelectedItem();
            let handler = PICK_HANDLERS.with(|c| {
                c.borrow()
                    .get(&root_id)
                    .and_then(|v| v.get(idx))
                    .and_then(|slot| slot.as_ref().map(|h| h.clone()))
            });
            if let Some(f) = handler {
                run_with_current_root(root_id, || f(sel_i as i64));
            }
        }

        /// SF Symbol toolbar items from [`super::toolbar_delegate::TishToolbarDelegate`].
        #[unsafe(method(tishToolbarClick:))]
        fn tish_toolbar_click(&self, sender: Option<&AnyObject>) {
            let Some(s) = sender else { return };
            let Some(item) = s.downcast_ref::<NSToolbarItem>() else {
                return;
            };
            let Some((root_id, idx)) = decode_toolbar_tag(item.tag() as isize) else {
                return;
            };
            run_with_current_root(root_id, || invoke_toolbar_action(root_id, idx));
        }
    }
);

impl MacosControlRouter {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        unsafe { msg_send![super(mtm.alloc().set_ivars(())), init] }
    }
}
