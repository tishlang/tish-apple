//! UIKit target/action router (click / text / switch).

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{NSObject, NSObjectProtocol};
use objc2_ui_kit::{UIButton, UISwitch, UITextField};
use tish_apple_common::handlers::{
    decode_control_tag, invoke_bool_handler, invoke_click_handler, invoke_text_change_handler,
};
use tishlang_ui::runtime::run_with_current_root;

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishIosControlRouter"]
    pub struct IosControlRouter;

    unsafe impl NSObjectProtocol for IosControlRouter {}

    impl IosControlRouter {
        #[unsafe(method(jsxClick:))]
        fn jsx_click(&self, sender: Option<&AnyObject>) {
            let Some(s) = sender else { return };
            let Some(btn) = s.downcast_ref::<UIButton>() else {
                return;
            };
            let (root_id, _) = decode_control_tag(btn.tag());
            run_with_current_root(root_id, || invoke_click_handler(btn.tag()));
        }

        #[unsafe(method(jsxTextChanged:))]
        fn jsx_text_changed(&self, sender: Option<&AnyObject>) {
            let Some(s) = sender else { return };
            let Some(tf) = s.downcast_ref::<UITextField>() else {
                return;
            };
            let text = tf
                .text()
                .map(|s| s.to_string())
                .unwrap_or_default();
            let (root_id, _) = decode_control_tag(tf.tag());
            run_with_current_root(root_id, || invoke_text_change_handler(tf.tag(), text));
        }

        #[unsafe(method(jsxBoolChanged:))]
        fn jsx_bool_changed(&self, sender: Option<&AnyObject>) {
            let Some(s) = sender else { return };
            let Some(sw) = s.downcast_ref::<UISwitch>() else {
                return;
            };
            let on = sw.isOn();
            let (root_id, _) = decode_control_tag(sw.tag());
            run_with_current_root(root_id, || invoke_bool_handler(sw.tag(), on));
        }
    }
);

pub fn new_router(mtm: MainThreadMarker) -> Retained<IosControlRouter> {
    unsafe { msg_send![IosControlRouter::alloc(mtm), init] }
}
