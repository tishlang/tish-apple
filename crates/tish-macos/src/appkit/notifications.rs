//! `UNUserNotificationCenter` helpers for `macos.notification*` and desktop `notification.*`
//! local invoke (no Tauri AppHandle).
//!
//! Bare binaries (e.g. `examples/hybrid/dist/hybrid-shell-apple`) are not `.app` bundles —
//! `+[UNUserNotificationCenter currentNotificationCenter]` throws
//! `NSInternalInconsistencyException` (`bundleProxyForCurrentProcess is nil`). We catch that
//! and fall back to an `NSAlert` so Notify still works in dev shells.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicI8, Ordering};
use std::sync::Once;

use block2::RcBlock;
use core::ptr::NonNull;
use objc2::exception::catch;
use objc2::MainThreadMarker;
use objc2_app_kit::NSAlert;
use objc2_foundation::{NSBundle, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent,
    UNNotificationRequest, UNNotificationSettings, UNUserNotificationCenter,
};
use tishlang_core::{ObjectMap, Value};

/// Cached auth: -1 unknown, 0 prompt, 1 denied, 2 granted, 3 unavailable (no .app bundle).
static AUTH_CACHE: AtomicI8 = AtomicI8::new(-1);
/// -1 unknown, 0 no UN center, 1 yes.
static UN_USABLE: AtomicI8 = AtomicI8::new(-1);
static REFRESH_ONCE: Once = Once::new();

fn status_to_cache(status: UNAuthorizationStatus) -> i8 {
    match status {
        UNAuthorizationStatus::Denied => 1,
        UNAuthorizationStatus::Authorized
        | UNAuthorizationStatus::Provisional
        | UNAuthorizationStatus::Ephemeral => 2,
        _ => 0,
    }
}

fn cache_to_state(v: i8) -> &'static str {
    match v {
        1 => "denied",
        2 => "granted",
        3 => "unavailable",
        _ => "prompt",
    }
}

/// `UNUserNotificationCenter` needs a real app bundle; probe once (catch ObjC exception).
fn user_notifications_usable() -> bool {
    match UN_USABLE.load(Ordering::Relaxed) {
        0 => false,
        1 => true,
        _ => {
            // Fast reject: no bundle id ⇒ not a packaged app.
            if NSBundle::mainBundle().bundleIdentifier().is_none() {
                UN_USABLE.store(0, Ordering::Relaxed);
                AUTH_CACHE.store(3, Ordering::Relaxed);
                return false;
            }
            let ok = catch(AssertUnwindSafe(|| {
                let _ = UNUserNotificationCenter::currentNotificationCenter();
            }))
            .is_ok();
            UN_USABLE.store(if ok { 1 } else { 0 }, Ordering::Relaxed);
            if !ok {
                AUTH_CACHE.store(3, Ordering::Relaxed);
            }
            ok
        }
    }
}

fn refresh_settings_async() {
    if !user_notifications_usable() {
        return;
    }
    let Ok(center) = catch(AssertUnwindSafe(|| {
        UNUserNotificationCenter::currentNotificationCenter()
    })) else {
        UN_USABLE.store(0, Ordering::Relaxed);
        AUTH_CACHE.store(3, Ordering::Relaxed);
        return;
    };
    let block = RcBlock::new(|settings: NonNull<UNNotificationSettings>| {
        let status = unsafe { settings.as_ref() }.authorizationStatus();
        AUTH_CACHE.store(status_to_cache(status), Ordering::Relaxed);
    });
    center.getNotificationSettingsWithCompletionHandler(&block);
}

/// Kick a settings refresh (idempotent first call) and return the broker-shaped state string.
pub fn permission_state() -> &'static str {
    if !user_notifications_usable() {
        return "unavailable";
    }
    REFRESH_ONCE.call_once(|| {
        refresh_settings_async();
    });
    if AUTH_CACHE.load(Ordering::Relaxed) >= 0 {
        refresh_settings_async();
    }
    cache_to_state(AUTH_CACHE.load(Ordering::Relaxed))
}

/// Request alert/sound/badge authorization (async). Returns current/cached state immediately.
pub fn request_permission() -> &'static str {
    if !user_notifications_usable() {
        return "unavailable";
    }
    let Ok(center) = catch(AssertUnwindSafe(|| {
        UNUserNotificationCenter::currentNotificationCenter()
    })) else {
        UN_USABLE.store(0, Ordering::Relaxed);
        AUTH_CACHE.store(3, Ordering::Relaxed);
        return "unavailable";
    };
    let opts = UNAuthorizationOptions::Alert
        | UNAuthorizationOptions::Sound
        | UNAuthorizationOptions::Badge;
    let block = RcBlock::new(|granted: objc2::runtime::Bool, _err: *mut objc2_foundation::NSError| {
        AUTH_CACHE.store(if granted.as_bool() { 2 } else { 1 }, Ordering::Relaxed);
    });
    center.requestAuthorizationWithOptions_completionHandler(opts, &block);
    let cached = AUTH_CACHE.load(Ordering::Relaxed);
    if cached < 0 {
        "prompt"
    } else {
        cache_to_state(cached)
    }
}

/// Dev / bare-binary fallback: show an AppKit alert instead of a system banner.
fn show_alert_fallback(title: &str, body: &str) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or_else(|| {
        "notification fallback requires the main thread".to_string()
    })?;
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    if !body.is_empty() {
        alert.setInformativeText(&NSString::from_str(body));
    }
    alert.runModal();
    Ok(())
}

/// Post a local notification banner. Uses a nil trigger (immediate delivery).
/// When UNUserNotificationCenter is unavailable (bare binary), falls back to NSAlert.
pub fn show(title: &str, body: &str) -> Result<(), String> {
    if !user_notifications_usable() {
        return show_alert_fallback(title, body);
    }

    if AUTH_CACHE.load(Ordering::Relaxed) < 0 {
        let _ = request_permission();
    }
    if AUTH_CACHE.load(Ordering::Relaxed) == 1 {
        return Err(
            "notifications denied — enable them in System Settings → Notifications".into(),
        );
    }
    if AUTH_CACHE.load(Ordering::Relaxed) == 3 {
        return show_alert_fallback(title, body);
    }

    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    if !body.is_empty() {
        content.setBody(&NSString::from_str(body));
    }

    let id = NSString::from_str(&format!(
        "tish-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    let request =
        UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);

    let Ok(center) = catch(AssertUnwindSafe(|| {
        UNUserNotificationCenter::currentNotificationCenter()
    })) else {
        UN_USABLE.store(0, Ordering::Relaxed);
        AUTH_CACHE.store(3, Ordering::Relaxed);
        return show_alert_fallback(title, body);
    };
    let block = RcBlock::new(|err: *mut objc2_foundation::NSError| {
        if !err.is_null() {
            let e = unsafe { &*err };
            eprintln!(
                "tish-macos: addNotificationRequest failed: {}",
                e.localizedDescription()
            );
        }
    });
    center.addNotificationRequest_withCompletionHandler(&request, Some(&block));
    Ok(())
}

pub fn native_permission_state(_a: &[Value]) -> Value {
    let mut o = ObjectMap::default();
    o.insert(
        std::sync::Arc::from("state"),
        Value::String(permission_state().into()),
    );
    Value::object(o)
}

pub fn native_request_permission(_a: &[Value]) -> Value {
    let mut o = ObjectMap::default();
    o.insert(
        std::sync::Arc::from("state"),
        Value::String(request_permission().into()),
    );
    Value::object(o)
}

pub fn native_show(a: &[Value]) -> Value {
    let title = a
        .first()
        .map(|v| v.to_display_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Tish".into());
    let body = a.get(1).map(|v| v.to_display_string()).unwrap_or_default();
    match show(&title, &body) {
        Ok(()) => {
            let mut o = ObjectMap::default();
            o.insert(std::sync::Arc::from("ok"), Value::Bool(true));
            Value::object(o)
        }
        Err(e) => Value::String(e.into()),
    }
}
