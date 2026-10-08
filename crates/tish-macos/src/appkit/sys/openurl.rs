//! `macos.onOpenUrl(cb)`: `cb(url)` for every URL macOS opens with this app (a scheme its
//! Info.plist claims under `CFBundleURLTypes`). macOS delivers each as a GetURL Apple Event, and a
//! cold launch queues it until a handler is installed, so call this before the run loop starts.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_foundation::NSString;
use tishlang_core::Value;

use super::{call, s};

/// 'GURL', the event class and id of a GetURL Apple Event.
const K_AE_GET_URL: u32 = u32::from_be_bytes(*b"GURL");
/// '----', the direct-object keyword holding the URL.
const KEY_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

thread_local! {
    static OPEN_URL: RefCell<Option<Value>> = const { RefCell::new(None) };
    static URL_TARGET: RefCell<Option<Retained<UrlTarget>>> = const { RefCell::new(None) };
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishMacosUrlTarget"]
    struct UrlTarget;

    impl UrlTarget {
        #[unsafe(method(handleGetURL:withReplyEvent:))]
        fn handle_get_url(&self, event: &AnyObject, _reply: &AnyObject) {
            let url: Option<String> = unsafe {
                let desc: *mut AnyObject = msg_send![event, paramDescriptorForKeyword: KEY_DIRECT_OBJECT];
                desc.as_ref().and_then(|d| {
                    let s: *mut NSString = msg_send![d, stringValue];
                    s.as_ref().map(|s| s.to_string())
                })
            };
            if let Some(url) = url {
                if let Some(cb) = OPEN_URL.with(|h| h.borrow().clone()) {
                    call(&cb, &[s(&url)]);
                }
            }
        }
    }
);

pub(super) fn on_open_url(args: &[Value]) -> Value {
    let Some(mtm) = MainThreadMarker::new() else {
        return Value::Bool(false);
    };
    let Some(cls) = AnyClass::get(c"NSAppleEventManager") else {
        return Value::Bool(false);
    };
    OPEN_URL.with(|h| *h.borrow_mut() = args.first().cloned());
    let target: Retained<UrlTarget> = unsafe { msg_send![UrlTarget::alloc(mtm), init] };
    unsafe {
        let manager: *mut AnyObject = msg_send![cls, sharedAppleEventManager];
        let _: () = msg_send![manager, setEventHandler: &*target, andSelector: sel!(handleGetURL:withReplyEvent:), forEventClass: K_AE_GET_URL, andEventID: K_AE_GET_URL];
    }
    URL_TARGET.with(|t| *t.borrow_mut() = Some(target));
    Value::Bool(true)
}
