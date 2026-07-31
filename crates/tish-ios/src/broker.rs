//! In-process broker for iOS (`state.*`, caps) — no Tauri.

use std::sync::Arc;

use serde_json::{json, Value as Json};
use tishlang_broker::CapBackend;
use tishlang_core::{ObjectMap, Value};

struct IosCaps;

impl CapBackend for IosCaps {
    fn try_invoke(&self, cmd: &str, args: &Json) -> Option<Result<Json, String>> {
        if cmd.starts_with("notification.") {
            return Some(crate::notifications::invoke(cmd, args));
        }
        if cmd.starts_with("dialog.") {
            return Some(crate::dialog::invoke(cmd, args));
        }
        if cmd.starts_with("webview.") {
            return crate::webview_cmds::invoke(cmd, args);
        }
        if cmd.starts_with("store.") {
            return crate::store::invoke(cmd, args);
        }
        None
    }
}

pub fn invoke_json(cmd: &str, args: Json) -> Result<Json, String> {
    let result = tishlang_broker::invoke(cmd, args.clone(), "ios", Some(&IosCaps))?;
    maybe_broadcast_state(cmd, &args, &result);
    Ok(result)
}

/// Push `state:changed` to bridged WKWebViews (desktop multi-transport parity).
fn maybe_broadcast_state(cmd: &str, args: &Json, result: &Json) {
    if !matches!(cmd, "state.set" | "state.patch" | "state.delete") {
        return;
    }
    if result.get("ok") != Some(&Json::Bool(true)) {
        return;
    }
    let path = result
        .get("path")
        .or_else(|| args.get("path"))
        .or_else(|| args.get("key"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let value = result.get("value").cloned().unwrap_or(Json::Null);
    let revision = result.get("revision").and_then(|v| v.as_u64()).unwrap_or(0);
    let source = args
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("ios");
    let payload = tishlang_broker::state_changed_payload(path, &value, revision, source);
    #[cfg(target_os = "ios")]
    {
        crate::broadcast_event("state:changed", &json_to_value(&payload));
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = payload;
    }
}

/// `invoke(cmd, args?)` — Tish native.
pub fn native_invoke(args: &[Value]) -> Value {
    let cmd = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    if cmd.is_empty() {
        return err_obj("invoke(cmd, args?): cmd required");
    }
    let args_json = args
        .get(1)
        .and_then(value_to_json)
        .unwrap_or(json!({}));
    match invoke_json(&cmd, args_json) {
        Ok(v) => json_to_value(&v),
        Err(e) => err_obj(&e),
    }
}

pub fn native_state_get(args: &[Value]) -> Value {
    let path = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    let mut o = ObjectMap::default();
    o.insert(Arc::from("path"), Value::String(path.into()));
    native_invoke(&[Value::String("state.get".into()), Value::object(o)])
}

pub fn native_state_set(args: &[Value]) -> Value {
    let path = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    let value = args.get(1).cloned().unwrap_or(Value::Null);
    let mut o = ObjectMap::default();
    o.insert(Arc::from("path"), Value::String(path.into()));
    o.insert(Arc::from("value"), value);
    native_invoke(&[Value::String("state.set".into()), Value::object(o)])
}

fn err_obj(msg: &str) -> Value {
    let mut o = ObjectMap::default();
    o.insert(Arc::from("ok"), Value::Bool(false));
    o.insert(Arc::from("error"), Value::String(msg.into()));
    Value::object(o)
}

fn value_to_json(v: &Value) -> Option<Json> {
    match v {
        Value::Null => Some(Json::Null),
        Value::Bool(b) => Some(Json::Bool(*b)),
        Value::Number(n) => serde_json::Number::from_f64(*n).map(Json::Number),
        Value::String(s) => Some(Json::String(s.to_string())),
        Value::Array(a) => {
            let items: Option<Vec<_>> = a.borrow().iter().map(value_to_json).collect();
            items.map(Json::Array)
        }
        Value::Object(o) => {
            let mut map = serde_json::Map::new();
            for (k, val) in o.borrow().strings.iter() {
                map.insert(k.to_string(), value_to_json(val)?);
            }
            Some(Json::Object(map))
        }
        _ => None,
    }
}

fn json_to_value(v: &Json) -> Value {
    match v {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        Json::String(s) => Value::String(s.clone().into()),
        Json::Array(arr) => Value::array(arr.iter().map(json_to_value).collect()),
        Json::Object(obj) => {
            let mut m = ObjectMap::default();
            for (k, val) in obj {
                m.insert(Arc::from(k.as_str()), json_to_value(val));
            }
            Value::object(m)
        }
    }
}
