//! `UNUserNotificationCenter` for iOS `notification.*` broker caps.

use serde_json::{json, Value};

#[cfg(target_os = "ios")]
mod inner {
    use std::sync::atomic::{AtomicI8, Ordering};
    use std::sync::Once;

    use block2::RcBlock;
    use core::ptr::NonNull;
    use objc2_foundation::NSString;
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent,
        UNNotificationRequest, UNNotificationSettings, UNUserNotificationCenter,
    };

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

    pub fn permission_state() -> &'static str {
        REFRESH_ONCE.call_once(|| {
            refresh_settings_async();
        });
        cache_to_state(AUTH_CACHE.load(Ordering::Relaxed))
    }

    pub fn request_permission() -> &'static str {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let opts = UNAuthorizationOptions::Alert
            | UNAuthorizationOptions::Sound
            | UNAuthorizationOptions::Badge;
        let block =
            RcBlock::new(|granted: objc2::runtime::Bool, _err: *mut objc2_foundation::NSError| {
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

    pub fn show(title: &str, body: &str) -> Result<(), String> {
        if AUTH_CACHE.load(Ordering::Relaxed) < 0 {
            let _ = request_permission();
        }
        if AUTH_CACHE.load(Ordering::Relaxed) == 1 {
            return Err("notifications denied".into());
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
                    "tish-ios: addNotificationRequest failed: {}",
                    e.localizedDescription()
                );
            }
        });
        center.addNotificationRequest_withCompletionHandler(&request, Some(&block));
        Ok(())
    }
}

pub fn invoke(cmd: &str, args: &Value) -> Result<Value, String> {
    match cmd {
        "notification.permissionState" => {
            #[cfg(target_os = "ios")]
            {
                return Ok(json!({ "state": inner::permission_state() }));
            }
            #[cfg(not(target_os = "ios"))]
            {
                let _ = args;
                Ok(json!({ "state": "prompt" }))
            }
        }
        "notification.requestPermission" => {
            #[cfg(target_os = "ios")]
            {
                return Ok(json!({ "state": inner::request_permission() }));
            }
            #[cfg(not(target_os = "ios"))]
            {
                Ok(json!({ "state": "prompt" }))
            }
        }
        "notification.show" => {
            let title = args
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Tish");
            let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
            #[cfg(target_os = "ios")]
            {
                inner::show(title, body)?;
                return Ok(json!({ "ok": true, "title": title, "body": body }));
            }
            #[cfg(not(target_os = "ios"))]
            {
                let _ = (title, body);
                Err("notifications only available on iOS".into())
            }
        }
        _ => Ok(tish_broker::unsupported_on("notification", "ios")),
    }
}
