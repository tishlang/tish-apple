//! UIKit entry (`tish:ios`).

mod build;
mod host;
mod router;

use std::sync::Arc;

use host::IosHost;
use tishlang_core::{ObjectMap, Value};
use tishlang_ui::runtime::{
    install_host_for_root, native_create_root, LEGACY_ROOT_ID,
};

pub fn ios_run(args: &[Value]) -> Value {
    let app_fn = match args.first() {
        Some(f) => f.clone(),
        None => return Value::Null,
    };

    let mtm = objc2::MainThreadMarker::new().expect("ios.run must run on the main thread");
    install_host_for_root(LEGACY_ROOT_ID, Box::new(IosHost::new(mtm, LEGACY_ROOT_ID)));
    let root_obj = native_create_root(&[Value::Null]);
    if let Value::Object(obj) = root_obj {
        if let Some(Value::Function(render)) = obj.borrow().strings.get("render") {
            render(&[app_fn]);
        }
    }
    Value::Null
}

pub fn ios_object() -> Value {
    let run = Value::native(ios_run);
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
    Value::object(root)
}
