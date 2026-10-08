//! `macos.accessibility` (needs the app in System Settings › Privacy & Security › Accessibility)
//! and `macos.screens()`.
//!
//! - `trusted(prompt)`: whether the app may use Accessibility; `prompt` asks macOS to show its
//!   permission dialog (once; later only the System Settings list)
//! - `selectedText()` -> the selected text in the focused field of any app, or null
//! - `replaceBeforeCursor(typed, text)`: replace `typed`, just before the cursor in the focused
//!   field, with `text` (select it, then set the selection) -> null, or why not; nothing changes
//!   unless the field really has `typed` there
//! - `focusedWindow()` -> the frontmost app's focused window `{ pid, app, x, y, w, h, key }` (`key`
//!   identifies the window while it exists) or `{ error }`
//! - `setFocusedWindowFrame(x, y, w, h)` -> null, or why the window didn't move
//! - `windowAction(pid, action)`: on app `pid`'s focused window, "minimize", "fullscreen"
//!   (toggles), "close" or "raise"; "unminimize" restores all its minimized windows. ->
//!   `{ count }` (windows changed) or `{ error }`, where the error reads after the app's name
//!   ("has no open window")
//! - `macos.screens()` -> `[{ visible, frame }]`, each `{ x, y, w, h }`: every display's usable
//!   area (menu bar and Dock left out) and whole frame, main display first. All in Accessibility's
//!   coordinates: origin at the main display's top left, y down.

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, MainThreadMarker};
use objc2_app_kit::{NSScreen, NSWorkspace};
use objc2_foundation::NSString;
use std::ffi::c_void;
use tishlang_core::Value;

use super::{arr, num_arg, obj, outcome, s, str_arg};

type CFTypeRef = *const c_void;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
    fn AXUIElementCreateSystemWide() -> CFTypeRef;
    fn AXUIElementCopyAttributeValue(
        element: CFTypeRef,
        attribute: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetAttributeValue(
        element: CFTypeRef,
        attribute: CFTypeRef,
        value: CFTypeRef,
    ) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: CFTypeRef,
        attribute: CFTypeRef,
        parameter: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXValueCreate(kind: u32, value: *const c_void) -> CFTypeRef;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, out: *mut c_void) -> bool;
    fn AXUIElementPerformAction(element: CFTypeRef, action: CFTypeRef) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFHash(cf: CFTypeRef) -> usize;
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFArrayGetCount(array: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> CFTypeRef;
    fn CFBooleanGetValue(boolean: CFTypeRef) -> bool;
    static kCFBooleanTrue: CFTypeRef;
    static kCFBooleanFalse: CFTypeRef;
}

const AX_POINT: u32 = 1;
const AX_SIZE: u32 = 2;
const AX_RANGE: u32 = 4;

#[repr(C)]
#[derive(Default)]
struct Pair {
    a: f64,
    b: f64,
}

/// CFRange, in UTF-16 units.
#[repr(C)]
#[derive(Default)]
struct Range {
    location: isize,
    length: isize,
}

/// A CF object we own; released on drop.
struct Owned(CFTypeRef);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}

fn copy_attr(element: CFTypeRef, name: &str) -> Option<Owned> {
    let attr = NSString::from_str(name);
    let mut out: CFTypeRef = std::ptr::null();
    let err = unsafe {
        AXUIElementCopyAttributeValue(element, Retained::as_ptr(&attr) as CFTypeRef, &mut out)
    };
    (err == 0 && !out.is_null()).then(|| Owned(out))
}

fn set_pair(element: CFTypeRef, name: &str, kind: u32, a: f64, b: f64) -> i32 {
    let attr = NSString::from_str(name);
    let p = Pair { a, b };
    let v = Owned(unsafe { AXValueCreate(kind, &p as *const Pair as *const c_void) });
    unsafe { AXUIElementSetAttributeValue(element, Retained::as_ptr(&attr) as CFTypeRef, v.0) }
}

fn get_pair(element: CFTypeRef, name: &str, kind: u32) -> Option<(f64, f64)> {
    let v = copy_attr(element, name)?;
    let mut p = Pair::default();
    unsafe { AXValueGetValue(v.0, kind, &mut p as *mut Pair as *mut c_void) }.then_some((p.a, p.b))
}

fn range_value(location: isize, length: isize) -> Owned {
    let r = Range { location, length };
    Owned(unsafe { AXValueCreate(AX_RANGE, &r as *const Range as *const c_void) })
}

fn set_range(element: CFTypeRef, location: isize, length: isize) -> i32 {
    let attr = NSString::from_str("AXSelectedTextRange");
    let v = range_value(location, length);
    unsafe { AXUIElementSetAttributeValue(element, Retained::as_ptr(&attr) as CFTypeRef, v.0) }
}

fn get_range(element: CFTypeRef) -> Option<(isize, isize)> {
    let v = copy_attr(element, "AXSelectedTextRange")?;
    let mut r = Range::default();
    unsafe { AXValueGetValue(v.0, AX_RANGE, &mut r as *mut Range as *mut c_void) }
        .then_some((r.location, r.length))
}

fn as_string(v: &Owned) -> Option<String> {
    if unsafe { CFGetTypeID(v.0) != CFStringGetTypeID() } {
        return None;
    }
    Some(unsafe { &*(v.0 as *const NSString) }.to_string())
}

/// The field's text in `location..location + length` (UTF-16 units). Asks for just that range,
/// or cuts it from the whole value for fields that do not answer range queries.
fn text_in_range(element: CFTypeRef, location: isize, length: isize) -> Option<String> {
    let attr = NSString::from_str("AXStringForRange");
    let param = range_value(location, length);
    let mut out: CFTypeRef = std::ptr::null();
    let err = unsafe {
        AXUIElementCopyParameterizedAttributeValue(
            element,
            Retained::as_ptr(&attr) as CFTypeRef,
            param.0,
            &mut out,
        )
    };
    if err == 0 && !out.is_null() {
        return as_string(&Owned(out));
    }
    let value = copy_attr(element, "AXValue")?;
    let all: Vec<u16> = as_string(&value)?.encode_utf16().collect();
    let (start, end) = (location as usize, (location + length) as usize);
    (end <= all.len()).then(|| String::from_utf16_lossy(&all[start..end]))
}

/// The selected text in the focused field of any app, or None (nothing selected, the app does
/// not expose it, or no permission).
fn selected_text() -> Option<String> {
    if !trusted(false) {
        return None;
    }
    let system = Owned(unsafe { AXUIElementCreateSystemWide() });
    let focused = copy_attr(system.0, "AXFocusedUIElement")?;
    let s = as_string(&copy_attr(focused.0, "AXSelectedText")?)?;
    (!s.is_empty()).then_some(s)
}

/// Replace `typed`, just before the cursor in the focused field of any app, with `text`: select
/// it, then set the selected text. Nothing changes unless the field really has `typed` there.
fn replace_typed(typed: &str, text: &str) -> Result<(), String> {
    if !trusted(false) {
        return Err("needs Accessibility permission".into());
    }
    let system = Owned(unsafe { AXUIElementCreateSystemWide() });
    let focused = copy_attr(system.0, "AXFocusedUIElement").ok_or("no focused text field")?;
    replace_before_cursor(focused.0, typed, text)
}

fn replace_before_cursor(field: CFTypeRef, typed: &str, text: &str) -> Result<(), String> {
    let (cursor, selected) = get_range(field).ok_or("the focused element has no text cursor")?;
    if selected != 0 {
        return Err("text is selected".into());
    }
    let n = typed.encode_utf16().count() as isize;
    let start = cursor - n;
    if start < 0 || text_in_range(field, start, n).as_deref() != Some(typed) {
        return Err(format!("the text before the cursor is not {typed}"));
    }
    if set_range(field, start, n) != 0 {
        return Err("the field did not let the app select the text".into());
    }
    let attr = NSString::from_str("AXSelectedText");
    let value = NSString::from_str(text);
    let err = unsafe {
        AXUIElementSetAttributeValue(
            field,
            Retained::as_ptr(&attr) as CFTypeRef,
            Retained::as_ptr(&value) as CFTypeRef,
        )
    };
    if err != 0 {
        set_range(field, cursor, 0);
        return Err("the field did not accept the text".into());
    }
    Ok(())
}

fn trusted(prompt: bool) -> bool {
    unsafe {
        let Some(dict_cls) = AnyClass::get(c"NSDictionary") else {
            return false;
        };
        let Some(num_cls) = AnyClass::get(c"NSNumber") else {
            return false;
        };
        let yes: Retained<AnyObject> = msg_send![num_cls, numberWithBool: prompt];
        let key = NSString::from_str("AXTrustedCheckOptionPrompt");
        let opts: Retained<AnyObject> =
            msg_send![dict_cls, dictionaryWithObject: &*yes, forKey: &*key];
        AXIsProcessTrustedWithOptions(Retained::as_ptr(&opts) as CFTypeRef)
    }
}

/// The frontmost app (not this one) and its focused or main window.
fn front_window() -> Result<(i32, String, Owned), String> {
    if !trusted(false) {
        return Err("needs Accessibility permission".into());
    }
    let app = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .ok_or("no frontmost app")?;
    let pid = app.processIdentifier();
    let name = app
        .localizedName()
        .map(|x| x.to_string())
        .unwrap_or_default();
    if pid == std::process::id() as i32 {
        return Err("no window to arrange".into());
    }
    let ax_app = Owned(unsafe { AXUIElementCreateApplication(pid) });
    let win = copy_attr(ax_app.0, "AXFocusedWindow")
        .or_else(|| copy_attr(ax_app.0, "AXMainWindow"))
        .ok_or_else(|| format!("{name} has no window"))?;
    Ok((pid, name, win))
}

fn set_bool(element: CFTypeRef, name: &str, on: bool) -> bool {
    let attr = NSString::from_str(name);
    let v = unsafe {
        if on {
            kCFBooleanTrue
        } else {
            kCFBooleanFalse
        }
    };
    unsafe { AXUIElementSetAttributeValue(element, Retained::as_ptr(&attr) as CFTypeRef, v) == 0 }
}

fn get_bool(element: CFTypeRef, name: &str) -> bool {
    copy_attr(element, name).is_some_and(|v| unsafe { CFBooleanGetValue(v.0) })
}

fn press(element: CFTypeRef, action: &str) -> bool {
    let a = NSString::from_str(action);
    unsafe { AXUIElementPerformAction(element, Retained::as_ptr(&a) as CFTypeRef) == 0 }
}

/// The window states `setFocusedWindowFrame` doesn't reach. Ok: how many windows changed.
fn window_action(pid: i32, action: &str) -> Result<usize, String> {
    if !trusted(false) {
        return Err("needs Accessibility permission".into());
    }
    let app = Owned(unsafe { AXUIElementCreateApplication(pid) });
    if action == "unminimize" {
        let windows = copy_attr(app.0, "AXWindows").ok_or("has no windows")?;
        let n = (0..unsafe { CFArrayGetCount(windows.0) })
            .map(|i| unsafe { CFArrayGetValueAtIndex(windows.0, i) })
            .filter(|&w| get_bool(w, "AXMinimized") && set_bool(w, "AXMinimized", false))
            .count();
        return if n == 0 {
            Err("has no minimized windows".into())
        } else {
            Ok(n)
        };
    }
    let w = copy_attr(app.0, "AXFocusedWindow")
        .or_else(|| copy_attr(app.0, "AXMainWindow"))
        .ok_or("has no open window")?;
    let done = match action {
        "minimize" => set_bool(w.0, "AXMinimized", true),
        "fullscreen" => set_bool(w.0, "AXFullScreen", !get_bool(w.0, "AXFullScreen")),
        "close" => copy_attr(w.0, "AXCloseButton").is_some_and(|b| press(b.0, "AXPress")),
        "raise" => press(w.0, "AXRaise"),
        _ => return Err(format!("unknown window action `{action}`")),
    };
    if done {
        Ok(1)
    } else {
        Err(format!("would not {action}"))
    }
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Value {
    obj(vec![
        ("x", Value::Number(x)),
        ("y", Value::Number(y)),
        ("w", Value::Number(w)),
        ("h", Value::Number(h)),
    ])
}

pub(super) fn t_trusted(args: &[Value]) -> Value {
    Value::Bool(trusted(matches!(args.first(), Some(Value::Bool(true)))))
}

pub(super) fn t_selected_text(_a: &[Value]) -> Value {
    selected_text().map_or(Value::Null, |t| s(&t))
}

pub(super) fn t_replace_before_cursor(args: &[Value]) -> Value {
    outcome(replace_typed(&str_arg(args, 0), &str_arg(args, 1)))
}

pub(super) fn t_focused_window(_a: &[Value]) -> Value {
    let r = (|| {
        let (pid, name, win) = front_window()?;
        let (x, y) =
            get_pair(win.0, "AXPosition", AX_POINT).ok_or("cannot read the window position")?;
        let (w, h) = get_pair(win.0, "AXSize", AX_SIZE).ok_or("cannot read the window size")?;
        let key = unsafe { CFHash(win.0) } as f64;
        Ok::<Value, String>(obj(vec![
            ("pid", Value::Number(pid as f64)),
            ("app", s(&name)),
            ("x", Value::Number(x)),
            ("y", Value::Number(y)),
            ("w", Value::Number(w)),
            ("h", Value::Number(h)),
            ("key", Value::Number(key)),
        ]))
    })();
    r.unwrap_or_else(|e| obj(vec![("error", s(&e))]))
}

pub(super) fn t_set_focused_window_frame(args: &[Value]) -> Value {
    let (x, y, w, h) = (
        num_arg(args, 0, 0.0),
        num_arg(args, 1, 0.0),
        num_arg(args, 2, 0.0),
        num_arg(args, 3, 0.0),
    );
    outcome((|| {
        let (_, name, win) = front_window()?;
        // Size first so the position fits on the new display, then size again: some apps clamp the
        // size to the display the window was on.
        set_pair(win.0, "AXSize", AX_SIZE, w, h);
        let err = set_pair(win.0, "AXPosition", AX_POINT, x, y);
        set_pair(win.0, "AXSize", AX_SIZE, w, h);
        if err != 0 {
            return Err(format!("{name} did not let its window move"));
        }
        Ok(())
    })())
}

pub(super) fn t_window_action(args: &[Value]) -> Value {
    match window_action(num_arg(args, 0, 0.0) as i32, &str_arg(args, 1)) {
        Ok(n) => obj(vec![("count", Value::Number(n as f64))]),
        Err(e) => obj(vec![("error", s(&e))]),
    }
}

pub(super) fn t_screens(_a: &[Value]) -> Value {
    let Some(mtm) = MainThreadMarker::new() else {
        return arr(Vec::new());
    };
    let all = NSScreen::screens(mtm);
    let Some(primary) = all.iter().next() else {
        return arr(Vec::new());
    };
    let top = primary.frame().size.height;
    let flip = |r: objc2_foundation::NSRect| {
        rect(
            r.origin.x,
            top - (r.origin.y + r.size.height),
            r.size.width,
            r.size.height,
        )
    };
    arr(all
        .iter()
        .map(|sc| {
            obj(vec![
                ("visible", flip(sc.visibleFrame())),
                ("frame", flip(sc.frame())),
            ])
        })
        .collect())
}
