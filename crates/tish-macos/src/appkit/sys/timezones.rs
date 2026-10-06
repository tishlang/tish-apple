//! `macos.timeZones`: macOS's time-zone database, with daylight saving as of a given moment.
//!
//! - `names()` -> every zone id ("Asia/Tokyo", …)
//! - `local()` -> the local zone's id
//! - `at(id, unix)` -> `{ offset, abbreviation }` (seconds east of UTC, "JST"), or null for an
//!   unknown id. (Called `at` on the namespace object; a method call `tz.at(…)` is fine, but the
//!   native backend lowers a bare `x.at(i)` on an unknown receiver to `Array.prototype.at`, so bind
//!   it first: `let zoneAt = macos.timeZones.at`.)
//! - `byAbbreviation("CET")` -> a zone id, or null

use objc2_foundation::{NSDate, NSString, NSTimeZone};
use tishlang_core::Value;

use super::{arr, num_arg, obj, s, str_arg};

pub(super) fn names(_a: &[Value]) -> Value {
    arr(NSTimeZone::knownTimeZoneNames().iter().map(|n| s(&n.to_string())).collect())
}

pub(super) fn local(_a: &[Value]) -> Value {
    s(&NSTimeZone::localTimeZone().name().to_string())
}

pub(super) fn at(args: &[Value]) -> Value {
    let Some(tz) = NSTimeZone::timeZoneWithName(&NSString::from_str(&str_arg(args, 0))) else { return Value::Null };
    let date = NSDate::dateWithTimeIntervalSince1970(num_arg(args, 1, 0.0));
    let abbr = tz.abbreviationForDate(&date).map(|x| x.to_string()).unwrap_or_default();
    obj(vec![("offset", Value::Number(tz.secondsFromGMTForDate(&date) as f64)), ("abbreviation", s(&abbr))])
}

pub(super) fn by_abbreviation(args: &[Value]) -> Value {
    NSTimeZone::timeZoneWithAbbreviation(&NSString::from_str(&str_arg(args, 0))).map_or(Value::Null, |tz| s(&tz.name().to_string()))
}
