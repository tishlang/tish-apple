//! macOS services that aren't UI: time zones, the dictionary, the pasteboard, files and apps
//! (NSWorkspace), folder watching (FSEvents) and URL schemes. Each is a namespace on `macos`
//! (`macos.timeZones`, `macos.dictionary`, …); callbacks run on the main thread.

mod accessibility;
mod contacts;
mod dictionary;
mod folders;
mod openurl;
mod pasteboard;
mod system;
mod sysinfo;
mod timezones;
mod workspace;

use std::sync::Arc;

use tishlang_core::{ObjectMap, Value};

pub(crate) fn s(v: &str) -> Value {
    Value::String(v.into())
}

pub(crate) fn obj(pairs: Vec<(&str, Value)>) -> Value {
    let mut m = ObjectMap::default();
    for (k, v) in pairs {
        m.insert(Arc::from(k), v);
    }
    Value::object(m)
}

pub(crate) fn arr(items: Vec<Value>) -> Value {
    Value::Array(tishlang_core::VmRef::new(items))
}

pub(crate) fn str_arg(args: &[Value], i: usize) -> String {
    match args.get(i) {
        Some(Value::String(x)) => x.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

pub(crate) fn num_arg(args: &[Value], i: usize, default: f64) -> f64 {
    match args.get(i) {
        Some(Value::Number(n)) => *n,
        _ => default,
    }
}

/// Call a Tish callback with `args`, ignoring a non-function.
pub(crate) fn call(cb: &Value, args: &[Value]) {
    if let Value::Function(f) = cb {
        let _ = f.call(args);
    }
}

fn namespace(fns: Vec<(&str, fn(&[Value]) -> Value)>) -> Value {
    obj(fns.into_iter().map(|(k, f)| (k, Value::native(f))).collect())
}

/// Add the namespaces to the `macos` object.
pub(crate) fn install(macos: &mut ObjectMap) {
    macos.insert(
        Arc::from("timeZones"),
        namespace(vec![("names", timezones::names), ("local", timezones::local), ("at", timezones::at), ("byAbbreviation", timezones::by_abbreviation)]),
    );
    macos.insert(Arc::from("dictionary"), namespace(vec![("lookup", dictionary::lookup)]));
    macos.insert(
        Arc::from("pasteboard"),
        namespace(vec![("readText", pasteboard::read_text), ("writeText", pasteboard::write_text), ("watch", pasteboard::watch)]),
    );
    macos.insert(
        Arc::from("workspace"),
        namespace(vec![
            ("open", workspace::open),
            ("reveal", workspace::reveal),
            ("trash", workspace::trash),
            ("appsFor", workspace::apps_for),
            ("openWith", workspace::open_with),
        ]),
    );
    macos.insert(
        Arc::from("system"),
        namespace(vec![
            ("lockScreen", system::t_lock),
            ("sleep", system::t_sleep),
            ("sleepDisplays", system::t_sleep_displays),
            ("screenSaver", system::t_screen_saver),
            ("restart", system::t_restart),
            ("shutDown", system::t_shut_down),
            ("logOut", system::t_log_out),
            ("emptyTrash", system::t_empty_trash),
            ("darkMode", system::t_dark_mode),
            ("setDarkMode", system::t_set_dark_mode),
            ("volume", system::t_volume),
            ("setVolume", system::t_set_volume),
            ("setMuted", system::t_set_muted),
            ("ejectAll", system::t_eject_all),
        ]),
    );
    macos.insert(
        Arc::from("apps"),
        namespace(vec![("running", system::t_running), ("act", system::t_act), ("quitAll", system::t_quit_all), ("hideAll", system::t_hide_all)]),
    );
    macos.insert(Arc::from("systemInfo"), Value::native(sysinfo::system_info));
    macos.insert(
        Arc::from("contacts"),
        namespace(vec![("status", contacts::t_status), ("request", contacts::t_request), ("query", contacts::t_search)]),
    );
    macos.insert(
        Arc::from("accessibility"),
        namespace(vec![
            ("trusted", accessibility::t_trusted),
            ("selectedText", accessibility::t_selected_text),
            ("replaceBeforeCursor", accessibility::t_replace_before_cursor),
            ("focusedWindow", accessibility::t_focused_window),
            ("setFocusedWindowFrame", accessibility::t_set_focused_window_frame),
        ]),
    );
    macos.insert(Arc::from("screens"), Value::native(accessibility::t_screens));
    macos.insert(Arc::from("watchFolders"), Value::native(folders::watch));
    macos.insert(Arc::from("onOpenUrl"), Value::native(openurl::on_open_url));
}

// ── Blocking work off the main thread ───────────────────────────────────────

thread_local! {
    /// Callbacks waiting for background work, by id (main thread only).
    static PENDING: std::cell::RefCell<std::collections::HashMap<u64, Value>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

static NEXT_PENDING: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Keep `cb` (main thread) until `deliver` answers it; returns its id.
pub(crate) fn hold(cb: Option<&Value>) -> u64 {
    let id = NEXT_PENDING.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if let Some(cb) = cb {
        PENDING.with(|p| p.borrow_mut().insert(id, cb.clone()));
    }
    id
}

/// From any thread: on the main queue, turn `data` into a value and pass it to callback `id`.
pub(crate) fn deliver<T: Send + 'static>(id: u64, data: T, make: fn(T) -> Value) {
    dispatch2::DispatchQueue::main().exec_async(move || {
        if let Some(cb) = PENDING.with(|p| p.borrow_mut().remove(&id)) {
            call(&cb, &[make(data)]);
        }
    });
}

/// Run `work` on a background thread, then `cb(make(result))` on the main thread. Only `work`'s
/// plain result crosses threads; the callback stays on the main thread.
pub(crate) fn in_background<T: Send + 'static>(cb: Option<&Value>, work: impl FnOnce() -> T + Send + 'static, make: fn(T) -> Value) {
    let id = hold(cb);
    std::thread::spawn(move || deliver(id, work(), make));
}

/// null for success, else the error message.
pub(crate) fn outcome(r: Result<(), String>) -> Value {
    match r {
        Ok(()) => Value::Null,
        Err(e) => s(&e),
    }
}
