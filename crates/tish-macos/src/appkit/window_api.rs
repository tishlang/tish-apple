//! Subset of web `window` backed by `NSWindow` (see plan § Tier 5).
//! Global `window.*` targets the current Tish root’s window ([`tishlang_ui::runtime::current_root_id`]).

use std::sync::Arc;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSScreen, NSView, NSWindow};
use objc2_core_foundation::CGSize;
use objc2_foundation::NSString;
use tishlang_core::{ObjectMap, Value};
use tishlang_ui::runtime::{current_root_id, LEGACY_ROOT_ID, RootId};

use super::handlers::{
    detail_metrics_ptr_for_root, is_split_sidebar_collapsed, toggle_split_sidebar, window_for_root,
};

pub(super) fn snap_window_to_region(win: &NSWindow, region: &str) {
    let mtm = MainThreadMarker::new().expect("snapToRegion on main thread");
    let screen = win.screen().or_else(|| NSScreen::mainScreen(mtm));
    let Some(sc) = screen else {
        return;
    };
    let vf = sc.visibleFrame();
    let mut frame = vf;
    let half = (vf.size.width / 2.0).max(200.0);
    match region {
        "leftHalf" | "left" => {
            frame.size.width = half;
        }
        "rightHalf" | "right" => {
            frame.origin.x = vf.origin.x + vf.size.width - half;
            frame.size.width = half;
        }
        "maximizeVisible" | "maximize" => {}
        _ => return,
    }
    win.setFrame_display(frame, true);
}

/// Per-root `nsWindow` API: resolves [`NSWindow`] via [`window_for_root`] on each call.
/// `macos.openWindow` uses this so the returned handle does **not** hold long-lived
/// [`Retained`] clones. After `windowWillClose:` unregisters the root, methods become no-ops
/// instead of retaining the window across host teardown.
pub fn ns_window_object_for_root(root_id: RootId) -> Value {
    let mut m = ObjectMap::default();

    let rid = root_id;
    m.insert(
        Arc::from("title"),
        Value::native(move |_a: &[Value]| {
            match window_for_root(rid) {
                Some(w) => Value::String(w.title().to_string().into()),
                None => Value::Null,
            }
        }),
    );

    let rid = root_id;
    m.insert(
        Arc::from("setTitle"),
        Value::native(move |a: &[Value]| {
            let Some(w) = window_for_root(rid) else {
                return Value::Null;
            };
            let title = a.first().map(|v| v.to_display_string()).unwrap_or_default();
            w.setTitle(&NSString::from_str(&title));
            Value::Null
        }),
    );

    let rid = root_id;
    m.insert(
        Arc::from("innerWidth"),
        Value::native(move |_a: &[Value]| {
            match window_for_root(rid) {
                Some(w) => {
                    let r = w.contentLayoutRect();
                    Value::Number(r.size.width as f64)
                }
                None => Value::Null,
            }
        }),
    );

    let rid = root_id;
    m.insert(
        Arc::from("innerHeight"),
        Value::native(move |_a: &[Value]| {
            match window_for_root(rid) {
                Some(w) => {
                    let r = w.contentLayoutRect();
                    Value::Number(r.size.height as f64)
                }
                None => Value::Null,
            }
        }),
    );

    let rid = root_id;
    m.insert(
        Arc::from("setContentSize"),
        Value::native(move |a: &[Value]| {
            let Some(w) = window_for_root(rid) else {
                return Value::Null;
            };
            let w0 = a.first().and_then(|v| v.as_number()).unwrap_or(400.0);
            let h0 = a.get(1).and_then(|v| v.as_number()).unwrap_or(300.0);
            w.setContentSize(CGSize::new(w0, h0));
            Value::Null
        }),
    );

    let rid = root_id;
    m.insert(
        Arc::from("setMinContentSize"),
        Value::native(move |a: &[Value]| {
            let Some(w) = window_for_root(rid) else {
                return Value::Null;
            };
            let w0 = a.first().and_then(|v| v.as_number()).unwrap_or(200.0);
            let h0 = a.get(1).and_then(|v| v.as_number()).unwrap_or(150.0);
            w.setMinSize(CGSize::new(w0, h0));
            Value::Null
        }),
    );

    let rid = root_id;
    m.insert(
        Arc::from("setMaxContentSize"),
        Value::native(move |a: &[Value]| {
            let Some(w) = window_for_root(rid) else {
                return Value::Null;
            };
            let w0 = a.first().and_then(|v| v.as_number()).unwrap_or(10_000.0);
            let h0 = a.get(1).and_then(|v| v.as_number()).unwrap_or(10_000.0);
            w.setMaxSize(CGSize::new(w0, h0));
            Value::Null
        }),
    );

    let rid = root_id;
    m.insert(
        Arc::from("minimize"),
        Value::native(move |_a: &[Value]| {
            if let Some(w) = window_for_root(rid) {
                w.miniaturize(None);
            }
            Value::Null
        }),
    );

    let rid = root_id;
    m.insert(
        Arc::from("zoom"),
        Value::native(move |_a: &[Value]| {
            if let Some(w) = window_for_root(rid) {
                w.zoom(None);
            }
            Value::Null
        }),
    );

    let rid = root_id;
    m.insert(
        Arc::from("close"),
        Value::native(move |_a: &[Value]| {
            if let Some(w) = window_for_root(rid) {
                w.performClose(None);
            }
            Value::Null
        }),
    );

    let rid = root_id;
    m.insert(
        Arc::from("focus"),
        Value::native(move |_a: &[Value]| {
            if let Some(w) = window_for_root(rid) {
                w.makeKeyAndOrderFront(None);
            }
            Value::Null
        }),
    );

    let rid = root_id;
    m.insert(
        Arc::from("snapToRegion"),
        Value::native(move |a: &[Value]| {
            let region = a.first().map(|v| v.to_display_string()).unwrap_or_default();
            if let Some(w) = window_for_root(rid) {
                snap_window_to_region(w.as_ref(), region.as_str());
            }
            Value::Null
        }),
    );

    let rid = root_id;
    m.insert(
        Arc::from("toggleSidebar"),
        Value::native(move |_a: &[Value]| {
            toggle_split_sidebar(rid);
            Value::Null
        }),
    );
    let rid = root_id;
    m.insert(
        Arc::from("sidebarCollapsed"),
        Value::native(move |_a: &[Value]| {
            Value::Bool(is_split_sidebar_collapsed(rid))
        }),
    );

    Value::object(m)
}

fn effective_root_id() -> RootId {
    current_root_id().unwrap_or(LEGACY_ROOT_ID)
}

fn with_window<R>(f: impl FnOnce(&NSWindow) -> R) -> Option<R> {
    let rid = effective_root_id();
    let w = window_for_root(rid)?;
    Some(f(w.as_ref()))
}

/// Application-level helpers (not tied to the current root’s `NSWindow`).
pub fn app_object() -> Value {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    let run_el = Value::native(|_| {
        let mtm = MainThreadMarker::new().expect("app.runEventLoop needs the main thread");
        NSApplication::sharedApplication(mtm).run();
        Value::Null
    });
    let spawn = Value::native(super::session_bus::spawn_peer_process);
    let activate = Value::native(|_| {
        let mtm = MainThreadMarker::new().expect("app.activate needs the main thread");
        NSApplication::sharedApplication(mtm).activate();
        Value::Null
    });
    let mut m = ObjectMap::default();
    m.insert(Arc::from("runEventLoop"), run_el);
    m.insert(Arc::from("spawnPeer"), spawn);
    m.insert(Arc::from("activate"), activate);
    Value::object(m)
}

pub fn window_object() -> Value {
    let mut m = ObjectMap::default();

    m.insert(
        Arc::from("title"),
        Value::native(|_a: &[Value]| {
            with_window(|w| Value::String(w.title().to_string().into())).unwrap_or(Value::Null)
        }),
    );

    m.insert(
        Arc::from("setTitle"),
        Value::native(|a: &[Value]| {
            let title = a.first().map(|v| v.to_display_string()).unwrap_or_default();
            with_window(|w| {
                w.setTitle(&NSString::from_str(&title));
            });
            Value::Null
        }),
    );

    m.insert(
        Arc::from("innerWidth"),
        Value::native(|_a: &[Value]| {
            let rid = effective_root_id();
            let p = detail_metrics_ptr_for_root(rid);
            if p != 0 {
                let v: &NSView = unsafe { &*(p as *const NSView) };
                return Value::Number(v.bounds().size.width as f64);
            }
            with_window(|w| {
                let r = w.contentLayoutRect();
                Value::Number(r.size.width as f64)
            })
            .unwrap_or(Value::Number(0.0))
        }),
    );

    m.insert(
        Arc::from("innerHeight"),
        Value::native(|_a: &[Value]| {
            let rid = effective_root_id();
            let p = detail_metrics_ptr_for_root(rid);
            if p != 0 {
                let v: &NSView = unsafe { &*(p as *const NSView) };
                return Value::Number(v.bounds().size.height as f64);
            }
            with_window(|w| {
                let r = w.contentLayoutRect();
                Value::Number(r.size.height as f64)
            })
            .unwrap_or(Value::Number(0.0))
        }),
    );

    m.insert(
        Arc::from("setContentSize"),
        Value::native(|a: &[Value]| {
            let w0 = a.first().and_then(|v| v.as_number()).unwrap_or(400.0);
            let h0 = a.get(1).and_then(|v| v.as_number()).unwrap_or(300.0);
            with_window(|win| {
                win.setContentSize(CGSize::new(w0, h0));
            });
            Value::Null
        }),
    );

    m.insert(
        Arc::from("setMinContentSize"),
        Value::native(|a: &[Value]| {
            let w0 = a.first().and_then(|v| v.as_number()).unwrap_or(200.0);
            let h0 = a.get(1).and_then(|v| v.as_number()).unwrap_or(150.0);
            with_window(|win| {
                win.setMinSize(CGSize::new(w0, h0));
            });
            Value::Null
        }),
    );

    m.insert(
        Arc::from("setMaxContentSize"),
        Value::native(|a: &[Value]| {
            let w0 = a.first().and_then(|v| v.as_number()).unwrap_or(10_000.0);
            let h0 = a.get(1).and_then(|v| v.as_number()).unwrap_or(10_000.0);
            with_window(|win| {
                win.setMaxSize(CGSize::new(w0, h0));
            });
            Value::Null
        }),
    );

    m.insert(
        Arc::from("minimize"),
        Value::native(|_a: &[Value]| {
            with_window(|w| {
                w.miniaturize(None);
            });
            Value::Null
        }),
    );

    m.insert(
        Arc::from("zoom"),
        Value::native(|_a: &[Value]| {
            with_window(|w| {
                w.zoom(None);
            });
            Value::Null
        }),
    );

    m.insert(
        Arc::from("close"),
        Value::native(|_a: &[Value]| {
            with_window(|w| {
                w.performClose(None);
            });
            Value::Null
        }),
    );

    m.insert(
        Arc::from("focus"),
        Value::native(|_a: &[Value]| {
            with_window(|w| {
                w.makeKeyAndOrderFront(None);
            });
            Value::Null
        }),
    );

    m.insert(
        Arc::from("snapToRegion"),
        Value::native(|a: &[Value]| {
            let region = a.first().map(|v| v.to_display_string()).unwrap_or_default();
            with_window(|w| snap_window_to_region(w, region.as_str()));
            Value::Null
        }),
    );

    m.insert(
        Arc::from("toggleSidebar"),
        Value::native(|_a: &[Value]| {
            toggle_split_sidebar(effective_root_id());
            Value::Null
        }),
    );
    m.insert(
        Arc::from("sidebarCollapsed"),
        Value::native(|_a: &[Value]| {
            Value::Bool(is_split_sidebar_collapsed(effective_root_id()))
        }),
    );

    Value::object(m)
}
