//! `macos.watchFolders(paths, latency, cb)`: `cb()` on the main thread after changes anywhere
//! under `paths` (recursively), coalesced over `latency` seconds (FSEvents). Folders that don't
//! exist are skipped; false when none do. Each call adds a watch; they last for the process.

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr;

use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSString};
use tishlang_core::Value;

use super::{call, num_arg};

type FSEventStreamRef = *mut c_void;
type Callback =
    extern "C" fn(FSEventStreamRef, *mut c_void, usize, *mut c_void, *const u32, *const u64);

#[repr(C)]
struct FSEventStreamContext {
    version: isize,
    info: *mut c_void,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
}

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn FSEventStreamCreate(
        alloc: *const c_void,
        callback: Callback,
        context: *const FSEventStreamContext,
        paths: *const c_void,
        since_when: u64,
        latency: f64,
        flags: u32,
    ) -> FSEventStreamRef;
    fn FSEventStreamSetDispatchQueue(stream: FSEventStreamRef, queue: *const c_void);
    fn FSEventStreamStart(stream: FSEventStreamRef) -> u8;
}

const SINCE_NOW: u64 = u64::MAX;

thread_local! {
    /// The callback of each watch, by the index passed to FSEvents as `info`.
    static CALLBACKS: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
}

extern "C" fn on_events(
    _s: FSEventStreamRef,
    info: *mut c_void,
    _n: usize,
    _paths: *mut c_void,
    _flags: *const u32,
    _ids: *const u64,
) {
    // Delivered on the main queue (FSEventStreamSetDispatchQueue below), where CALLBACKS lives.
    let cb = CALLBACKS.with(|c| c.borrow().get(info as usize).cloned());
    if let Some(cb) = cb {
        call(&cb, &[]);
    }
}

pub(super) fn watch(args: &[Value]) -> Value {
    let dirs: Vec<String> = match args.first() {
        Some(Value::Array(a)) => a
            .borrow()
            .iter()
            .filter_map(|v| {
                if let Value::String(x) = v {
                    Some(x.to_string())
                } else {
                    None
                }
            })
            .collect(),
        Some(Value::String(x)) => vec![x.to_string()],
        _ => Vec::new(),
    };
    let latency = num_arg(args, 1, 1.0).max(0.0);
    let cb = args.get(2).cloned().unwrap_or(Value::Null);
    let existing: Vec<Retained<NSString>> = dirs
        .iter()
        .filter(|d| std::path::Path::new(d).is_dir())
        .map(|d| NSString::from_str(d))
        .collect();
    if existing.is_empty() {
        return Value::Bool(false);
    }
    let refs: Vec<&NSString> = existing.iter().map(|x| &**x).collect();
    let paths = NSArray::from_slice(&refs);
    let index = CALLBACKS.with(|c| {
        let mut c = c.borrow_mut();
        c.push(cb);
        c.len() - 1
    });
    unsafe {
        let ctx = FSEventStreamContext {
            version: 0,
            info: index as *mut c_void,
            retain: ptr::null(),
            release: ptr::null(),
            copy_description: ptr::null(),
        };
        let stream = FSEventStreamCreate(
            ptr::null(),
            on_events,
            &ctx,
            Retained::as_ptr(&paths) as *const c_void,
            SINCE_NOW,
            latency,
            0,
        );
        if stream.is_null() {
            return Value::Bool(false);
        }
        FSEventStreamSetDispatchQueue(
            stream,
            DispatchQueue::main() as *const DispatchQueue as *const c_void,
        );
        Value::Bool(FSEventStreamStart(stream) != 0)
    }
}
