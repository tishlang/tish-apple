//! Session-scoped messages between peer processes via `CFNotificationCenter` distributed center.

use std::cell::RefCell;
use std::ffi::c_void;
use std::process::Command;
use dispatch2::DispatchQueue;
use objc2_core_foundation::{
    CFDictionary, CFMutableDictionary, CFNotificationCenter, CFNotificationSuspensionBehavior,
    CFString,
};
use tishlang_core::{NativeFn, Value};

const NOTIFY_NAME: &str = "com.tishlang.tish-macos.session";
const USER_INFO_KEY: &str = "tish";

static OBSERVER_TOKEN: u8 = 0;

thread_local! {
    static SESSION_CACHE: RefCell<Option<String>> = RefCell::new(None);
    static MESSAGE_HANDLER: RefCell<Option<NativeFn>> = RefCell::new(None);
}

static OBSERVER_REGISTERED: std::sync::Mutex<bool> = std::sync::Mutex::new(false);

fn current_session_id() -> String {
    SESSION_CACHE.with(|c| {
        let mut b = c.borrow_mut();
        if let Some(ref s) = *b {
            return s.clone();
        }
        let s = std::env::var("TISH_MACOS_SESSION_ID").unwrap_or_else(|_| {
            let id = format!(
                "{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            );
            std::env::set_var("TISH_MACOS_SESSION_ID", &id);
            id
        });
        *b = Some(s.clone());
        s
    })
}

/// Ensure `TISH_MACOS_SESSION_ID` is set (for this process and for `spawnPeer` children).
pub fn ensure_session_id() -> String {
    current_session_id()
}

unsafe extern "C-unwind" fn distributed_callback(
    _center: *mut CFNotificationCenter,
    _observer: *mut c_void,
    _name: *const objc2_core_foundation::CFNotificationName,
    _object: *const c_void,
    user_info: *const objc2_core_foundation::CFDictionary,
) {
    if user_info.is_null() {
        return;
    }
    let dict: &objc2_core_foundation::CFDictionary<CFString, CFString> =
        unsafe { &*(user_info.cast()) };
    let key = CFString::from_static_str(USER_INFO_KEY);
    let payload = unsafe { dict.get_unchecked(&*key).and_then(|cf| cf.as_str_unchecked()) };
    let Some(payload) = payload else {
        return;
    };
    let mut lines = payload.lines();
    let session = lines.next().unwrap_or("").to_string();
    let channel = lines.next().unwrap_or("").to_string();
    let body = lines.collect::<Vec<_>>().join("\n");

    let expect = std::env::var("TISH_MACOS_SESSION_ID").unwrap_or_default();
    if session != expect {
        return;
    }

    DispatchQueue::main().exec_async(move || {
        // Clone the handler while the borrow is short — the Tish callback may re-render and call
        // `onSessionMessage`, which must `borrow_mut` the same RefCell.
        let to_run = MESSAGE_HANDLER.with(|h| h.borrow().clone());
        if let Some(f) = to_run {
            let _ = f.call(&[
                Value::String(channel.into()),
                Value::String(body.into()),
            ]);
        }
    });
}

fn ensure_observer_registered() {
    let mut g = OBSERVER_REGISTERED.lock().unwrap();
    if *g {
        return;
    }
    let Some(center) = CFNotificationCenter::distributed_center() else {
        return;
    };
    let name = CFString::from_static_str(NOTIFY_NAME);
    unsafe {
        center.add_observer(
            std::ptr::addr_of!(OBSERVER_TOKEN).cast::<c_void>(),
            Some(distributed_callback),
            Some(&name),
            std::ptr::null(),
            CFNotificationSuspensionBehavior::DeliverImmediately,
        );
    }
    *g = true;
}

/// `postSessionMessage(channel, body)` — UTF-8 strings; delivered to peers sharing `TISH_MACOS_SESSION_ID`.
pub fn post_session_message(args: &[Value]) -> Value {
    let channel = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    let body = args.get(1).map(|v| v.to_display_string()).unwrap_or_default();
    let session = current_session_id();

    let Some(center) = CFNotificationCenter::distributed_center() else {
        return Value::Null;
    };

    let payload = format!("{session}\n{channel}\n{body}");
    let dict = CFMutableDictionary::<CFString, CFString>::with_capacity(1);
    let k = CFString::from_static_str(USER_INFO_KEY);
    let v = CFString::from_str(&payload);
    dict.add(&*k, &*v);

    let name = CFString::from_static_str(NOTIFY_NAME);
    let m: &CFMutableDictionary<CFString, CFString> = dict.as_ref();
    let d: &CFDictionary<CFString, CFString> = m.as_ref();
    unsafe {
        center.post_notification(
            Some(&name),
            std::ptr::null(),
            Some(d.as_ref()),
            true,
        );
    }
    Value::Null
}

/// `onSessionMessage(handler)` — `handler(channel, body)` runs on the main queue.
pub fn on_session_message(args: &[Value]) -> Value {
    let Some(Value::Function(f)) = args.first() else {
        return Value::Null;
    };
    let f = f.clone();
    MESSAGE_HANDLER.with(|h| {
        *h.borrow_mut() = Some(f);
    });
    ensure_observer_registered();
    Value::Null
}

/// Spawn another instance of this executable with the same session id (`TISH_MACOS_CHILD=1`).
pub fn spawn_peer_process(_args: &[Value]) -> Value {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return Value::Null,
    };
    let session = current_session_id();
    let _ = Command::new(exe)
        .env("TISH_MACOS_SESSION_ID", session)
        .env("TISH_MACOS_CHILD", "1")
        .spawn();
    Value::Null
}
