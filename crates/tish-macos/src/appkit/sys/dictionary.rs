//! `macos.dictionary.lookup(word)` -> the plain text of each homograph of `word` in the dictionary
//! macOS ships (New Oxford American Dictionary, else Oxford Dictionary of English), first one first;
//! [] when it has none. Each text reads: headword, `| pronunciation |`, then per part of speech
//! numbered senses, then PHRASES, DERIVATIVES and ORIGIN sections.
//!
//! The public `DCSCopyTextDefinition` returns only the first homograph ("bank" the river side, not
//! the money one), so the record functions are looked up at run time for the rest; without them
//! the first one is still returned.

use std::ffi::{c_char, c_void, CString};
use std::sync::OnceLock;

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSArray, NSString};

type Ptr = *const c_void;

#[repr(C)]
struct CFRange {
    location: isize,
    length: isize,
}

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn DCSCopyTextDefinition(dictionary: Ptr, text: Ptr, range: CFRange) -> Ptr;
}

extern "C" {
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// The default dictionary's plain-text entry for `word` (its first homograph), or None.
fn lookup_first(word: &str) -> Option<String> {
    let word = word.trim();
    if word.is_empty() {
        return None;
    }
    let text = NSString::from_str(word);
    let range = CFRange {
        location: 0,
        length: text.length() as isize,
    };
    let raw =
        unsafe { DCSCopyTextDefinition(std::ptr::null(), Retained::as_ptr(&text).cast(), range) };
    // +1 CFString, toll-free bridged to NSString.
    let def = unsafe { Retained::from_raw(raw as *mut NSString) }?;
    Some(def.to_string())
}

struct Records {
    dictionary: usize,
    search: extern "C" fn(Ptr, Ptr, Ptr, Ptr) -> Ptr,
    headword: extern "C" fn(Ptr) -> Ptr,
    copy_data: extern "C" fn(Ptr, i64) -> Ptr,
}

/// `DCSRecordCopyData` version that returns the same plain text as `DCSCopyTextDefinition`.
const RECORD_TEXT: i64 = 3;
/// The dictionary `DCSCopyTextDefinition` answers from, first one found.
const DICTIONARIES: &[&str] = &[
    "New Oxford American Dictionary",
    "Oxford Dictionary of English",
];

fn records() -> Option<&'static Records> {
    static RECORDS: OnceLock<Option<Records>> = OnceLock::new();
    RECORDS
        .get_or_init(|| unsafe {
            let lib =
                CString::new("/System/Library/Frameworks/CoreServices.framework/CoreServices")
                    .ok()?;
            let h = dlopen(lib.as_ptr(), 1);
            if h.is_null() {
                return None;
            }
            let sym = |n: &str| {
                CString::new(n)
                    .ok()
                    .map(|c| dlsym(h, c.as_ptr()))
                    .filter(|p| !p.is_null())
            };
            let available: extern "C" fn() -> Ptr =
                std::mem::transmute(sym("DCSCopyAvailableDictionaries")?);
            let name: extern "C" fn(Ptr) -> Ptr = std::mem::transmute(sym("DCSDictionaryGetName")?);
            let r = Records {
                dictionary: 0,
                search: std::mem::transmute(sym("DCSCopyRecordsForSearchString")?),
                headword: std::mem::transmute(sym("DCSRecordGetHeadword")?),
                copy_data: std::mem::transmute(sym("DCSRecordCopyData")?),
            };
            // A +1 NSSet, kept for the life of the process so its dictionaries stay valid.
            let set = available() as *const AnyObject;
            if set.is_null() {
                return None;
            }
            let all: Retained<NSArray<AnyObject>> = msg_send![&*set, allObjects];
            let named: Vec<(String, usize)> = all
                .iter()
                .map(|d| {
                    let p = Retained::as_ptr(&d) as Ptr;
                    (cf_string(name(p)), p as usize)
                })
                .collect();
            let dictionary = DICTIONARIES
                .iter()
                .find_map(|want| named.iter().find(|(n, _)| n == want).map(|&(_, p)| p))?;
            Some(Records { dictionary, ..r })
        })
        .as_ref()
}

/// A +0 CFString as a Rust string.
fn cf_string(p: Ptr) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { &*(p as *const NSString) }.to_string()
}

/// Every homograph of `word` as plain text, the first one first; empty when there is none.
fn lookup_all(word: &str) -> Vec<String> {
    let first = lookup_first(word);
    let Some(r) = records() else {
        return first.into_iter().collect();
    };
    // Records also match other words; keep the homographs of the entry the public call found.
    let base = first
        .as_deref()
        .map(|t| headword(t.split(" | ").next().unwrap_or(""), word))
        .unwrap_or_else(|| word.trim().to_string());
    let query = NSString::from_str(word.trim());
    let found = (r.search)(
        r.dictionary as Ptr,
        Retained::as_ptr(&query).cast(),
        std::ptr::null(),
        std::ptr::null(),
    );
    let Some(found) = (unsafe { Retained::from_raw(found as *mut NSArray<AnyObject>) }) else {
        return first.into_iter().collect();
    };
    let mut out = Vec::new();
    for rec in found.iter() {
        let p = Retained::as_ptr(&rec) as Ptr;
        if !cf_string((r.headword)(p)).eq_ignore_ascii_case(&base) {
            continue;
        }
        if let Some(t) =
            unsafe { Retained::from_raw((r.copy_data)(p, RECORD_TEXT) as *mut NSString) }
        {
            out.push(t.to_string());
        }
    }
    if out.is_empty() {
        first.into_iter().collect()
    } else {
        out
    }
}

/// "serendipity ser·en·dip·i·ty" → "serendipity"; "Paris 1 Par·is" → "Paris".
/// (dict.tish has the same rule, for the entry it parses.)
fn headword(head: &str, asked: &str) -> String {
    let words: Vec<&str> = head
        .split_whitespace()
        .take_while(|w| !w.contains('·') && !w.chars().all(|c| c.is_ascii_digit()))
        .collect();
    if words.is_empty() {
        asked.to_string()
    } else {
        words.join(" ")
    }
}

pub(super) fn lookup(args: &[tishlang_core::Value]) -> tishlang_core::Value {
    super::arr(
        lookup_all(&super::str_arg(args, 0))
            .iter()
            .map(|t| super::s(t))
            .collect(),
    )
}
