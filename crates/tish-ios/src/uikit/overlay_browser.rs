//! Native in-app browser surface for app shells (the Electron `BrowserView`
//! pattern): a SECOND `WKWebView` positioned over a rectangle the embedded page
//! chooses. A top-level webview is not a frame, so `X-Frame-Options` /
//! `frame-ancestors` do not apply — sites that refuse `<iframe>` embedding
//! (google.com, most large origins) render normally.
//!
//! Installed automatically on `<webview shell={true} bridge={true}>` surfaces as
//! the script-message channel **`tishBrowser`**. The page drives it with JSON
//! STRINGS (matching the `tish` bridge channel convention):
//!   {"cmd":"show","x":…,"y":…,"w":…,"h":…}   position + unhide (CSS px == pt
//!   {"cmd":"frame", …same…}                   for a full-bleed shell webview)
//!   {"cmd":"hide"}                            hide (page overlays open, panel gone)
//!   {"cmd":"navigate","url":"https://…"}
//!   {"cmd":"back"} / {"cmd":"forward"} / {"cmd":"reload"}
//! and navigation state flows back by evaluating
//!   window.__TISH_BROWSER_ON__({url,title,loading,canGoBack,canGoForward})
//! on the host webview after every navigation transition.

#![allow(non_snake_case)]

use std::cell::RefCell;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString, NSURL, NSURLRequest};
use objc2_web_kit::{WKScriptMessage, WKScriptMessageHandler, WKUserContentController};

use super::wk_webview::WKWebView;

pub const OVERLAY_HANDLER_NAME: &str = "tishBrowser";

pub struct TishOverlayBrowserIvars {
    host: RefCell<Option<Retained<WKWebView>>>,
    overlay: RefCell<Option<Retained<WKWebView>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishOverlayBrowser"]
    #[ivars = TishOverlayBrowserIvars]
    pub struct TishOverlayBrowser;

    unsafe impl NSObjectProtocol for TishOverlayBrowser {}

    unsafe impl WKScriptMessageHandler for TishOverlayBrowser {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        unsafe fn userContentController_didReceiveScriptMessage(
            &self,
            _ucc: &WKUserContentController,
            message: &WKScriptMessage,
        ) {
            let body = message.body();
            let Some(ns) = body.downcast_ref::<NSString>() else {
                eprintln!("tish-ios: tishBrowser message body must be a JSON string");
                return;
            };
            self.handle(&ns.to_string());
        }
    }

    // WKNavigationDelegate (informal here — conformance is by selector; WebKit
    // only needs the methods to exist). Every transition pushes fresh state.
    impl TishOverlayBrowser {
        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn nav_start(&self, _wv: &WKWebView, _nav: *mut AnyObject) {
            self.push_state();
        }

        #[unsafe(method(webView:didCommitNavigation:))]
        fn nav_commit(&self, _wv: &WKWebView, _nav: *mut AnyObject) {
            self.push_state();
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn nav_finish(&self, _wv: &WKWebView, _nav: *mut AnyObject) {
            self.push_state();
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn nav_fail(&self, _wv: &WKWebView, _nav: *mut AnyObject, _err: *mut AnyObject) {
            self.push_state();
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn nav_fail_prov(&self, _wv: &WKWebView, _nav: *mut AnyObject, _err: *mut AnyObject) {
            self.push_state();
        }
    }
);

impl TishOverlayBrowser {
    fn new(mtm: MainThreadMarker, host: Retained<WKWebView>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TishOverlayBrowserIvars {
            host: RefCell::new(Some(host)),
            overlay: RefCell::new(None),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn handle(&self, text: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
            eprintln!("tish-ios: tishBrowser message is not valid JSON: {text}");
            return;
        };
        let cmd = v.get("cmd").and_then(|x| x.as_str()).unwrap_or("");
        match cmd {
            "show" | "frame" => {
                let (x, y) = (num(&v, "x"), num(&v, "y"));
                let (w, h) = (num(&v, "w"), num(&v, "h"));
                if w > 1.0 && h > 1.0 {
                    if let Some(wv) = self.ensure_overlay() {
                        let rect = CGRect::new(CGPoint::new(x, y), CGSize::new(w, h));
                        unsafe {
                            let _: () = msg_send![&*wv, setFrame: rect];
                            let _: () = msg_send![&*wv, setHidden: false];
                        }
                    }
                }
            }
            "hide" => {
                if let Some(wv) = self.ivars().overlay.borrow().as_ref() {
                    unsafe {
                        let _: () = msg_send![&**wv, setHidden: true];
                    }
                }
            }
            "navigate" => {
                let url = v.get("url").and_then(|x| x.as_str()).unwrap_or("");
                if url.is_empty() {
                    return;
                }
                if let Some(wv) = self.ensure_overlay() {
                    let ns = NSString::from_str(url);
                    if let Some(nsurl) = NSURL::URLWithString(&ns) {
                        let req = NSURLRequest::requestWithURL(&nsurl);
                        unsafe { wv.loadRequest(&req) };
                    }
                }
            }
            "back" => self.overlay_send(sel!(goBack)),
            "forward" => self.overlay_send(sel!(goForward)),
            "reload" => self.overlay_send(sel!(reload)),
            other => eprintln!("tish-ios: unknown tishBrowser cmd {other:?}"),
        }
    }

    fn overlay_send(&self, sel: objc2::runtime::Sel) {
        if let Some(wv) = self.ivars().overlay.borrow().as_ref() {
            unsafe {
                let _: *mut AnyObject = msg_send![&**wv, performSelector: sel];
            }
        }
    }

    fn ensure_overlay(&self) -> Option<Retained<WKWebView>> {
        if let Some(wv) = self.ivars().overlay.borrow().as_ref() {
            return Some(wv.clone());
        }
        let host = self.ivars().host.borrow().as_ref()?.clone();
        let superview: Option<Retained<AnyObject>> = unsafe { msg_send![&*host, superview] };
        let superview = superview?;
        let mtm = MainThreadMarker::new()?;
        let wv = unsafe { WKWebView::initWithFrame(WKWebView::alloc(mtm), CGRect::ZERO) };
        unsafe {
            let proto: &ProtocolObject<dyn NSObjectProtocol> = ProtocolObject::from_ref(self);
            let _: () = msg_send![&*wv, setNavigationDelegate: proto];
            let _: () = msg_send![&*wv, setHidden: true];
            let sel = sel!(setInspectable:);
            let responds: bool = msg_send![&*wv, respondsToSelector: sel];
            if responds {
                let _: () = msg_send![&*wv, setInspectable: true];
            }
            // Above the host webview, same parent — composites and dies with the app.
            let _: () = msg_send![&*superview, addSubview: &*wv];
        }
        *self.ivars().overlay.borrow_mut() = Some(wv.clone());
        Some(wv)
    }

    fn push_state(&self) {
        let overlay = self.ivars().overlay.borrow().as_ref().cloned();
        let host = self.ivars().host.borrow().as_ref().cloned();
        let (Some(overlay), Some(host)) = (overlay, host) else {
            return;
        };
        let url: Option<Retained<NSURL>> = unsafe { msg_send![&*overlay, URL] };
        let url = url
            .and_then(|u| u.absoluteString().map(|s| s.to_string()))
            .unwrap_or_default();
        let title: Option<Retained<NSString>> = unsafe { msg_send![&*overlay, title] };
        let title = title.map(|s| s.to_string()).unwrap_or_default();
        let loading: bool = unsafe { msg_send![&*overlay, isLoading] };
        let can_back: bool = unsafe { msg_send![&*overlay, canGoBack] };
        let can_forward: bool = unsafe { msg_send![&*overlay, canGoForward] };
        let state = serde_json::json!({
            "url": url,
            "title": title,
            "loading": loading,
            "canGoBack": can_back,
            "canGoForward": can_forward,
        });
        let js = format!("window.__TISH_BROWSER_ON__&&window.__TISH_BROWSER_ON__({state});");
        let ns = NSString::from_str(&js);
        let block = RcBlock::new(|_obj: *mut AnyObject, _err: *mut objc2_foundation::NSError| {});
        unsafe {
            host.evaluateJavaScript_completionHandler(&ns, Some(&*block));
        }
    }
}

thread_local! {
    static OVERLAYS: RefCell<Vec<Retained<TishOverlayBrowser>>> = const { RefCell::new(Vec::new()) };
}

/// Attach the overlay-browser channel to a shell webview's content controller.
/// Called from `create_webview` for `shell={true} bridge={true}` surfaces.
pub fn install_overlay_browser(
    mtm: MainThreadMarker,
    ucc: &WKUserContentController,
    host: &Retained<WKWebView>,
) {
    let handler = TishOverlayBrowser::new(mtm, host.clone());
    let proto = ProtocolObject::from_ref(&*handler);
    let name = NSString::from_str(OVERLAY_HANDLER_NAME);
    unsafe {
        ucc.addScriptMessageHandler_name(proto, &name);
    }
    OVERLAYS.with(|v| v.borrow_mut().push(handler));
}

fn num(v: &serde_json::Value, key: &str) -> f64 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(0.0)
}
