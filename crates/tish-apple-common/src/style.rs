//! React-style prop helpers shared by AppKit and UIKit hosts.

use tishlang_core::{PropMap, Value};

pub fn props_string(props: &PropMap, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(Value::String(s)) = props.get(*k) {
            return Some(s.to_string());
        }
    }
    None
}

pub fn props_f64(props: &PropMap, keys: &[&str], default: f64) -> f64 {
    for k in keys {
        if let Some(n) = props.get(*k).and_then(|v| v.as_number()) {
            return n;
        }
    }
    default
}

pub fn props_bool(props: &PropMap, keys: &[&str], default: bool) -> bool {
    for k in keys {
        if let Some(v) = props.get(*k) {
            return match v {
                Value::Bool(b) => *b,
                Value::Number(n) => *n != 0.0,
                Value::Null => false,
                _ => default,
            };
        }
    }
    default
}

/// Parse `#RGB`, `#RRGGBB`, `#RRGGBBAA` into sRGB components (0..1) and alpha.
pub fn parse_hex_color(s: &str) -> Option<(f64, f64, f64, f64)> {
    let t = s.trim();
    let hex = t.strip_prefix('#')?;
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect();
    match bytes.len() {
        3 => Some((
            bytes[0] as f64 / 255.0,
            bytes[1] as f64 / 255.0,
            bytes[2] as f64 / 255.0,
            1.0,
        )),
        4 => Some((
            bytes[0] as f64 / 255.0,
            bytes[1] as f64 / 255.0,
            bytes[2] as f64 / 255.0,
            bytes[3] as f64 / 255.0,
        )),
        6 => Some((
            bytes[0] as f64 / 255.0,
            bytes[1] as f64 / 255.0,
            bytes[2] as f64 / 255.0,
            1.0,
        )),
        8 => Some((
            bytes[0] as f64 / 255.0,
            bytes[1] as f64 / 255.0,
            bytes[2] as f64 / 255.0,
            bytes[3] as f64 / 255.0,
        )),
        _ => None,
    }
}
