//! `macos.spotlight.query(query, options, cb?)`: files from Spotlight's index (MDQuery,
//! in-process; no crawling of its own). `query` is an MDQuery string such as
//! `kMDItemFSName == "*tax*"cd && kMDItemContentTypeTree == "com.adobe.pdf"`.
//!
//! `options`: `{ scope, max }`, `scope` being "home" (the default) or a folder path, `max` the most
//! items (default 400). Items come back in Spotlight's order (not by relevance):
//! `[{ path, contentType, size, created, modified, lastUsed }]`, dates in Unix seconds (0 when
//! unknown; `lastUsed` 0 when never opened).
//!
//! Without `cb` the query runs here and returns the items; with `cb` it runs on a background thread
//! and `cb({ items, error })` comes back on the main thread.

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSString};
use tishlang_core::Value;

use super::{arr, in_background, obj, s, str_arg};

type CFTypeRef = *const c_void;

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn MDQueryCreate(alloc: CFTypeRef, query: CFTypeRef, value_list_attrs: CFTypeRef, sorting_attrs: CFTypeRef) -> CFTypeRef;
    fn MDQuerySetSearchScope(query: CFTypeRef, scope: CFTypeRef, options: u32);
    fn MDQuerySetMaxCount(query: CFTypeRef, size: isize);
    fn MDQueryExecute(query: CFTypeRef, options: usize) -> u8;
    fn MDQueryGetResultCount(query: CFTypeRef) -> isize;
    fn MDQueryGetResultAtIndex(query: CFTypeRef, idx: isize) -> CFTypeRef;
    fn MDItemCopyAttribute(item: CFTypeRef, name: CFTypeRef) -> CFTypeRef;
    static kMDItemPath: CFTypeRef;
    static kMDItemContentType: CFTypeRef;
    static kMDItemLastUsedDate: CFTypeRef;
    static kMDItemFSSize: CFTypeRef;
    static kMDItemFSCreationDate: CFTypeRef;
    static kMDItemFSContentChangeDate: CFTypeRef;
    static kMDQueryScopeHome: CFTypeRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFDateGetAbsoluteTime(date: CFTypeRef) -> f64;
    fn CFNumberGetValue(number: CFTypeRef, kind: i64, out: *mut c_void) -> bool;
}

const K_CF_NUMBER_SINT64: i64 = 4;
/// CFAbsoluteTime counts from 2001-01-01; Unix time from 1970.
const CF_EPOCH_UNIX: f64 = 978_307_200.0;
const K_MD_QUERY_SYNCHRONOUS: usize = 1;

struct Item {
    path: String,
    content_type: String,
    size: u64,
    created: f64,
    modified: f64,
    last_used: f64,
}

unsafe fn copy_string(item: CFTypeRef, attr: CFTypeRef) -> Option<String> {
    let v = MDItemCopyAttribute(item, attr);
    if v.is_null() {
        return None;
    }
    // +1, and CFString is toll-free bridged to NSString.
    let s: Retained<NSString> = Retained::from_raw(v as *mut NSString)?;
    Some(s.to_string())
}

unsafe fn copy_date(item: CFTypeRef, attr: CFTypeRef) -> f64 {
    let v = MDItemCopyAttribute(item, attr);
    if v.is_null() {
        return 0.0;
    }
    let t = CFDateGetAbsoluteTime(v);
    CFRelease(v);
    t + CF_EPOCH_UNIX
}

unsafe fn copy_u64(item: CFTypeRef, attr: CFTypeRef) -> u64 {
    let v = MDItemCopyAttribute(item, attr);
    if v.is_null() {
        return 0;
    }
    let mut n: i64 = 0;
    CFNumberGetValue(v, K_CF_NUMBER_SINT64, &mut n as *mut i64 as *mut c_void);
    CFRelease(v);
    n.max(0) as u64
}

fn run(query: &str, scope: &str, max: isize) -> Result<Vec<Item>, String> {
    let mut out = Vec::new();
    unsafe {
        let qs = NSString::from_str(query);
        let mdq = MDQueryCreate(std::ptr::null(), Retained::as_ptr(&qs) as CFTypeRef, std::ptr::null(), std::ptr::null());
        if mdq.is_null() {
            return Err(format!("not a Spotlight query: {query}"));
        }
        let folder = NSString::from_str(scope);
        let scopes: Retained<NSArray<NSString>> =
            if scope.is_empty() || scope == "home" { NSArray::from_slice(&[&*(kMDQueryScopeHome as *const NSString)]) } else { NSArray::from_slice(&[&*folder]) };
        MDQuerySetSearchScope(mdq, Retained::as_ptr(&scopes) as CFTypeRef, 0);
        MDQuerySetMaxCount(mdq, max);
        if MDQueryExecute(mdq, K_MD_QUERY_SYNCHRONOUS) != 0 {
            for i in 0..MDQueryGetResultCount(mdq) {
                let item = MDQueryGetResultAtIndex(mdq, i);
                let Some(path) = copy_string(item, kMDItemPath) else { continue };
                out.push(Item {
                    path,
                    content_type: copy_string(item, kMDItemContentType).unwrap_or_default(),
                    size: copy_u64(item, kMDItemFSSize),
                    created: copy_date(item, kMDItemFSCreationDate),
                    modified: copy_date(item, kMDItemFSContentChangeDate),
                    last_used: copy_date(item, kMDItemLastUsedDate),
                });
            }
        }
        CFRelease(mdq);
    }
    Ok(out)
}

fn date(t: f64) -> Value {
    // copy_date adds the epoch to a missing (0) date too; report never/unknown as 0.
    Value::Number(if t <= CF_EPOCH_UNIX { 0.0 } else { t })
}

fn items_value(items: &[Item]) -> Value {
    arr(items
        .iter()
        .map(|i| {
            obj(vec![
                ("path", s(&i.path)),
                ("contentType", s(&i.content_type)),
                ("size", Value::Number(i.size as f64)),
                ("created", date(i.created)),
                ("modified", date(i.modified)),
                ("lastUsed", date(i.last_used)),
            ])
        })
        .collect())
}

pub(super) fn query(args: &[Value]) -> Value {
    let q = str_arg(args, 0);
    let (scope, max) = match args.get(1) {
        Some(Value::Object(o)) => {
            let o = o.borrow();
            let scope = match o.strings.get("scope") {
                Some(Value::String(x)) => x.to_string(),
                _ => String::new(),
            };
            let max = match o.strings.get("max") {
                Some(Value::Number(n)) => *n as isize,
                _ => 400,
            };
            (scope, max)
        }
        _ => (String::new(), 400),
    };
    match args.get(2) {
        Some(cb @ Value::Function(_)) => {
            in_background(Some(cb), move || run(&q, &scope, max.max(1)), |r| match r {
                Ok(items) => obj(vec![("items", items_value(&items)), ("error", s(""))]),
                Err(e) => obj(vec![("items", arr(Vec::new())), ("error", s(&e))]),
            });
            Value::Null
        }
        _ => run(&q, &scope, max.max(1)).map(|items| items_value(&items)).unwrap_or_else(|_| arr(Vec::new())),
    }
}
