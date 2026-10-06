//! Async on the AppKit main thread, without running Tish off it.
//!
//! On the native backend `await` blocks the calling OS thread, and Tish code must stay on the
//! thread that started it (module functions and globals live in per-thread storage). An app whose
//! UI runs on the main thread therefore can't `await` a fetch or a timer there without freezing.
//! These two calls let it keep going and be called back on the main thread instead:
//!
//! - `macos.whenSettled(promise, cb)`: waits for any Tish promise (`fetch(url)`,
//!   `response.text()`, `reader.read()`, …) on a background thread, then calls `cb(value, error)`
//!   on the main thread: `error` is null when it fulfilled, `value` null when it rejected. A value
//!   that isn't a promise is passed straight to `cb`, on the next main-queue turn.
//! - `macos.startTimers()`: drives the global `setTimeout` / `setInterval` from the main run loop
//!   (an app started with `macos.run` already has this; call it when AppKit is set up otherwise).
//!
//! Streaming a response body, for example:
//!
//! ```tish
//! fn pump(reader, onText, done) {
//!   macos.whenSettled(reader.read(), (r, err) => {
//!     if (err !== null || r.done) { done(err); return }
//!     onText(r.value)
//!     pump(reader, onText, done)
//!   })
//! }
//! macos.whenSettled(fetch(url), (res, err) => pump(res.body.getReader(), show, finish))
//! ```

use dispatch2::DispatchQueue;
use tishlang_core::Value;

/// Carries values to the main queue. Sound because a `Value::Promise` only exists when the runtime
/// is built with `http` or `promise`, both of which turn on `send-values` (`Value: Send`). The
/// callback itself is only called, cloned or dropped on the main thread.
struct ToMain<T>(T);
unsafe impl<T> Send for ToMain<T> {}

impl<T> ToMain<T> {
    fn into_inner(self) -> T {
        self.0
    }
}

fn call(cb: &Value, value: Value, error: Value) {
    if let Value::Function(f) = cb {
        let _ = f.call(&[value, error]);
    }
}

pub(crate) fn when_settled(args: &[Value]) -> Value {
    let cb = args.get(1).cloned().unwrap_or(Value::Null);
    match args.first().cloned().unwrap_or(Value::Null) {
        Value::Promise(p) => {
            let cb = ToMain(cb);
            std::thread::spawn(move || {
                let result = p.block_until_settled();
                let payload = ToMain((cb.into_inner(), result));
                DispatchQueue::main().exec_async(move || {
                    let (cb, result) = payload.into_inner();
                    match result {
                        Ok(v) => call(&cb, v, Value::Null),
                        Err(e) => call(&cb, Value::Null, e),
                    }
                });
            });
        }
        other => {
            let payload = ToMain((cb, other));
            DispatchQueue::main().exec_async(move || {
                let (cb, v) = payload.into_inner();
                call(&cb, v, Value::Null);
            });
        }
    }
    Value::Null
}
