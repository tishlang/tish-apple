//! `UserDefaults` + `NSSound` helpers for the Tish `macos.*` API surface.

use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::NSSound;
use objc2_foundation::{NSString, NSUserDefaults};

pub(crate) const PREF_PREFIX: &str = "tish.aiMessenger.";

fn pref_key(key: &str) -> String {
    format!("{PREF_PREFIX}{key}")
}

pub(crate) fn preference_get(key: &str) -> Option<String> {
    let defs = NSUserDefaults::standardUserDefaults();
    let k = NSString::from_str(&pref_key(key));
    defs.stringForKey(&k).map(|s| s.to_string())
}

pub(crate) fn preference_set(key: &str, value: &str) {
    let defs = NSUserDefaults::standardUserDefaults();
    let k = NSString::from_str(&pref_key(key));
    let v = NSString::from_str(value);
    let v_ref: &AnyObject = unsafe { &*std::ptr::from_ref(&*v).cast::<AnyObject>() };
    unsafe {
        defs.setObject_forKey(Some(v_ref), &k);
    }
}

pub(crate) fn play_named_sound(name: &str) {
    let n = name.trim();
    if n.is_empty() || n.eq_ignore_ascii_case("none") {
        return;
    }
    let _mtm = MainThreadMarker::new().expect("NSSound.play on main thread");
    let ns_name = NSString::from_str(n);
    if let Some(snd) = NSSound::soundNamed(ns_name.as_ref()) {
        let _ = snd.play();
        return;
    }
    let path = format!("/System/Library/Sounds/{n}.aiff");
    let path_ns = NSString::from_str(&path);
    let allocated = NSSound::alloc();
    if let Some(snd) = NSSound::initWithContentsOfFile_byReference(allocated, &path_ns, true) {
        let _ = snd.play();
    }
}
