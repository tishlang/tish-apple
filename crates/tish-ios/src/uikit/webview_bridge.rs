//! WKWebView script bridge — same `__TISH_APP__` contract as tish-macos.
//!
//! Opt in with `<webview bridge={true} id="…" onBridgeInvoke={fn} />`.

#![allow(non_snake_case)]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSObject, NSObjectProtocol, NSString, NSURL, NSURLRequest};
use objc2_web_kit::{
    WKScriptMessage, WKScriptMessageHandler, WKUserContentController, WKUserScript,
    WKUserScriptInjectionTime, WKWebViewConfiguration,
};
use tishlang_core::{ObjectMap, Value};
use tishlang_ui::runtime::RootId;

use super::wk_webview::WKWebView;

pub const HANDLER_NAME: &str = "tish";

/// Injected at document-start when `bridge` is enabled (shared with macOS / desktop).
pub const BOOTSTRAP_JS: &str = r#"(function () {
  if (window.__TISH_APP__) return;
  var pending = new Map();
  var listeners = new Map();
  function post(msg) {
    try {
      webkit.messageHandlers.tish.postMessage(JSON.stringify(msg));
    } catch (e) {
      console.error("[tish-app] bridge post failed", e);
    }
  }
  window.__TISH_APP__ = {
    protocol: "desktop/v1",
    surface: "webview",
    getCurrentWindowLabel: function () {
      return window.__TISH_SURFACE_ID__ || "wk";
    },
    invoke: function (cmd, args) {
      var id = String(Date.now()) + "-" + Math.random().toString(36).slice(2);
      return new Promise(function (resolve, reject) {
        pending.set(id, { resolve: resolve, reject: reject });
        post({ type: "invoke", id: id, cmd: cmd, args: args || {} });
      });
    },
    listen: function (eventName, handler) {
      if (!listeners.has(eventName)) listeners.set(eventName, new Set());
      listeners.get(eventName).add(handler);
      return Promise.resolve(function () {
        var set = listeners.get(eventName);
        if (set) set.delete(handler);
      });
    },
    emit: function (eventName, payload) {
      post({ type: "emit", event: eventName, payload: payload });
      return Promise.resolve();
    },
    __resolve: function (id, ok, value) {
      var p = pending.get(id);
      if (!p) return;
      pending.delete(id);
      if (ok) p.resolve(value);
      else p.reject(value);
    },
    __dispatch: function (eventName, payload) {
      var set = listeners.get(eventName);
      if (!set) return;
      set.forEach(function (fn) {
        try { fn(payload); } catch (e) {}
      });
    },
  };
  window.__TISH_DESKTOP__ = window.__TISH_APP__;
})();"#;

struct BridgeEntry {
    webview: Retained<WKWebView>,
    #[allow(dead_code)]
    handler: Retained<TishWebViewScriptHandler>,
    on_invoke: Option<Rc<dyn Fn(String, Value) -> Value>>,
    on_emit: Option<Rc<dyn Fn(String, Value)>>,
}

thread_local! {
    static BRIDGES: RefCell<HashMap<String, BridgeEntry>> = RefCell::new(HashMap::new());
}

pub struct TishWebViewScriptHandlerIvars {
    surface_id: RefCell<String>,
    root_id: Cell<RootId>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishIosWebViewScriptHandler"]
    #[ivars = TishWebViewScriptHandlerIvars]
    pub struct TishWebViewScriptHandler;

    unsafe impl NSObjectProtocol for TishWebViewScriptHandler {}

    unsafe impl WKScriptMessageHandler for TishWebViewScriptHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        unsafe fn userContentController_didReceiveScriptMessage(
            &self,
            _user_content_controller: &WKUserContentController,
            message: &WKScriptMessage,
        ) {
            let surface = self.ivars().surface_id.borrow().clone();
            let body = message.body();
            let Some(ns) = body.downcast_ref::<NSString>() else {
                eprintln!("tish-ios: bridge message body must be a JSON string");
                return;
            };
            let text = ns.to_string();
            handle_bridge_message(&surface, &text);
        }
    }
);

impl TishWebViewScriptHandler {
    pub fn new(mtm: MainThreadMarker, root_id: RootId, surface_id: String) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TishWebViewScriptHandlerIvars {
            surface_id: RefCell::new(surface_id),
            root_id: Cell::new(root_id),
        });
        unsafe { msg_send![super(this), init] }
    }
}

fn handle_bridge_message(surface_id: &str, text: &str) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        eprintln!("tish-ios: bridge JSON parse failed");
        return;
    };
    let typ = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match typ {
        "invoke" => {
            let id = v
                .get("id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let cmd = v
                .get("cmd")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let args = json_to_value(v.get("args").cloned().unwrap_or(serde_json::Value::Null));
            let handler = BRIDGES.with(|m| {
                m.borrow()
                    .get(surface_id)
                    .and_then(|e| e.on_invoke.clone())
            });
            let result = if let Some(h) = handler {
                h(cmd, args)
            } else {
                let mut o = ObjectMap::default();
                o.insert(Arc::from("ok"), Value::Bool(false));
                o.insert(Arc::from("code"), Value::String("unsupported".into()));
                o.insert(Arc::from("capability"), Value::String("bridge".into()));
                o.insert(
                    Arc::from("message"),
                    Value::String("no onBridgeInvoke handler".into()),
                );
                Value::object(o)
            };
            reply_invoke(surface_id, &id, true, &result);
        }
        "emit" => {
            let event = v
                .get("event")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let payload = json_to_value(v.get("payload").cloned().unwrap_or(serde_json::Value::Null));
            let handler = BRIDGES.with(|m| {
                m.borrow()
                    .get(surface_id)
                    .and_then(|e| e.on_emit.clone())
            });
            if let Some(h) = handler {
                h(event, payload);
            }
        }
        _ => {
            eprintln!("tish-ios: unknown bridge message type {typ:?}");
        }
    }
}

fn reply_invoke(surface_id: &str, id: &str, ok: bool, value: &Value) {
    let json = value_to_json_string(value);
    let id_js = serde_json::to_string(id).unwrap_or_else(|_| "\"\"".into());
    let js = format!("window.__TISH_APP__&&window.__TISH_APP__.__resolve({id_js},{ok},{json});");
    let _ = evaluate_js(surface_id, &js);
}

pub fn create_webview(
    mtm: MainThreadMarker,
    root_id: RootId,
    frame: CGRect,
    props: &tishlang_core::PropMap,
) -> Retained<WKWebView> {
    let bridge = props_bool_true(props, &["bridge"]);
    let surface_id = props_string(props, &["id", "surfaceId", "label"])
        .unwrap_or_else(|| format!("wk-{root_id}"));

    if !bridge {
        return unsafe { WKWebView::initWithFrame(WKWebView::alloc(mtm), frame) };
    }

    let config = unsafe { WKWebViewConfiguration::new(mtm) };
    let ucc = unsafe { config.userContentController() };
    let handler = TishWebViewScriptHandler::new(mtm, root_id, surface_id.clone());
    let proto = ProtocolObject::from_ref(&*handler);
    let name = NSString::from_str(HANDLER_NAME);
    unsafe {
        ucc.addScriptMessageHandler_name(proto, &name);
    }

    let mut boot = String::from(BOOTSTRAP_JS);
    let sid_js = serde_json::to_string(&surface_id).unwrap_or_else(|_| "\"wk\"".into());
    boot.push_str(&format!("\nwindow.__TISH_SURFACE_ID__={sid_js};\n"));
    let src = NSString::from_str(&boot);
    let script = unsafe {
        WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
            WKUserScript::alloc(mtm),
            &src,
            WKUserScriptInjectionTime::AtDocumentStart,
            true,
        )
    };
    unsafe {
        ucc.addUserScript(&script);
    }

    let wv = unsafe {
        WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &config)
    };

    let on_invoke = prop_invoke_handler(props);
    let on_emit = prop_emit_handler(props);

    let sid = surface_id.clone();
    BRIDGES.with(|m| {
        m.borrow_mut().insert(
            surface_id,
            BridgeEntry {
                webview: wv.clone(),
                handler,
                on_invoke,
                on_emit,
            },
        );
    });
    register_surface(&sid);

    wv
}

pub fn detach_bridge(webview: &WKWebView) {
    let mut remove_keys: Vec<String> = Vec::new();
    BRIDGES.with(|m| {
        for (k, e) in m.borrow().iter() {
            if &*e.webview as *const WKWebView == webview as *const WKWebView {
                remove_keys.push(k.clone());
            }
        }
        let mut map = m.borrow_mut();
        for k in remove_keys {
            if let Some(entry) = map.remove(&k) {
                let ucc = unsafe { entry.webview.configuration().userContentController() };
                let name = NSString::from_str(HANDLER_NAME);
                unsafe {
                    ucc.removeScriptMessageHandlerForName(&name);
                }
                unregister_surface(&k);
            }
        }
    });
}

fn register_surface(surface_id: &str) {
    tish_broker::GLOBAL_SURFACES.register(tish_broker::SurfaceInfo {
        id: surface_id.to_string(),
        kind: tish_broker::SurfaceKind::Webview,
        platform: Some("ios".into()),
        label: Some(surface_id.to_string()),
    });
}

fn unregister_surface(surface_id: &str) {
    let _ = tish_broker::GLOBAL_SURFACES.unregister(surface_id);
}

pub fn list_ids() -> Vec<String> {
    BRIDGES.with(|m| m.borrow().keys().cloned().collect())
}

/// Load URL and/or HTML into a bridged WKWebView (`webview.load`).
pub fn load_content(
    surface_id: &str,
    url: Option<&str>,
    html: Option<&str>,
) -> Result<(), String> {
    let wv = BRIDGES.with(|m| m.borrow().get(surface_id).map(|e| e.webview.clone()));
    let Some(wv) = wv else {
        return Err(format!("no bridged webview for surfaceId={surface_id}"));
    };
    if let Some(doc) = html.filter(|s| !s.is_empty()) {
        unsafe {
            wv.loadHTMLString_baseURL(&NSString::from_str(doc), None);
        }
        return Ok(());
    }
    let Some(src) = url.filter(|s| !s.is_empty()) else {
        return Err("webview.load requires url or html".into());
    };
    const PREFIX: &str = "data:text/html,";
    if let Some(rest) = src.strip_prefix(PREFIX) {
        unsafe {
            wv.loadHTMLString_baseURL(&NSString::from_str(rest), None);
        }
        return Ok(());
    }
    let Some(nsurl) = NSURL::URLWithString(&NSString::from_str(src)) else {
        return Err(format!("invalid url: {src}"));
    };
    let req = NSURLRequest::requestWithURL(&nsurl);
    unsafe {
        wv.loadRequest(&req);
    }
    Ok(())
}

pub fn post_event_json(
    surface_id: &str,
    event: &str,
    payload: &serde_json::Value,
) -> Result<(), String> {
    let event_js = serde_json::to_string(event).unwrap_or_else(|_| "\"\"".into());
    let payload_js = serde_json::to_string(payload).unwrap_or_else(|_| "null".into());
    let js = format!(
        "window.__TISH_APP__&&window.__TISH_APP__.__dispatch({event_js},{payload_js});"
    );
    evaluate_js(surface_id, &js)
}

pub fn evaluate_js(surface_id: &str, js: &str) -> Result<(), String> {
    let wv = BRIDGES.with(|m| m.borrow().get(surface_id).map(|e| e.webview.clone()));
    let Some(wv) = wv else {
        return Err(format!("no bridged webview for surfaceId={surface_id}"));
    };
    let ns = NSString::from_str(js);
    let block = RcBlock::new(|_obj: *mut AnyObject, err: *mut objc2_foundation::NSError| {
        if !err.is_null() {
            eprintln!("tish-ios: evaluateJavaScript error");
        }
    });
    unsafe {
        wv.evaluateJavaScript_completionHandler(&ns, Some(&*block));
    }
    Ok(())
}

pub fn post_event(surface_id: &str, event: &str, payload: &Value) -> Result<(), String> {
    let event_js = serde_json::to_string(event).unwrap_or_else(|_| "\"\"".into());
    let payload_js = value_to_json_string(payload);
    let js = format!(
        "window.__TISH_APP__&&window.__TISH_APP__.__dispatch({event_js},{payload_js});"
    );
    evaluate_js(surface_id, &js)
}

pub fn broadcast_event(event: &str, payload: &Value) {
    let ids: Vec<String> = BRIDGES.with(|m| m.borrow().keys().cloned().collect());
    for id in ids {
        let _ = post_event(&id, event, payload);
    }
}

/// `ios.webviewEval(surfaceId, js)`
pub fn native_webview_eval(args: &[Value]) -> Value {
    let sid = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    let js = args
        .get(1)
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    match evaluate_js(&sid, &js) {
        Ok(()) => Value::Bool(true),
        Err(e) => Value::String(e.into()),
    }
}

/// `ios.webviewPostMessage(surfaceId, event, payload?)`
pub fn native_webview_post_message(args: &[Value]) -> Value {
    let sid = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    let event = args
        .get(1)
        .map(|v| v.to_display_string())
        .unwrap_or_else(|| "message".into());
    let payload = args.get(2).cloned().unwrap_or(Value::Null);
    match post_event(&sid, &event, &payload) {
        Ok(()) => Value::Bool(true),
        Err(e) => Value::String(e.into()),
    }
}

fn props_bool_true(props: &tishlang_core::PropMap, keys: &[&str]) -> bool {
    tishlang_apple_common::style::props_bool(props, keys, false)
}

fn props_string(props: &tishlang_core::PropMap, keys: &[&str]) -> Option<String> {
    tishlang_apple_common::style::props_string(props, keys)
}

fn prop_fn(props: &tishlang_core::PropMap, keys: &[&str]) -> Option<Value> {
    for k in keys {
        if let Some(v) = props.get(*k) {
            if matches!(v, Value::Function(_)) {
                return Some(v.clone());
            }
        }
    }
    None
}

fn prop_invoke_handler(props: &tishlang_core::PropMap) -> Option<Rc<dyn Fn(String, Value) -> Value>> {
    let f = prop_fn(props, &["onBridgeInvoke", "on_bridge_invoke", "onInvoke"])?;
    let Value::Function(func) = f else {
        return None;
    };
    Some(Rc::new(move |cmd: String, args: Value| {
        let mut o = ObjectMap::default();
        o.insert(Arc::from("cmd"), Value::String(cmd.into()));
        o.insert(Arc::from("args"), args);
        func.call(&[Value::object(o)])
    }))
}

fn prop_emit_handler(props: &tishlang_core::PropMap) -> Option<Rc<dyn Fn(String, Value)>> {
    let f = prop_fn(props, &["onBridgeEmit", "on_bridge_emit", "onEmit"])?;
    let Value::Function(func) = f else {
        return None;
    };
    Some(Rc::new(move |event: String, payload: Value| {
        let mut o = ObjectMap::default();
        o.insert(Arc::from("event"), Value::String(event.into()));
        o.insert(Arc::from("payload"), payload);
        let _ = func.call(&[Value::object(o)]);
    }))
}

fn json_to_value(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Value::String(s.into()),
        serde_json::Value::Array(a) => Value::array(a.into_iter().map(json_to_value).collect()),
        serde_json::Value::Object(map) => {
            let mut o = ObjectMap::default();
            for (k, v) in map {
                o.insert(Arc::from(k.as_str()), json_to_value(v));
            }
            Value::object(o)
        }
    }
}

fn value_to_json_string(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => {
            if n.is_finite() {
                n.to_string()
            } else {
                "null".into()
            }
        }
        Value::String(s) => serde_json::to_string(s.as_str()).unwrap_or_else(|_| "\"\"".into()),
        Value::Array(a) => {
            let parts: Vec<String> = a.borrow().iter().map(value_to_json_string).collect();
            format!("[{}]", parts.join(","))
        }
        Value::Object(o) => {
            let mut parts = Vec::new();
            for (k, val) in o.borrow().strings.iter() {
                let key = serde_json::to_string(&**k).unwrap_or_else(|_| "\"\"".into());
                parts.push(format!("{key}:{}", value_to_json_string(val)));
            }
            format!("{{{}}}", parts.join(","))
        }
        _ => "null".into(),
    }
}
