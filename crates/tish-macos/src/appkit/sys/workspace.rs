//! `macos.workspace`: opening things with NSWorkspace, and moving files to the Trash.
//!
//! - `open(target)`: a URL (`https://…`, `mailto:…`, any scheme) or a file, folder or app path ->
//!   whether macOS took it
//! - `reveal(path)`: select it in a Finder window -> false when it doesn't exist
//! - `trash(path)` -> `{ ok, path, error }`, `path` being where it went in the Trash
//! - `appsFor(path)` -> `[{ name, path, default }]`, the apps that can open it, default first
//! - `openWith(path, app)` -> `{ ok, error }`

use std::path::Path;

use objc2::rc::Retained;
use objc2_app_kit::{NSWorkspace, NSWorkspaceOpenConfiguration};
use objc2_foundation::{NSArray, NSFileManager, NSString, NSURL};
use tishlang_core::Value;

use super::{arr, obj, s, str_arg};

fn result(r: Result<(), String>) -> Value {
    match r {
        Ok(()) => obj(vec![("ok", Value::Bool(true)), ("error", s(""))]),
        Err(e) => obj(vec![("ok", Value::Bool(false)), ("error", s(&e))]),
    }
}

pub(super) fn open(args: &[Value]) -> Value {
    let target = str_arg(args, 0);
    let ns = NSString::from_str(&target);
    let url = if target.contains(':') && !target.starts_with('/') {
        NSURL::URLWithString(&ns)
    } else {
        Some(NSURL::fileURLWithPath(&ns))
    };
    Value::Bool(url.is_some_and(|u| NSWorkspace::sharedWorkspace().openURL(&u)))
}

pub(super) fn reveal(args: &[Value]) -> Value {
    let path = str_arg(args, 0);
    if !Path::new(&path).exists() {
        return Value::Bool(false);
    }
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path));
    NSWorkspace::sharedWorkspace()
        .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
    Value::Bool(true)
}

pub(super) fn trash(args: &[Value]) -> Value {
    let path = str_arg(args, 0);
    if !Path::new(&path).exists() {
        return obj(vec![
            ("ok", Value::Bool(false)),
            ("path", s("")),
            ("error", s(&format!("no such file: {path}"))),
        ]);
    }
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path));
    let mut out: Option<Retained<NSURL>> = None;
    match NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, Some(&mut out))
    {
        Ok(()) => {
            let went = out
                .and_then(|u| u.path())
                .map(|p| p.to_string())
                .unwrap_or_default();
            obj(vec![
                ("ok", Value::Bool(true)),
                ("path", s(&went)),
                ("error", s("")),
            ])
        }
        Err(e) => obj(vec![
            ("ok", Value::Bool(false)),
            ("path", s("")),
            ("error", s(&e.localizedDescription().to_string())),
        ]),
    }
}

pub(super) fn apps_for(args: &[Value]) -> Value {
    let ws = NSWorkspace::sharedWorkspace();
    let url = NSURL::fileURLWithPath(&NSString::from_str(&str_arg(args, 0)));
    let fm = NSFileManager::defaultManager();
    let default = ws
        .URLForApplicationToOpenURL(&url)
        .and_then(|u| u.path())
        .map(|p| p.to_string())
        .unwrap_or_default();
    let mut out: Vec<(String, String, bool)> = Vec::new();
    for app in ws.URLsForApplicationsToOpenURL(&url).iter() {
        let Some(p) = app.path().map(|p| p.to_string()) else {
            continue;
        };
        if out.iter().any(|o| o.1 == p) {
            continue;
        }
        let name = fm.displayNameAtPath(&NSString::from_str(&p)).to_string();
        let name = name.strip_suffix(".app").unwrap_or(&name).to_string();
        out.push((name, p.clone(), p == default));
    }
    out.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then(a.0.to_lowercase().cmp(&b.0.to_lowercase()))
    });
    arr(out
        .into_iter()
        .map(|(n, p, d)| {
            obj(vec![
                ("name", s(&n)),
                ("path", s(&p)),
                ("default", Value::Bool(d)),
            ])
        })
        .collect())
}

pub(super) fn open_with(args: &[Value]) -> Value {
    let (path, app) = (str_arg(args, 0), str_arg(args, 1));
    if !Path::new(&path).exists() {
        return result(Err(format!("no such file: {path}")));
    }
    if !Path::new(&app).exists() {
        return result(Err(format!("no such app: {app}")));
    }
    let file = NSURL::fileURLWithPath(&NSString::from_str(&path));
    let app_url = NSURL::fileURLWithPath(&NSString::from_str(&app));
    NSWorkspace::sharedWorkspace().openURLs_withApplicationAtURL_configuration_completionHandler(
        &NSArray::from_retained_slice(&[file]),
        &app_url,
        &NSWorkspaceOpenConfiguration::new(),
        None,
    );
    result(Ok(()))
}
