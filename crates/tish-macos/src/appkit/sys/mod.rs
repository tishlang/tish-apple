//! macOS services that aren't UI: time zones, the dictionary, the pasteboard, files and apps
//! (NSWorkspace), folder watching (FSEvents) and URL schemes. Each is a namespace on `macos`
//! (`macos.timeZones`, `macos.dictionary`, …); callbacks run on the main thread.

mod dictionary;
mod folders;
mod openurl;
mod pasteboard;
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
    macos.insert(Arc::from("watchFolders"), Value::native(folders::watch));
    macos.insert(Arc::from("onOpenUrl"), Value::native(openurl::on_open_url));
}
