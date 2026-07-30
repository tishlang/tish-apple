//! `UITextViewDelegate` for `text_editor` `onChange` / `onInput`.

use objc2::rc::Retained;
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{NSObject, NSObjectProtocol};
use objc2_ui_kit::{UIScrollViewDelegate, UITextView, UITextViewDelegate};
use tishlang_apple_common::handlers::{decode_control_tag, invoke_text_change_handler};
use tishlang_ui::runtime::run_with_current_root;

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishIosTextViewDelegate"]
    pub struct IosTextViewDelegate;

    unsafe impl NSObjectProtocol for IosTextViewDelegate {}

    unsafe impl UIScrollViewDelegate for IosTextViewDelegate {}

    unsafe impl UITextViewDelegate for IosTextViewDelegate {
        #[unsafe(method(textViewDidChange:))]
        fn text_view_did_change(&self, text_view: &UITextView) {
            let tag = text_view.tag();
            if tag == 0 {
                return;
            }
            let text = text_view.text().to_string();
            let (root_id, _) = decode_control_tag(tag);
            run_with_current_root(root_id, || invoke_text_change_handler(tag, text));
        }
    }
);

impl IosTextViewDelegate {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        unsafe { msg_send![IosTextViewDelegate::alloc(mtm), init] }
    }
}
