//! WKWebView script bridge — parity target for desktop `bridge.js` (`window.__TISH_APP__`).
//!
//! Opt in with `<webview bridge={true} id="…" onBridgeInvoke={fn} />`.
//! JS posts JSON strings to `webkit.messageHandlers.tish`; native replies / events via
//! `evaluateJavaScript` → `__TISH_APP__.__resolve` / `__dispatch`.

#![allow(non_snake_case)]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString, NSURL, NSURLRequest};
use objc2_web_kit::{
    WKScriptMessage, WKScriptMessageHandler, WKUserContentController, WKUserScript,
    WKUserScriptInjectionTime, WKWebView, WKWebViewConfiguration,
};
use tishlang_core::{ObjectMap, Value};
use tishlang_ui::runtime::RootId;

pub const HANDLER_NAME: &str = "tish";

/// Injected at document-start when `bridge` is enabled.
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
    /// Keep the handler alive; UCC also retains it until removeScriptMessageHandler.
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
    #[name = "TishWebViewScriptHandler"]
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
                eprintln!("tish-macos: bridge message body must be a JSON string");
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
        eprintln!("tish-macos: bridge JSON parse failed");
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
            eprintln!("tish-macos: unknown bridge message type {typ:?}");
        }
    }
}

fn reply_invoke(surface_id: &str, id: &str, ok: bool, value: &Value) {
    let json = value_to_json_string(value);
    let id_js = serde_json::to_string(id).unwrap_or_else(|_| "\"\"".into());
    let js = format!(
        "window.__TISH_APP__&&window.__TISH_APP__.__resolve({id_js},{ok},{json});"
    );
    let _ = evaluate_js(surface_id, &js);
}

/// Create a WKWebView with optional script bridge.
pub fn create_webview(
    mtm: MainThreadMarker,
    root_id: RootId,
    frame: objc2_foundation::NSRect,
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
    boot.push_str(&format!(
        "\nwindow.__TISH_SURFACE_ID__={sid_js};\n"
    ));
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

pub fn sync_bridge_handlers(surface_id: &str, props: &tishlang_core::PropMap) {
    BRIDGES.with(|m| {
        if let Some(e) = m.borrow_mut().get_mut(surface_id) {
            e.on_invoke = prop_invoke_handler(props);
            e.on_emit = prop_emit_handler(props);
        }
    });
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
        platform: Some("macos".into()),
        label: Some(surface_id.to_string()),
    });
}

fn unregister_surface(surface_id: &str) {
    let _ = tish_broker::GLOBAL_SURFACES.unregister(surface_id);
}

pub fn list_ids() -> Vec<String> {
    BRIDGES.with(|m| m.borrow().keys().cloned().collect())
}

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
        let html_ns = NSString::from_str(doc);
        unsafe {
            let _: () = msg_send![&*wv, loadHTMLString: &*html_ns, baseURL: None::<&NSURL>];
        }
        return Ok(());
    }
    let Some(src) = url.filter(|s| !s.is_empty()) else {
        return Err("webview.load requires url or html".into());
    };
    const PREFIX: &str = "data:text/html,";
    if let Some(rest) = src.strip_prefix(PREFIX) {
        let html_ns = NSString::from_str(rest);
        unsafe {
            let _: () = msg_send![&*wv, loadHTMLString: &*html_ns, baseURL: None::<&NSURL>];
        }
        return Ok(());
    }
    let Some(nsurl) = NSURL::URLWithString(&NSString::from_str(src)) else {
        return Err(format!("invalid url: {src}"));
    };
    let req = NSURLRequest::requestWithURL(&nsurl);
    unsafe {
        let _ = WKWebView::loadRequest(&*wv, &req);
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

/// Broker `webview.*` over host WK panes (desktop `local_invoke` / Tauri fallback).
pub fn broker_try_invoke(
    cmd: &str,
    args: &serde_json::Value,
) -> Option<Result<serde_json::Value, String>> {
    use serde_json::{json, Value as Json};
    match cmd {
        "webview.list" => {
            let labels = list_ids();
            Some(Ok(json!({ "ok": true, "labels": labels, "surfaceIds": labels })))
        }
        "webview.eval" => {
            let sid = match surface_id_arg(args) {
                Ok(s) => s,
                Err(e) => return Some(Err(e)),
            };
            let js = match args
                .get("js")
                .or_else(|| args.get("script"))
                .and_then(|v| v.as_str())
            {
                Some(s) => s,
                None => return Some(Err("js required".into())),
            };
            Some(evaluate_js(&sid, js).map(|_| json!({ "ok": true, "surfaceId": sid })))
        }
        "webview.postMessage" => {
            let sid = match surface_id_arg(args) {
                Ok(s) => s,
                Err(e) => return Some(Err(e)),
            };
            let channel = args
                .get("channel")
                .or_else(|| args.get("event"))
                .and_then(|v| v.as_str())
                .unwrap_or("message");
            let body = args
                .get("body")
                .or_else(|| args.get("payload"))
                .cloned()
                .unwrap_or(Json::Null);
            Some(
                post_event_json(&sid, channel, &body)
                    .map(|_| json!({ "ok": true, "surfaceId": sid, "channel": channel })),
            )
        }
        "webview.load" => {
            let sid = match surface_id_arg(args) {
                Ok(s) => s,
                Err(e) => return Some(Err(e)),
            };
            let url = args.get("url").and_then(|v| v.as_str());
            let html = args.get("html").and_then(|v| v.as_str());
            Some(load_content(&sid, url, html).map(|_| {
                json!({
                    "ok": true,
                    "surfaceId": sid,
                    "url": url,
                    "html": html.is_some(),
                })
            }))
        }
        _ if cmd.starts_with("webview.") => Some(Ok(tish_broker::unsupported("webview"))),
        _ => None,
    }
}

fn surface_id_arg(args: &serde_json::Value) -> Result<String, String> {
    args.get("surfaceId")
        .or_else(|| args.get("label"))
        .or_else(|| args.get("id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "surfaceId required".into())
}

pub fn evaluate_js(surface_id: &str, js: &str) -> Result<(), String> {
    let wv = BRIDGES.with(|m| m.borrow().get(surface_id).map(|e| e.webview.clone()));
    let Some(wv) = wv else {
        return Err(format!("no bridged webview for surfaceId={surface_id}"));
    };
    let ns = NSString::from_str(js);
    let block = RcBlock::new(|_obj: *mut AnyObject, err: *mut objc2_foundation::NSError| {
        if !err.is_null() {
            // Best-effort log; avoid panicking in ObjC callback.
            eprintln!("tish-macos: evaluateJavaScript error");
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

/// Push an event to every bridged WKWebView (desktop BrokerCore multi-transport).
pub fn broadcast_event(event: &str, payload: &Value) {
    let ids: Vec<String> = BRIDGES.with(|m| m.borrow().keys().cloned().collect());
    for id in ids {
        let _ = post_event(&id, event, payload);
    }
}

/// `macos.webviewEval(surfaceId, js)`
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

/// `macos.webviewPostMessage(surfaceId, event, payload?)`
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
    tish_apple_common::style::props_bool(props, keys, false)
}

fn props_string(props: &tishlang_core::PropMap, keys: &[&str]) -> Option<String> {
    tish_apple_common::style::props_string(props, keys)
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
        serde_json::Value::Array(a) => {
            Value::array(a.into_iter().map(json_to_value).collect())
        }
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
            // Clone first — send-values VmRef Mutex is non-reentrant; holding borrow
            // while recursing into nested values self-deadlocks (UI freeze at 0% CPU).
            let items: Vec<Value> = a.borrow().clone();
            let parts: Vec<String> = items.iter().map(value_to_json_string).collect();
            format!("[{}]", parts.join(","))
        }
        Value::Object(o) => {
            let entries: Vec<(String, Value)> = o
                .borrow()
                .strings
                .iter()
                .map(|(k, val)| (k.to_string(), val.clone()))
                .collect();
            let mut parts = Vec::new();
            for (k, val) in entries {
                let key = serde_json::to_string(&k).unwrap_or_else(|_| "\"\"".into());
                parts.push(format!("{key}:{}", value_to_json_string(&val)));
            }
            format!("{{{}}}", parts.join(","))
        }
        _ => "null".into(),
    }
}
