//! UIKit host for Tish JSX (`tish:ios`).

#[cfg(target_os = "ios")]
mod uikit;

#[cfg(target_os = "ios")]
pub use uikit::ios_object;

#[cfg(not(target_os = "ios"))]
use std::sync::Arc;

#[cfg(not(target_os = "ios"))]
use tishlang_core::{ObjectMap, Value};

/// Non-iOS stub for `cargo check` on macOS/Linux CI.
#[cfg(not(target_os = "ios"))]
pub fn ios_object() -> Value {
    let noop = Value::native(|_a: &[Value]| Value::Null);
    let run = Value::native(|_a: &[Value]| {
        eprintln!("tish-ios: ios.run is only available on iOS.");
        Value::Null
    });
    let mut ios_inner = ObjectMap::default();
    ios_inner.insert(Arc::from("run"), run);
    let mut root = ObjectMap::default();
    root.insert(Arc::from("ios"), Value::object(ios_inner));
    root.insert(
        Arc::from("useState"),
        Value::native(tishlang_ui::runtime::native_use_state),
    );
    root.insert(
        Arc::from("useMemo"),
        Value::native(tishlang_ui::runtime::native_use_memo),
    );
    root.insert(
        Arc::from("useEffect"),
        Value::native(tishlang_ui::runtime::native_use_effect),
    );
    root.insert(
        Arc::from("useRef"),
        Value::native(tishlang_ui::runtime::native_use_ref),
    );
    root.insert(
        Arc::from("useLayoutEffect"),
        Value::native(tishlang_ui::runtime::native_use_layout_effect),
    );
    root.insert(Arc::from("Window"), noop);
    Value::object(root)
}
