//! `UNUserNotificationCenter` helpers for `macos.notification*` and desktop `notification.*`
//! local invoke (no Tauri AppHandle).

use std::sync::atomic::{AtomicI8, Ordering};
use std::sync::Once;

use block2::RcBlock;
use core::ptr::NonNull;
use objc2_foundation::NSString;
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent,
    UNNotificationRequest, UNNotificationSettings, UNUserNotificationCenter,
};
use tishlang_core::{ObjectMap, Value};

/// Cached auth: -1 unknown, 0 prompt, 1 denied, 2 granted.
static AUTH_CACHE: AtomicI8 = AtomicI8::new(-1);
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
        _ => "prompt",
    }
}

fn refresh_settings_async() {
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let block = RcBlock::new(|settings: NonNull<UNNotificationSettings>| {
        let status = unsafe { settings.as_ref() }.authorizationStatus();
        AUTH_CACHE.store(status_to_cache(status), Ordering::Relaxed);
    });
    center.getNotificationSettingsWithCompletionHandler(&block);
}

/// Kick a settings refresh (idempotent first call) and return the broker-shaped state string.
pub fn permission_state() -> &'static str {
    REFRESH_ONCE.call_once(|| {
        refresh_settings_async();
    });
    // Opportunistic refresh when already known (stale after System Settings change).
    if AUTH_CACHE.load(Ordering::Relaxed) >= 0 {
        refresh_settings_async();
    }
    cache_to_state(AUTH_CACHE.load(Ordering::Relaxed))
}

/// Request alert/sound/badge authorization (async). Returns current/cached state immediately.
pub fn request_permission() -> &'static str {
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let opts = UNAuthorizationOptions::Alert
        | UNAuthorizationOptions::Sound
        | UNAuthorizationOptions::Badge;
    let block = RcBlock::new(|granted: objc2::runtime::Bool, _err: *mut objc2_foundation::NSError| {
        AUTH_CACHE.store(if granted.as_bool() { 2 } else { 1 }, Ordering::Relaxed);
    });
    center.requestAuthorizationWithOptions_completionHandler(opts, &block);
    // Until the completion fires, surface prompt (or last known).
    let cached = AUTH_CACHE.load(Ordering::Relaxed);
    if cached < 0 {
        "prompt"
    } else {
        cache_to_state(cached)
    }
}

/// Post a local notification banner. Uses a nil trigger (immediate delivery).
pub fn show(title: &str, body: &str) -> Result<(), String> {
    // Ensure we have asked at least once; show still attempts if previously granted.
    if AUTH_CACHE.load(Ordering::Relaxed) < 0 {
        let _ = request_permission();
    }
    if AUTH_CACHE.load(Ordering::Relaxed) == 1 {
        return Err(
            "notifications denied — enable them in System Settings → Notifications".into(),
        );
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

    let center = UNUserNotificationCenter::currentNotificationCenter();
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
