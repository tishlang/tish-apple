//! Thin UIView-backed `WKWebView` binding.
//!
//! `objc2-web-kit`'s generated `WKWebView` is macOS/`NSView`-only
//! ([objc2#637](https://github.com/madsmtm/objc2/issues/637)). Pattern matches wry's iOS host.

#![allow(non_snake_case)]

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{extern_class, extern_methods, MainThreadOnly};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSString, NSURL};
use objc2_ui_kit::{UIResponder, UIView};
use objc2_web_kit::WKWebViewConfiguration;

extern_class!(
    #[unsafe(super(UIView, UIResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[derive(Debug, PartialEq, Eq, Hash)]
    pub struct WKWebView;
);

unsafe impl NSObjectProtocol for WKWebView {}

impl WKWebView {
    extern_methods!(
        #[unsafe(method(configuration))]
        pub unsafe fn configuration(&self) -> Retained<WKWebViewConfiguration>;

        #[unsafe(method(initWithFrame:configuration:))]
        pub unsafe fn initWithFrame_configuration(
            this: Allocated<Self>,
            frame: CGRect,
            configuration: &WKWebViewConfiguration,
        ) -> Retained<Self>;

        #[unsafe(method(initWithFrame:))]
        pub unsafe fn initWithFrame(this: Allocated<Self>, frame: CGRect) -> Retained<Self>;

        #[unsafe(method(loadRequest:))]
        pub unsafe fn loadRequest(&self, request: &objc2_foundation::NSURLRequest);

        #[unsafe(method(loadHTMLString:baseURL:))]
        pub unsafe fn loadHTMLString_baseURL(
            &self,
            string: &NSString,
            base_url: Option<&NSURL>,
        );

        #[unsafe(method(evaluateJavaScript:completionHandler:))]
        pub unsafe fn evaluateJavaScript_completionHandler(
            &self,
            java_script_string: &NSString,
            completion_handler: Option<
                &block2::Block<dyn Fn(*mut AnyObject, *mut objc2_foundation::NSError)>,
            >,
        );
    );
}
