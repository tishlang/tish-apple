//! `macos.system` and `macos.apps`: system actions and the apps in the Dock.
//!
//! `macos.system` (each action returns null, or why it failed):
//! - `lockScreen()`, `sleep()`, `sleepDisplays()`, `screenSaver()`
//! - `restart()`, `shutDown()`, `logOut()` (Apple Events to loginwindow, which asks the user)
//! - `emptyTrash(cb)`: `cb(error)` once Finder has emptied it (macOS asks once to let the app
//!   control Finder)
//! - `darkMode()` -> true/false (null when it can't tell), `setDarkMode(on)`
//! - `volume()` -> `{ level, muted }` (0–100) or `{ error }`; `setVolume(percent)`, `setMuted(on)`
//! - `ejectAll(cb)`: `cb({ ejected, failed })`, names and "name: why"
//!
//! `macos.apps`:
//! - `running()` -> `[{ pid, name, path, bundleId, active, hidden, memory }]`, most memory first,
//!   `memory` being the footprint in bytes (what Activity Monitor shows)
//! - `act(pid, action)`: `switch`, `hide`, `unhide`, `quit` or `force-quit` -> `{ ok, message, error }`
//! - `quitAll()` (all but Finder), `hideAll()` -> how many apps

use std::ffi::{c_char, c_void, CString};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::msg_send;
use objc2_app_kit::{NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace};
use objc2_foundation::{NSArray, NSString, NSURL};

extern "C" {
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut c_void) -> i32;
}

const RTLD_LAZY: i32 = 1;

fn symbol(lib: &str, name: &str) -> Option<*mut c_void> {
    let (l, n) = (CString::new(lib).ok()?, CString::new(name).ok()?);
    unsafe {
        let h = dlopen(l.as_ptr(), RTLD_LAZY);
        if h.is_null() {
            return None;
        }
        let s = dlsym(h, n.as_ptr());
        (!s.is_null()).then_some(s)
    }
}

pub fn lock_screen() -> Result<(), String> {
    let f = symbol("/System/Library/PrivateFrameworks/login.framework/Versions/Current/login", "SACLockScreenImmediate")
        .ok_or("cannot find SACLockScreenImmediate")?;
    let f: extern "C" fn() -> i32 = unsafe { std::mem::transmute(f) };
    f();
    Ok(())
}

fn pmset(arg: &str) -> Result<(), String> {
    let st = std::process::Command::new("/usr/bin/pmset").arg(arg).status().map_err(|e| e.to_string())?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("pmset {arg} failed"))
    }
}

pub fn sleep() -> Result<(), String> {
    pmset("sleepnow")
}

pub fn sleep_displays() -> Result<(), String> {
    pmset("displaysleepnow")
}

const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | (s[3] as u32)
}

/// Send Apple Event `class`/`id` to the app `bundle`. `wait` waits up to that many seconds for the
/// reply; 0 sends without waiting.
fn apple_event(bundle: &str, class: &[u8; 4], id: &[u8; 4], wait: f64) -> Result<(), String> {
    let cls = AnyClass::get(c"NSAppleEventDescriptor").ok_or("no NSAppleEventDescriptor")?;
    let bundle = NSString::from_str(bundle);
    unsafe {
        let target: Option<Retained<AnyObject>> = msg_send![cls, descriptorWithBundleIdentifier: &*bundle];
        let target = target.ok_or("bad target")?;
        let ev: Option<Retained<AnyObject>> = msg_send![
            cls,
            appleEventWithEventClass: fourcc(class),
            eventID: fourcc(id),
            targetDescriptor: &*target,
            returnID: -1i16,
            transactionID: 0i32
        ];
        let ev = ev.ok_or("cannot create the Apple Event")?;
        // kAENoReply = 1, kAEWaitReply = 3
        let options: usize = if wait > 0.0 { 3 } else { 1 };
        let mut err: *mut AnyObject = std::ptr::null_mut();
        let reply: Option<Retained<AnyObject>> = msg_send![&*ev, sendEventWithOptions: options, timeout: wait.max(1.0), error: &mut err];
        if reply.is_none() && !err.is_null() {
            let desc: Retained<NSString> = msg_send![err, localizedDescription];
            return Err(desc.to_string());
        }
    }
    Ok(())
}

pub fn restart() -> Result<(), String> {
    apple_event("com.apple.loginwindow", b"aevt", b"rest", 0.0)
}

pub fn shut_down() -> Result<(), String> {
    apple_event("com.apple.loginwindow", b"aevt", b"shut", 0.0)
}

pub fn log_out() -> Result<(), String> {
    apple_event("com.apple.loginwindow", b"aevt", b"rlgo", 0.0)
}

/// Asks Finder (macOS asks the user once to allow the app to control Finder). Blocks: worker thread.
pub fn empty_trash() -> Result<(), String> {
    apple_event("com.apple.finder", b"fndr", b"empt", 60.0)
}

pub fn screen_saver() -> bool {
    let url = NSURL::fileURLWithPath(&NSString::from_str("/System/Library/CoreServices/ScreenSaverEngine.app"));
    NSWorkspace::sharedWorkspace().openURL(&url)
}

// ── Appearance ──────────────────────────────────────────────────────────────

const SKYLIGHT: &str = "/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight";

pub fn dark_mode() -> Option<bool> {
    let f = symbol(SKYLIGHT, "SLSGetAppearanceThemeLegacy")?;
    let f: extern "C" fn() -> bool = unsafe { std::mem::transmute(f) };
    Some(f())
}

pub fn set_dark_mode(on: bool) -> Result<(), String> {
    let f = symbol(SKYLIGHT, "SLSSetAppearanceThemeLegacy").ok_or("cannot change the appearance on this macOS")?;
    let f: extern "C" fn(bool) = unsafe { std::mem::transmute(f) };
    f(on);
    Ok(())
}

// ── Sound ───────────────────────────────────────────────────────────────────

#[repr(C)]
struct PropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectGetPropertyData(obj: u32, addr: *const PropertyAddress, qsize: u32, q: *const c_void, size: *mut u32, data: *mut c_void) -> i32;
    fn AudioObjectSetPropertyData(obj: u32, addr: *const PropertyAddress, qsize: u32, q: *const c_void, size: u32, data: *const c_void) -> i32;
}

const SYSTEM_OBJECT: u32 = 1;

fn output_device() -> Result<u32, String> {
    let addr = PropertyAddress { selector: fourcc(b"dOut"), scope: fourcc(b"glob"), element: 0 };
    let mut dev: u32 = 0;
    let mut size = 4u32;
    let st = unsafe { AudioObjectGetPropertyData(SYSTEM_OBJECT, &addr, 0, std::ptr::null(), &mut size, &mut dev as *mut u32 as *mut c_void) };
    if st != 0 || dev == 0 {
        return Err("no sound output device".into());
    }
    Ok(dev)
}

/// Output volume 0–100 and mute.
pub fn volume() -> Result<(f64, bool), String> {
    let dev = output_device()?;
    let vol_addr = PropertyAddress { selector: fourcc(b"vmvc"), scope: fourcc(b"outp"), element: 0 };
    let mut v: f32 = 0.0;
    let mut size = 4u32;
    let st = unsafe { AudioObjectGetPropertyData(dev, &vol_addr, 0, std::ptr::null(), &mut size, &mut v as *mut f32 as *mut c_void) };
    if st != 0 {
        return Err("the output device has no volume control".into());
    }
    let mute_addr = PropertyAddress { selector: fourcc(b"mute"), scope: fourcc(b"outp"), element: 0 };
    let mut m: u32 = 0;
    let mut size = 4u32;
    let st = unsafe { AudioObjectGetPropertyData(dev, &mute_addr, 0, std::ptr::null(), &mut size, &mut m as *mut u32 as *mut c_void) };
    Ok(((v as f64 * 100.0).round(), st == 0 && m != 0))
}

pub fn set_volume(percent: f64) -> Result<(), String> {
    let dev = output_device()?;
    let addr = PropertyAddress { selector: fourcc(b"vmvc"), scope: fourcc(b"outp"), element: 0 };
    let v = (percent.clamp(0.0, 100.0) / 100.0) as f32;
    let st = unsafe { AudioObjectSetPropertyData(dev, &addr, 0, std::ptr::null(), 4, &v as *const f32 as *const c_void) };
    if st != 0 {
        return Err("cannot set the volume of this output device".into());
    }
    if percent > 0.0 && volume().is_ok_and(|(_, muted)| muted) {
        set_mute(false)?;
    }
    Ok(())
}

pub fn set_mute(on: bool) -> Result<(), String> {
    let dev = output_device()?;
    let addr = PropertyAddress { selector: fourcc(b"mute"), scope: fourcc(b"outp"), element: 0 };
    let m: u32 = on as u32;
    let st = unsafe { AudioObjectSetPropertyData(dev, &addr, 0, std::ptr::null(), 4, &m as *const u32 as *const c_void) };
    if st != 0 {
        return Err("this output device cannot be muted".into());
    }
    Ok(())
}

// ── Disks ───────────────────────────────────────────────────────────────────

/// Unmount and eject every ejectable volume. Returns the names ejected and the failures. Blocks.
pub fn eject_all() -> (Vec<String>, Vec<String>) {
    let mut ejected = Vec::new();
    let mut failed = Vec::new();
    unsafe {
        let Some(fm_cls) = AnyClass::get(c"NSFileManager") else { return (ejected, failed) };
        let fm: Retained<AnyObject> = msg_send![fm_cls, defaultManager];
        let keys = NSArray::from_retained_slice(&[NSString::from_str("NSURLVolumeIsEjectableKey"), NSString::from_str("NSURLVolumeIsRemovableKey"), NSString::from_str("NSURLVolumeIsInternalKey"), NSString::from_str("NSURLVolumeLocalizedNameKey")]);
        // NSVolumeEnumerationSkipHiddenVolumes = 2
        let urls: Option<Retained<NSArray<NSURL>>> = msg_send![&*fm, mountedVolumeURLsIncludingResourceValuesForKeys: &*keys, options: 2usize];
        let Some(urls) = urls else { return (ejected, failed) };
        let ws = NSWorkspace::sharedWorkspace();
        for i in 0..urls.count() {
            let url = urls.objectAtIndex(i);
            let flag = |key: &str| -> bool {
                let mut v: *mut AnyObject = std::ptr::null_mut();
                let k = NSString::from_str(key);
                let ok: bool = msg_send![&*url, getResourceValue: &mut v, forKey: &*k, error: std::ptr::null_mut::<*mut AnyObject>()];
                ok && !v.is_null() && { let b: bool = msg_send![v, boolValue]; b }
            };
            let path = url.path().map(|p| p.to_string()).unwrap_or_default();
            if path == "/" || !(flag("NSURLVolumeIsEjectableKey") || flag("NSURLVolumeIsRemovableKey") || (!flag("NSURLVolumeIsInternalKey") && path.starts_with("/Volumes/"))) {
                continue;
            }
            let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(path.clone());
            let mut err: *mut AnyObject = std::ptr::null_mut();
            let ok: bool = msg_send![&*ws, unmountAndEjectDeviceAtURL: &*url, error: &mut err];
            if ok {
                ejected.push(name);
            } else {
                let why = if err.is_null() { "busy".to_string() } else { let d: Retained<NSString> = msg_send![err, localizedDescription]; d.to_string() };
                failed.push(format!("{name}: {why}"));
            }
        }
    }
    (ejected, failed)
}

// ── Running apps ────────────────────────────────────────────────────────────

pub struct RunningApp {
    pub pid: i32,
    pub name: String,
    pub path: String,
    pub bundle_id: String,
    pub active: bool,
    pub hidden: bool,
    /// Physical memory footprint in bytes (what Activity Monitor calls Memory).
    pub memory: u64,
}

fn footprint(pid: i32) -> u64 {
    // rusage_info_v0: 16-byte uuid, then u64s; ri_phys_footprint is the 8th.
    let mut buf = [0u64; 16];
    let st = unsafe { proc_pid_rusage(pid, 0, buf.as_mut_ptr() as *mut c_void) };
    if st == 0 {
        buf[2 + 7]
    } else {
        0
    }
}

/// Apps in the Dock (regular activation policy), this app excluded, by memory use.
pub fn running_apps() -> Vec<RunningApp> {
    let me = std::process::id() as i32;
    let apps = NSWorkspace::sharedWorkspace().runningApplications();
    let mut out: Vec<RunningApp> = (0..apps.count())
        .map(|i| apps.objectAtIndex(i))
        .filter(|a| a.activationPolicy() == NSApplicationActivationPolicy::Regular && a.processIdentifier() != me)
        .map(|a| {
            let pid = a.processIdentifier();
            RunningApp {
                pid,
                name: a.localizedName().map(|s| s.to_string()).unwrap_or_default(),
                path: a.bundleURL().and_then(|u| u.path()).map(|p| p.to_string()).unwrap_or_default(),
                bundle_id: a.bundleIdentifier().map(|s| s.to_string()).unwrap_or_default(),
                active: a.isActive(),
                hidden: a.isHidden(),
                memory: footprint(pid),
            }
        })
        .collect();
    out.sort_by(|a, b| b.memory.cmp(&a.memory));
    out
}

fn app_by_pid(pid: i32) -> Option<Retained<NSRunningApplication>> {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
}

/// `switch`, `hide`, `unhide`, `quit` or `force-quit` the app with `pid`. Returns a short message.
pub fn app_action(pid: i32, action: &str) -> Result<String, String> {
    let a = app_by_pid(pid).ok_or_else(|| format!("no app with pid {pid}"))?;
    let name = a.localizedName().map(|s| s.to_string()).unwrap_or_default();
    if !matches!(action, "switch" | "hide" | "unhide" | "quit" | "force-quit") {
        return Err(format!("unknown app action `{action}`"));
    }
    let ok = match action {
        "switch" => {
            a.unhide();
            #[allow(deprecated)]
            a.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows)
        }
        // On macOS 26 both return NO even when the app hides or shows, so the result is ignored.
        "hide" => {
            a.hide();
            true
        }
        "unhide" => {
            a.unhide();
            true
        }
        "quit" => a.terminate(),
        _ => a.forceTerminate(),
    };
    if !ok {
        return Err(format!("{name} did not {}", action.replace('-', " ")));
    }
    Ok(match action {
        "hide" => format!("Hid {name}"),
        "unhide" => format!("Showing {name}"),
        "quit" => format!("Asked {name} to quit"),
        "force-quit" => format!("Force quit {name}"),
        _ => String::new(),
    })
}

/// Quit every app in the Dock but Finder (and this app). Returns how many were asked.
pub fn quit_all() -> usize {
    let mut n = 0;
    for a in running_apps().iter().filter(|a| a.bundle_id != "com.apple.finder") {
        if app_by_pid(a.pid).is_some_and(|x| x.terminate()) {
            n += 1;
        }
    }
    n
}

/// Hide every app in the Dock (shows the desktop).
pub fn hide_all() -> usize {
    let mut n = 0;
    for a in running_apps() {
        if let Some(x) = app_by_pid(a.pid) {
            x.hide();
            n += 1;
        }
    }
    n
}

// ── Tish surface ────────────────────────────────────────────────────────────

use super::{arr, in_background, num_arg, obj, outcome, s, str_arg};
use tishlang_core::Value;

fn flag(args: &[Value], i: usize) -> bool {
    matches!(args.get(i), Some(Value::Bool(true)))
}

pub(super) fn t_lock(_a: &[Value]) -> Value { outcome(lock_screen()) }
pub(super) fn t_sleep(_a: &[Value]) -> Value { outcome(sleep()) }
pub(super) fn t_sleep_displays(_a: &[Value]) -> Value { outcome(sleep_displays()) }
pub(super) fn t_restart(_a: &[Value]) -> Value { outcome(restart()) }
pub(super) fn t_shut_down(_a: &[Value]) -> Value { outcome(shut_down()) }
pub(super) fn t_log_out(_a: &[Value]) -> Value { outcome(log_out()) }
pub(super) fn t_screen_saver(_a: &[Value]) -> Value { outcome(screen_saver().then_some(()).ok_or_else(|| "could not start the screen saver".to_string())) }

pub(super) fn t_empty_trash(args: &[Value]) -> Value {
    in_background(args.first(), empty_trash, outcome);
    Value::Null
}

pub(super) fn t_dark_mode(_a: &[Value]) -> Value {
    dark_mode().map_or(Value::Null, Value::Bool)
}

pub(super) fn t_set_dark_mode(args: &[Value]) -> Value { outcome(set_dark_mode(flag(args, 0))) }

pub(super) fn t_volume(_a: &[Value]) -> Value {
    match volume() {
        Ok((level, muted)) => obj(vec![("level", Value::Number(level)), ("muted", Value::Bool(muted))]),
        Err(e) => obj(vec![("error", s(&e))]),
    }
}

pub(super) fn t_set_volume(args: &[Value]) -> Value { outcome(set_volume(num_arg(args, 0, 0.0))) }
pub(super) fn t_set_muted(args: &[Value]) -> Value { outcome(set_mute(flag(args, 0))) }

pub(super) fn t_eject_all(args: &[Value]) -> Value {
    in_background(args.first(), eject_all, |(ejected, failed)| {
        obj(vec![("ejected", arr(ejected.iter().map(|x| s(x)).collect())), ("failed", arr(failed.iter().map(|x| s(x)).collect()))])
    });
    Value::Null
}

pub(super) fn t_running(_a: &[Value]) -> Value {
    arr(running_apps()
        .into_iter()
        .map(|a| {
            obj(vec![
                ("pid", Value::Number(a.pid as f64)),
                ("name", s(&a.name)),
                ("path", s(&a.path)),
                ("bundleId", s(&a.bundle_id)),
                ("active", Value::Bool(a.active)),
                ("hidden", Value::Bool(a.hidden)),
                ("memory", Value::Number(a.memory as f64)),
            ])
        })
        .collect())
}

pub(super) fn t_act(args: &[Value]) -> Value {
    match app_action(num_arg(args, 0, -1.0) as i32, &str_arg(args, 1)) {
        Ok(m) => obj(vec![("ok", Value::Bool(true)), ("message", s(&m)), ("error", s(""))]),
        Err(e) => obj(vec![("ok", Value::Bool(false)), ("message", s("")), ("error", s(&e))]),
    }
}

pub(super) fn t_quit_all(_a: &[Value]) -> Value { Value::Number(quit_all() as f64) }
pub(super) fn t_hide_all(_a: &[Value]) -> Value { Value::Number(hide_all() as f64) }
