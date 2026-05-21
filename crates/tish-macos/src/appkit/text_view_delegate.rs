//! `NSTextViewDelegate` for `TextEditor` `onChange` / `onInput`.

use objc2::rc::Retained;
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSTextDelegate, NSTextView, NSTextViewDelegate};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol};

use tishlang_ui::runtime::run_with_current_root;

use super::handlers::{
    decode_control_tag, text_change_tag_from_text_view, TEXT_CHANGE_HANDLERS,
};

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishMacosTextViewDelegate"]
    pub struct TextViewDelegate;

    unsafe impl NSObjectProtocol for TextViewDelegate {}

    unsafe impl NSTextDelegate for TextViewDelegate {
        #[unsafe(method(textDidChange:))]
        fn textDidChange(&self, n: &NSNotification) {
            let Some(obj) = n.object() else {
                return;
            };
            let Some(tv) = obj.downcast_ref::<NSTextView>() else {
                return;
            };
            let tag = text_change_tag_from_text_view(tv)
                .unwrap_or_else(|| tv.tag() as isize);
            if tag < 0 {
                return;
            }
            let (root_id, idx) = decode_control_tag(tag);
            let s = tv.string().to_string();
            let handler = TEXT_CHANGE_HANDLERS.with(|c| {
                c.borrow()
                    .get(&root_id)
                    .and_then(|v| v.get(idx))
                    .and_then(|slot| slot.as_ref().map(|h| h.clone()))
            });
            if let Some(f) = handler {
                run_with_current_root(root_id, || f(s));
            }
        }
    }

    unsafe impl NSTextViewDelegate for TextViewDelegate {}
);

impl TextViewDelegate {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        unsafe { msg_send![super(mtm.alloc().set_ivars(())), init] }
    }
}
