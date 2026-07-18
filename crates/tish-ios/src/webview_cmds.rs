//! Broker `webview.*` over host WK panes (`<webview bridge id=…>`).
//!
//! Same invoke surface as desktop Tauri panes: `webview.load` / `postMessage` / `list` / `eval`.
//! Host helpers `ios.webviewEval` / `ios.webviewPostMessage` remain as thin aliases.

use serde_json::Value as Json;

pub fn invoke(cmd: &str, args: &Json) -> Option<Result<Json, String>> {
    match cmd {
        "webview.load" | "webview.postMessage" | "webview.list" | "webview.eval" => {}
        _ => return None,
    }
    #[cfg(target_os = "ios")]
    {
        Some(dispatch(cmd, args))
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = args;
        Some(Ok(tish_broker::unsupported_on("webview", "ios")))
    }
}

#[cfg(target_os = "ios")]
fn dispatch(cmd: &str, args: &Json) -> Result<Json, String> {
    use crate::uikit::webview_bridge;
    use serde_json::json;

    match cmd {
        "webview.list" => {
            let labels = webview_bridge::list_ids();
            Ok(json!({ "ok": true, "labels": labels, "surfaceIds": labels }))
        }
        "webview.eval" => {
            let sid = surface_id(args)?;
            let js = args
                .get("js")
                .or_else(|| args.get("script"))
                .and_then(|v| v.as_str())
                .ok_or("js required")?;
            webview_bridge::evaluate_js(&sid, js)?;
            Ok(json!({ "ok": true, "surfaceId": sid }))
        }
        "webview.postMessage" => {
            let sid = surface_id(args)?;
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
            webview_bridge::post_event_json(&sid, channel, &body)?;
            Ok(json!({ "ok": true, "surfaceId": sid, "channel": channel }))
        }
        "webview.load" => {
            let sid = surface_id(args)?;
            let url = args.get("url").and_then(|v| v.as_str());
            let html = args.get("html").and_then(|v| v.as_str());
            webview_bridge::load_content(&sid, url, html)?;
            Ok(json!({
                "ok": true,
                "surfaceId": sid,
                "url": url,
                "html": html.is_some(),
            }))
        }
        _ => Ok(tish_broker::unsupported("webview")),
    }
}

#[cfg(target_os = "ios")]
fn surface_id(args: &Json) -> Result<String, String> {
    args.get("surfaceId")
        .or_else(|| args.get("label"))
        .or_else(|| args.get("id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "surfaceId required".into())
}
