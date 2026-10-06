//! `macos.contacts`: the address book (Contacts framework). Reading needs the user's permission,
//! and the app needs `NSContactsUsageDescription` in its Info.plist (macOS ends an app that asks
//! without one). Only `request` shows the prompt; everything else fails quietly without access.
//!
//! - `status()` -> "notDetermined", "restricted", "denied", "authorized" or "limited"
//! - `request(cb)`: show the prompt once (later calls answer from the saved choice); `cb(granted)`
//! - `query(text, limit, cb)`: `cb({ contacts, error })`, contacts whose name matches `text`
//!   (name prefix first, then word prefix), every contact in address-book order when it's empty.
//!   Each is `{ id, name, given, family, nickname, org, title, emails, phones }`, the last two
//!   `[{ label, value }]` with labels in the user's language.

use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::msg_send;
use objc2_foundation::{NSArray, NSError, NSString};

#[link(name = "Contacts", kind = "framework")]
extern "C" {
    static CNContactGivenNameKey: &'static NSString;
    static CNContactFamilyNameKey: &'static NSString;
    static CNContactNicknameKey: &'static NSString;
    static CNContactOrganizationNameKey: &'static NSString;
    static CNContactJobTitleKey: &'static NSString;
    static CNContactEmailAddressesKey: &'static NSString;
    static CNContactPhoneNumbersKey: &'static NSString;
}

/// `CNEntityTypeContacts`.
const ENTITY_CONTACTS: isize = 0;
/// `CNContactSortOrderUserDefault`.
const SORT_USER_DEFAULT: isize = 1;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Field {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Contact {
    pub id: String,
    pub given: String,
    pub family: String,
    pub nickname: String,
    pub org: String,
    pub title: String,
    pub emails: Vec<Field>,
    pub phones: Vec<Field>,
}

impl Contact {
    /// "Given Family", else the nickname, company, first email or first phone.
    pub fn name(&self) -> String {
        let full = [self.given.trim(), self.family.trim()].iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" ");
        let named = [full.as_str(), self.nickname.trim(), self.org.trim()].into_iter().find(|n| !n.is_empty()).map(str::to_string);
        named
            .or_else(|| self.emails.first().map(|e| e.value.clone()))
            .or_else(|| self.phones.first().map(|p| p.value.clone()))
            .unwrap_or_default()
    }
}

/// notDetermined, restricted, denied, authorized or limited. Never prompts.
fn status() -> &'static str {
    let Some(cls) = AnyClass::get(c"CNContactStore") else { return "restricted" };
    let s: isize = unsafe { msg_send![cls, authorizationStatusForEntityType: ENTITY_CONTACTS] };
    match s {
        0 => "notDetermined",
        1 => "restricted",
        2 => "denied",
        3 => "authorized",
        _ => "limited",
    }
}

fn allowed() -> bool {
    matches!(status(), "authorized" | "limited")
}

/// Shows the system prompt (once; later calls answer from the saved choice). `done(granted)` runs
/// on a framework queue.
fn request(done: impl Fn(bool) + 'static) {
    let Some(cls) = AnyClass::get(c"CNContactStore") else { return done(false) };
    let store: Retained<AnyObject> = unsafe { msg_send![cls, new] };
    let keep = store.clone();
    let block = RcBlock::new(move |granted: Bool, _e: *mut NSError| {
        let _ = &keep;
        done(granted.as_bool());
    });
    unsafe {
        let _: () = msg_send![&*store, requestAccessForEntityType: ENTITY_CONTACTS, completionHandler: &*block];
    }
}

fn keys() -> Retained<NSArray<NSString>> {
    unsafe {
        NSArray::from_slice(&[
            CNContactGivenNameKey,
            CNContactFamilyNameKey,
            CNContactNicknameKey,
            CNContactOrganizationNameKey,
            CNContactJobTitleKey,
            CNContactEmailAddressesKey,
            CNContactPhoneNumbersKey,
        ])
    }
}

fn text(s: Option<Retained<NSString>>) -> String {
    s.map(|s| s.to_string()).unwrap_or_default()
}

/// "_$!<Mobile>!$_" and friends, in the user's language.
fn label(lv: &AnyObject) -> String {
    let raw: Option<Retained<NSString>> = unsafe { msg_send![lv, label] };
    let Some(raw) = raw else { return String::new() };
    let Some(cls) = AnyClass::get(c"CNLabeledValue") else { return raw.to_string() };
    let shown: Option<Retained<NSString>> = unsafe { msg_send![cls, localizedStringForLabel: &*raw] };
    shown.map(|s| s.to_string()).unwrap_or_else(|| raw.to_string())
}

fn fields(contact: &AnyObject, phones: bool) -> Vec<Field> {
    let list: Option<Retained<NSArray<AnyObject>>> =
        unsafe { if phones { msg_send![contact, phoneNumbers] } else { msg_send![contact, emailAddresses] } };
    let Some(list) = list else { return Vec::new() };
    list.iter()
        .filter_map(|lv| {
            let value: Option<Retained<AnyObject>> = unsafe { msg_send![&*lv, value] };
            let value = value?;
            let value = if phones {
                text(unsafe { msg_send![&*value, stringValue] })
            } else {
                let s: &NSString = unsafe { &*(Retained::as_ptr(&value) as *const NSString) };
                s.to_string()
            };
            (!value.is_empty()).then(|| Field { label: label(&lv), value })
        })
        .collect()
}

fn read(c: &AnyObject) -> Contact {
    unsafe {
        Contact {
            id: text(msg_send![c, identifier]),
            given: text(msg_send![c, givenName]),
            family: text(msg_send![c, familyName]),
            nickname: text(msg_send![c, nickname]),
            org: text(msg_send![c, organizationName]),
            title: text(msg_send![c, jobTitle]),
            emails: fields(c, false),
            phones: fields(c, true),
        }
    }
}

/// 0 when the name starts with `query`, 1 when a word in it does, 2 otherwise.
fn rank(c: &Contact, query: &str) -> u8 {
    let q = query.trim().to_lowercase();
    let name = c.name().to_lowercase();
    if q.is_empty() || name.starts_with(&q) {
        0
    } else if name.split_whitespace().any(|w| w.starts_with(&q)) {
        1
    } else {
        2
    }
}

fn sort(list: &mut [Contact], query: &str) {
    list.sort_by_cached_key(|c| (rank(c, query), c.name().to_lowercase()));
}

/// Contacts whose name matches `query`, best first; every contact (address-book order) when the
/// query is empty.
fn search(query: &str, limit: usize) -> Result<Vec<Contact>, String> {
    if !allowed() {
        return Err(format!("no access to contacts ({})", status()));
    }
    let store_cls = AnyClass::get(c"CNContactStore").ok_or("Contacts is unavailable")?;
    let store: Retained<AnyObject> = unsafe { msg_send![store_cls, new] };
    let keys = keys();
    if query.trim().is_empty() {
        return all(&store, &keys, limit);
    }
    let contact_cls = AnyClass::get(c"CNContact").ok_or("Contacts is unavailable")?;
    let name = NSString::from_str(query.trim());
    let predicate: Retained<AnyObject> = unsafe { msg_send![contact_cls, predicateForContactsMatchingName: &*name] };
    let found: Result<Retained<NSArray<AnyObject>>, Retained<NSError>> =
        unsafe { msg_send![&*store, unifiedContactsMatchingPredicate: &*predicate, keysToFetch: &*keys, error: _] };
    let found = found.map_err(|e| e.localizedDescription().to_string())?;
    let mut list: Vec<Contact> = found.iter().map(|c| read(&c)).collect();
    sort(&mut list, query);
    list.truncate(limit);
    Ok(list)
}

fn all(store: &AnyObject, keys: &NSArray<NSString>, limit: usize) -> Result<Vec<Contact>, String> {
    let req_cls = AnyClass::get(c"CNContactFetchRequest").ok_or("Contacts is unavailable")?;
    let req: Retained<AnyObject> = unsafe {
        let r: objc2::rc::Allocated<AnyObject> = msg_send![req_cls, alloc];
        msg_send![r, initWithKeysToFetch: keys]
    };
    unsafe {
        let _: () = msg_send![&*req, setSortOrder: SORT_USER_DEFAULT];
        let _: () = msg_send![&*req, setUnifyResults: true];
    }
    let out = std::cell::RefCell::new(Vec::new());
    let block = RcBlock::new(|c: NonNull<AnyObject>, stop: NonNull<Bool>| {
        let mut list = out.borrow_mut();
        list.push(read(unsafe { c.as_ref() }));
        if list.len() >= limit {
            unsafe { *stop.as_ptr() = Bool::YES };
        }
    });
    let ok: bool = unsafe {
        msg_send![store, enumerateContactsWithFetchRequest: &*req, error: std::ptr::null_mut::<*mut NSError>(), usingBlock: &*block]
    };
    drop(block);
    if !ok {
        return Err("could not read contacts".into());
    }
    Ok(out.into_inner())
}

// ── Tish surface ────────────────────────────────────────────────────────────

use super::{arr, deliver, hold, in_background, num_arg, obj, s, str_arg};
use tishlang_core::Value;

fn fields_value(list: &[Field]) -> Value {
    arr(list.iter().map(|f| obj(vec![("label", s(&f.label)), ("value", s(&f.value))])).collect())
}

fn contact_value(c: &Contact) -> Value {
    obj(vec![
        ("id", s(&c.id)),
        ("name", s(&c.name())),
        ("given", s(&c.given)),
        ("family", s(&c.family)),
        ("nickname", s(&c.nickname)),
        ("org", s(&c.org)),
        ("title", s(&c.title)),
        ("emails", fields_value(&c.emails)),
        ("phones", fields_value(&c.phones)),
    ])
}

pub(super) fn t_status(_a: &[Value]) -> Value {
    s(status())
}

pub(super) fn t_request(args: &[Value]) -> Value {
    let id = hold(args.first());
    request(move |granted| deliver(id, granted, Value::Bool));
    Value::Null
}

pub(super) fn t_search(args: &[Value]) -> Value {
    let query = str_arg(args, 0);
    let limit = num_arg(args, 1, 50.0).max(0.0) as usize;
    in_background(args.get(2), move || search(&query, limit), |r| match r {
        Ok(list) => obj(vec![("contacts", arr(list.iter().map(contact_value).collect())), ("error", s(""))]),
        Err(e) => obj(vec![("contacts", arr(Vec::new())), ("error", s(&e))]),
    });
    Value::Null
}
