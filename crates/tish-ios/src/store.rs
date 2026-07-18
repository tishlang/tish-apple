//! Persisted `store.*` — same command contract as desktop Tauri plugin-store.
//! Backed by `NSUserDefaults` (JSON object per store path).

use serde_json::{json, Value as Json};

pub fn invoke(cmd: &str, args: &Json) -> Option<Result<Json, String>> {
    match cmd {
        "store.get" | "store.set" | "store.delete" | "store.keys" | "store.clear" => {}
        _ => return None,
    }
    #[cfg(target_os = "ios")]
    {
        Some(dispatch(cmd, args))
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = args;
        Some(Ok(tish_broker::unsupported_on("store", "ios")))
    }
}

#[cfg(target_os = "ios")]
fn dispatch(cmd: &str, args: &Json) -> Result<Json, String> {
    use std::collections::BTreeMap;

    use objc2_foundation::{NSString, NSUserDefaults};

    fn store_path(args: &Json) -> String {
        args.get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("store.json")
            .to_string()
    }

    fn defaults_key(path: &str) -> String {
        format!("tish.store.{path}")
    }

    fn load_map(path: &str) -> BTreeMap<String, Json> {
        let ud = NSUserDefaults::standardUserDefaults();
        let key = NSString::from_str(&defaults_key(path));
        let Some(s) = ud.stringForKey(&key) else {
            return BTreeMap::new();
        };
        let text = s.to_string();
        if text.is_empty() {
            return BTreeMap::new();
        }
        match serde_json::from_str::<BTreeMap<String, Json>>(&text) {
            Ok(m) => m,
            Err(_) => BTreeMap::new(),
        }
    }

    fn save_map(path: &str, map: &BTreeMap<String, Json>) -> Result<(), String> {
        let ud = NSUserDefaults::standardUserDefaults();
        let key = NSString::from_str(&defaults_key(path));
        let text = serde_json::to_string(map).map_err(|e| e.to_string())?;
        let value = NSString::from_str(&text);
        unsafe {
            ud.setObject_forKey(Some(&*value), &key);
        }
        let _ = ud.synchronize();
        Ok(())
    }

    match cmd {
        "store.get" => {
            let path = store_path(args);
            let key = args
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or("key required")?;
            let map = load_map(&path);
            Ok(json!({
                "ok": true,
                "value": map.get(key).cloned().unwrap_or(Json::Null),
            }))
        }
        "store.set" => {
            let path = store_path(args);
            let key = args
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or("key required")?
                .to_string();
            let value = args.get("value").cloned().unwrap_or(Json::Null);
            let mut map = load_map(&path);
            map.insert(key, value);
            save_map(&path, &map)?;
            Ok(json!({ "ok": true }))
        }
        "store.delete" => {
            let path = store_path(args);
            let key = args
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or("key required")?;
            let mut map = load_map(&path);
            let deleted = map.remove(key).is_some();
            save_map(&path, &map)?;
            Ok(json!({ "ok": true, "deleted": deleted }))
        }
        "store.keys" => {
            let path = store_path(args);
            let map = load_map(&path);
            let keys: Vec<String> = map.keys().cloned().collect();
            Ok(json!({ "ok": true, "keys": keys }))
        }
        "store.clear" => {
            let path = store_path(args);
            save_map(&path, &BTreeMap::new())?;
            Ok(json!({ "ok": true }))
        }
        _ => Ok(tish_broker::unsupported("store")),
    }
}
