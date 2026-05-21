//! Shared `NSTextFieldDelegate` for `onChange` / `onInput` (clone handler before Tish).

use objc2::rc::Retained;
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSControl, NSControlTextEditingDelegate, NSTextFieldDelegate};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol};

use tishlang_ui::runtime::run_with_current_root;

use super::handlers::{decode_control_tag, TEXT_CHANGE_HANDLERS};

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishMacosTextFieldDelegate"]
    pub struct TextFieldDelegate;

    unsafe impl NSObjectProtocol for TextFieldDelegate {}

    unsafe impl NSControlTextEditingDelegate for TextFieldDelegate {
        #[unsafe(method(controlTextDidChange:))]
        fn controlTextDidChange(&self, n: &NSNotification) {
            let Some(obj) = n.object() else {
                return;
            };
            let Some(ctrl) = obj.downcast_ref::<NSControl>() else {
                return;
            };
            let (root_id, idx) = decode_control_tag(ctrl.tag());
            let text = ctrl.stringValue().to_string();
            let handler = TEXT_CHANGE_HANDLERS.with(|c| {
                c.borrow()
                    .get(&root_id)
                    .and_then(|v| v.get(idx))
                    .and_then(|slot| slot.as_ref().map(|h| h.clone()))
            });
            if let Some(f) = handler {
                run_with_current_root(root_id, || f(text));
            }
        }
    }

    unsafe impl NSTextFieldDelegate for TextFieldDelegate {}
);

impl TextFieldDelegate {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        unsafe { msg_send![super(mtm.alloc().set_ivars(())), init] }
    }
}
