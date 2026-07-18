//! Thread-local callback registries keyed by Tish [`RootId`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use tishlang_ui::runtime::{RootId, LEGACY_ROOT_ID};

thread_local! {
    static CLICK_HANDLERS: RefCell<HashMap<RootId, Vec<Option<Rc<dyn Fn()>>>>> =
        RefCell::new(HashMap::new());
    static TEXT_CHANGE_HANDLERS: RefCell<HashMap<RootId, Vec<Option<Rc<dyn Fn(String)>>>>> =
        RefCell::new(HashMap::new());
    static BOOL_HANDLERS: RefCell<HashMap<RootId, Vec<Option<Rc<dyn Fn(bool)>>>>> =
        RefCell::new(HashMap::new());
    static F64_HANDLERS: RefCell<HashMap<RootId, Vec<Option<Rc<dyn Fn(f64)>>>>> =
        RefCell::new(HashMap::new());
}

#[inline]
pub fn encode_control_tag(root_id: RootId, idx: usize) -> isize {
    (((root_id as u64) << 32) | (idx as u64 & 0xFFFF_FFFF)) as i64 as isize
}

#[inline]
pub fn decode_control_tag(tag: isize) -> (RootId, usize) {
    let t = tag as u64;
    let hi = (t >> 32) as RootId;
    let lo = (t & 0xFFFF_FFFF) as usize;
    if hi == 0 {
        (LEGACY_ROOT_ID, lo)
    } else {
        (hi, lo)
    }
}

pub fn register_click_handler(root_id: RootId, handler: Rc<dyn Fn()>) -> isize {
    CLICK_HANDLERS.with(|map| {
        let mut m = map.borrow_mut();
        let vec = m.entry(root_id).or_default();
        let idx = vec.len();
        vec.push(Some(handler));
        encode_control_tag(root_id, idx)
    })
}

pub fn update_click_handler(root_id: RootId, slot: usize, handler: Rc<dyn Fn()>) -> isize {
    CLICK_HANDLERS.with(|map| {
        let mut m = map.borrow_mut();
        let vec = m.entry(root_id).or_default();
        while vec.len() <= slot {
            vec.push(None);
        }
        vec[slot] = Some(handler);
        encode_control_tag(root_id, slot)
    })
}

pub fn invoke_click_handler(tag: isize) {
    let (root_id, slot) = decode_control_tag(tag);
    let handler = CLICK_HANDLERS.with(|map| {
        map.borrow()
            .get(&root_id)
            .and_then(|vec| vec.get(slot))
            .and_then(|slot| slot.as_ref().cloned())
    });
    if let Some(handler) = handler {
        handler();
    }
}

pub fn clear_click_handlers_for_root(root_id: RootId) {
    CLICK_HANDLERS.with(|map| {
        map.borrow_mut().remove(&root_id);
    });
}

pub fn register_text_change_handler(root_id: RootId, handler: Rc<dyn Fn(String)>) -> isize {
    TEXT_CHANGE_HANDLERS.with(|map| {
        let mut m = map.borrow_mut();
        let vec = m.entry(root_id).or_default();
        let idx = vec.len();
        vec.push(Some(handler));
        encode_control_tag(root_id, idx)
    })
}

pub fn update_text_change_handler(
    root_id: RootId,
    slot: usize,
    handler: Rc<dyn Fn(String)>,
) -> isize {
    TEXT_CHANGE_HANDLERS.with(|map| {
        let mut m = map.borrow_mut();
        let vec = m.entry(root_id).or_default();
        while vec.len() <= slot {
            vec.push(None);
        }
        vec[slot] = Some(handler);
        encode_control_tag(root_id, slot)
    })
}

pub fn invoke_text_change_handler(tag: isize, text: String) {
    let (root_id, slot) = decode_control_tag(tag);
    let handler = TEXT_CHANGE_HANDLERS.with(|map| {
        map.borrow()
            .get(&root_id)
            .and_then(|vec| vec.get(slot))
            .and_then(|slot| slot.as_ref().cloned())
    });
    if let Some(handler) = handler {
        handler(text);
    }
}

pub fn clear_text_change_handlers_for_root(root_id: RootId) {
    TEXT_CHANGE_HANDLERS.with(|map| {
        map.borrow_mut().remove(&root_id);
    });
}

pub fn register_bool_handler(root_id: RootId, handler: Rc<dyn Fn(bool)>) -> isize {
    BOOL_HANDLERS.with(|map| {
        let mut m = map.borrow_mut();
        let vec = m.entry(root_id).or_default();
        let idx = vec.len();
        vec.push(Some(handler));
        encode_control_tag(root_id, idx)
    })
}

pub fn update_bool_handler(root_id: RootId, slot: usize, handler: Rc<dyn Fn(bool)>) -> isize {
    BOOL_HANDLERS.with(|map| {
        let mut m = map.borrow_mut();
        let vec = m.entry(root_id).or_default();
        while vec.len() <= slot {
            vec.push(None);
        }
        vec[slot] = Some(handler);
        encode_control_tag(root_id, slot)
    })
}

pub fn invoke_bool_handler(tag: isize, value: bool) {
    let (root_id, slot) = decode_control_tag(tag);
    let handler = BOOL_HANDLERS.with(|map| {
        map.borrow()
            .get(&root_id)
            .and_then(|vec| vec.get(slot))
            .and_then(|slot| slot.as_ref().cloned())
    });
    if let Some(handler) = handler {
        handler(value);
    }
}

pub fn clear_bool_handlers_for_root(root_id: RootId) {
    BOOL_HANDLERS.with(|map| {
        map.borrow_mut().remove(&root_id);
    });
}

pub fn register_f64_handler(root_id: RootId, handler: Rc<dyn Fn(f64)>) -> isize {
    F64_HANDLERS.with(|map| {
        let mut m = map.borrow_mut();
        let vec = m.entry(root_id).or_default();
        let idx = vec.len();
        vec.push(Some(handler));
        encode_control_tag(root_id, idx)
    })
}

pub fn update_f64_handler(root_id: RootId, slot: usize, handler: Rc<dyn Fn(f64)>) -> isize {
    F64_HANDLERS.with(|map| {
        let mut m = map.borrow_mut();
        let vec = m.entry(root_id).or_default();
        while vec.len() <= slot {
            vec.push(None);
        }
        vec[slot] = Some(handler);
        encode_control_tag(root_id, slot)
    })
}

pub fn invoke_f64_handler(tag: isize, value: f64) {
    let (root_id, slot) = decode_control_tag(tag);
    let handler = F64_HANDLERS.with(|map| {
        map.borrow()
            .get(&root_id)
            .and_then(|vec| vec.get(slot))
            .and_then(|slot| slot.as_ref().cloned())
    });
    if let Some(handler) = handler {
        handler(value);
    }
}

pub fn clear_f64_handlers_for_root(root_id: RootId) {
    F64_HANDLERS.with(|map| {
        map.borrow_mut().remove(&root_id);
    });
}

/// Clear all shared control handler registries for a root (iOS full rebuild).
pub fn clear_all_handlers_for_root(root_id: RootId) {
    clear_click_handlers_for_root(root_id);
    clear_text_change_handlers_for_root(root_id);
    clear_bool_handlers_for_root(root_id);
    clear_f64_handlers_for_root(root_id);
}
