//! `macos.icons`: images registered under a name, so `<image src={name}>` (and `macos.statusItem`)
//! can show them.
//!
//! - `file(path)` -> the Finder icon of a file, folder or app. Names come from a fixed ring of 512,
//!   so file results can't grow the registry without bound. macOS loads these icons lazily (an
//!   image view would keep its placeholder), so each is drawn once offscreen, one per main-loop
//!   turn; `onLoaded(cb)` calls `cb()` when that queue runs dry, for a redraw.
//! - `symbol(name)` -> an SF Symbol, or "" when there's no such symbol
//! - `image(path, template, reload)` -> the image file at `path`, or "" when it isn't one.
//!   `template` draws it in its view's tint, like a symbol; `reload` reads it again (it changed).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::path::Path;

use dispatch2::{DispatchQueue, DispatchTime};
use objc2::rc::Retained;
use objc2::AllocAnyThread;
use objc2_app_kit::{NSImage, NSWorkspace};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use tishlang_core::Value;

use super::{call, s, str_arg};

const SLOTS: usize = 512;

thread_local! {
    static BY_PATH: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static RING: RefCell<(usize, Vec<String>)> = const { RefCell::new((0, Vec::new())) };
    static WARM_QUEUE: RefCell<VecDeque<Retained<NSImage>>> = RefCell::new(VecDeque::new());
    static WARMING: Cell<bool> = const { Cell::new(false) };
    static ON_LOADED: RefCell<Option<Value>> = const { RefCell::new(None) };
}

fn flag(args: &[Value], i: usize) -> bool {
    matches!(args.get(i), Some(Value::Bool(true)))
}

pub(super) fn file(args: &[Value]) -> Value {
    let path = str_arg(args, 0);
    if let Some(n) = BY_PATH.with(|m| m.borrow().get(&path).cloned()) {
        return s(&n);
    }
    let slot = RING.with(|r| {
        let mut r = r.borrow_mut();
        let slot = r.0 % SLOTS;
        r.0 += 1;
        if slot < r.1.len() {
            let evicted = std::mem::replace(&mut r.1[slot], path.clone());
            BY_PATH.with(|m| m.borrow_mut().remove(&evicted));
        } else {
            r.1.push(path.clone());
        }
        slot
    });
    let name = format!("tish-icon-{slot}");
    let ns_name = NSString::from_str(&name);
    if let Some(old) = NSImage::imageNamed(&ns_name) {
        old.setName(None);
    }
    let img = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&path));
    img.setName(Some(&ns_name));
    BY_PATH.with(|m| m.borrow_mut().insert(path, name.clone()));
    warm_later(img);
    s(&name)
}

pub(super) fn on_loaded(args: &[Value]) -> Value {
    ON_LOADED.with(|c| *c.borrow_mut() = args.first().cloned());
    Value::Null
}

fn warm_later(img: Retained<NSImage>) {
    WARM_QUEUE.with(|q| q.borrow_mut().push_back(img));
    if !WARMING.with(|w| w.replace(true)) {
        schedule_warm();
    }
}

fn schedule_warm() {
    let when =
        DispatchTime::try_from(std::time::Duration::from_millis(1)).unwrap_or(DispatchTime::NOW);
    let _ = DispatchQueue::main().after(when, warm_next);
}

fn warm_next() {
    let Some(img) = WARM_QUEUE.with(|q| q.borrow_mut().pop_front()) else {
        WARMING.with(|w| w.set(false));
        if let Some(cb) = ON_LOADED.with(|c| c.borrow().clone()) {
            call(&cb, &[]);
        }
        return;
    };
    let side = NSSize::new(128.0, 128.0);
    let canvas = NSImage::initWithSize(NSImage::alloc(), side);
    #[allow(deprecated)]
    {
        canvas.lockFocus();
        img.drawInRect(NSRect::new(NSPoint::new(0.0, 0.0), side));
        canvas.unlockFocus();
    }
    schedule_warm();
}

pub(super) fn symbol(args: &[Value]) -> Value {
    let sym = str_arg(args, 0);
    let name = format!("tish-symbol-{sym}");
    let ns_name = NSString::from_str(&name);
    if NSImage::imageNamed(&ns_name).is_none() {
        let desc = NSString::from_str(&sym);
        match NSImage::imageWithSystemSymbolName_accessibilityDescription(&desc, Some(&desc)) {
            Some(img) => {
                img.setName(Some(&ns_name));
            }
            None => return s(""),
        }
    }
    s(&name)
}

/// FNV-1a, so a path gets a short stable image name.
fn fnv(x: &str) -> u64 {
    x.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

/// Make the image in `path` the one named `name`. False when it isn't a readable image.
fn register(name: &str, path: &Path) -> bool {
    let file = NSString::from_str(&path.to_string_lossy());
    let Some(img) = NSImage::initWithContentsOfFile(NSImage::alloc(), &file) else {
        return false;
    };
    if !img.isValid() {
        return false;
    }
    let ns_name = NSString::from_str(name);
    if let Some(old) = NSImage::imageNamed(&ns_name) {
        old.setName(None);
    }
    img.setName(Some(&ns_name))
}

pub(super) fn image(args: &[Value]) -> Value {
    let path = str_arg(args, 0);
    let (template, reload) = (flag(args, 1), flag(args, 2));
    let name = format!(
        "tish-file-{:016x}{}",
        fnv(&path),
        if template { "-t" } else { "" }
    );
    let ns_name = NSString::from_str(&name);
    if (reload || NSImage::imageNamed(&ns_name).is_none()) && !register(&name, Path::new(&path)) {
        return s("");
    }
    if let Some(img) = NSImage::imageNamed(&ns_name) {
        img.setTemplate(template);
    }
    s(&name)
}
