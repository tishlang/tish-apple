//! Thread-local callback registries keyed by Tish [`RootId`]. Cleared per root on full `commit_root`.
//! Never call Tish while holding a borrow.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2_app_kit::{
    NSUserInterfaceItemIdentification, NSSplitViewController, NSTextView, NSWindow,
};
use objc2_foundation::NSString;
use tishlang_ui::runtime::{RootId, LEGACY_ROOT_ID};

/// Bit 31 of the tag low word marks `NSToolbarItem` routing (see [`encode_toolbar_tag`]). Control
/// handler indices stay below that bit.
const TOOLBAR_TAG_LOW_MARKER: u64 = 1 << 31;

thread_local! {
    pub static TEXT_CHANGE_HANDLERS: RefCell<HashMap<RootId, Vec<Option<Rc<dyn Fn(String)>>>>> =
        RefCell::new(HashMap::new());
    pub static BOOL_HANDLERS: RefCell<HashMap<RootId, Vec<Option<Rc<dyn Fn(bool)>>>>> =
        RefCell::new(HashMap::new());
    pub static F64_HANDLERS: RefCell<HashMap<RootId, Vec<Option<Rc<dyn Fn(f64)>>>>> =
        RefCell::new(HashMap::new());
    pub static PICK_HANDLERS: RefCell<HashMap<RootId, Vec<Option<Rc<dyn Fn(i64)>>>>> =
        RefCell::new(HashMap::new());
    /// Primary `NSWindow` for [`LEGACY_ROOT_ID`] (fallback in `macos.run`).
    pub static MACOS_MAIN_WINDOW: RefCell<Option<Retained<NSWindow>>> = RefCell::new(None);
    static ROOT_WINDOWS: RefCell<HashMap<RootId, Retained<NSWindow>>> = RefCell::new(HashMap::new());
    static DETAIL_METRICS_PTR_BY_ROOT: RefCell<HashMap<RootId, usize>> = RefCell::new(HashMap::new());
    /// Per-root ordered action ids for declarative SF Symbol toolbar items (index → string passed to `onToolbarAction`).
    static TOOLBAR_ACTION_IDS: RefCell<HashMap<RootId, Vec<String>>> = RefCell::new(HashMap::new());
    /// Optional `onToolbarAction` from `<SidebarWindow>` (receives toolbar item `id` as `Value::String`).
    static TOOLBAR_ACTION_CALLBACK: RefCell<HashMap<RootId, Option<Rc<dyn Fn(String)>>>> =
        RefCell::new(HashMap::new());
    /// `NSSplitViewController` for `SidebarWindow` roots (`window.toggleSidebar` / `sidebarCollapsed`).
    static SPLIT_VIEW_CONTROLLERS: RefCell<HashMap<RootId, Retained<NSSplitViewController>>> =
        RefCell::new(HashMap::new());
}

pub use tish_apple_common::handlers::{
    clear_click_handlers_for_root, decode_control_tag, encode_control_tag, invoke_click_handler,
    register_click_handler, update_click_handler,
};

/// `NSTextView` is not an `NSControl`; `setTag:` is not supported on text views on current AppKit.
/// Store the packed handler id in `NSUserInterfaceItemIdentification` instead.
const TEXT_CHANGE_ID_PREFIX: &str = "tish.textChange.";

pub fn install_text_change_tag_on_text_view(tv: &NSTextView, tag: isize) {
    let s = format!("{TEXT_CHANGE_ID_PREFIX}{tag}");
    let ident = NSString::from_str(&s);
    NSUserInterfaceItemIdentification::setIdentifier(tv, Some(&ident));
}

pub fn text_change_tag_from_text_view(tv: &NSTextView) -> Option<isize> {
    let id = NSUserInterfaceItemIdentification::identifier(tv)?;
    let s = id.to_string();
    let rest = s.strip_prefix(TEXT_CHANGE_ID_PREFIX)?;
    rest.parse::<i64>().ok().map(|v| v as isize)
}

#[inline]
pub fn encode_toolbar_tag(root_id: RootId, idx: usize) -> isize {
    (((root_id as u64) << 32)
        | TOOLBAR_TAG_LOW_MARKER
        | (idx as u64 & 0x7FFF_FFFF)) as i64 as isize
}

#[inline]
pub fn decode_toolbar_tag(tag: isize) -> Option<(RootId, usize)> {
    let t = tag as u64;
    let lo = t & 0xFFFF_FFFF;
    if (lo & TOOLBAR_TAG_LOW_MARKER) == 0 {
        return None;
    }
    let idx = (lo & 0x7FFF_FFFF) as usize;
    let hi = (t >> 32) as RootId;
    let rid = if hi == 0 { LEGACY_ROOT_ID } else { hi };
    Some((rid, idx))
}

pub fn clear_toolbar_handlers(root_id: RootId) {
    TOOLBAR_ACTION_IDS.with(|m| {
        m.borrow_mut().remove(&root_id);
    });
    TOOLBAR_ACTION_CALLBACK.with(|m| {
        m.borrow_mut().remove(&root_id);
    });
}

pub fn set_toolbar_action_callback(root_id: RootId, f: Option<Rc<dyn Fn(String)>>) {
    TOOLBAR_ACTION_CALLBACK.with(|m| {
        match f {
            Some(cb) => {
                m.borrow_mut().insert(root_id, Some(cb));
            }
            None => {
                m.borrow_mut().remove(&root_id);
            }
        }
    });
}

/// Register next custom toolbar slot; returns index for [`encode_toolbar_tag`].
pub fn register_toolbar_action_slot(root_id: RootId, action_id: String) -> usize {
    TOOLBAR_ACTION_IDS.with(|cell| {
        let mut m = cell.borrow_mut();
        let v = m.entry(root_id).or_default();
        let i = v.len();
        v.push(action_id);
        i
    })
}

pub fn toolbar_action_id_for_slot(root_id: RootId, idx: usize) -> Option<String> {
    TOOLBAR_ACTION_IDS.with(|m| m.borrow().get(&root_id).and_then(|v| v.get(idx).cloned()))
}

pub fn invoke_toolbar_action(root_id: RootId, idx: usize) {
    let id = toolbar_action_id_for_slot(root_id, idx);
    let cb = TOOLBAR_ACTION_CALLBACK.with(|m| m.borrow().get(&root_id).cloned().flatten());
    if let (Some(id), Some(f)) = (id, cb) {
        f(id);
    }
}

/// Associate an `NSWindow` with a Tish [`RootId`] (for `window.*` and layout routing).
pub fn register_root_window(root_id: RootId, window: Retained<NSWindow>) {
    ROOT_WINDOWS.with(|m| {
        m.borrow_mut().insert(root_id, window.clone());
    });
    if root_id == LEGACY_ROOT_ID {
        MACOS_MAIN_WINDOW.with(|c| {
            *c.borrow_mut() = Some(window);
        });
    }
}

pub fn unregister_root_window(root_id: RootId) {
    clear_handlers_for_root(root_id);
    SPLIT_VIEW_CONTROLLERS.with(|m| {
        m.borrow_mut().remove(&root_id);
    });
    ROOT_WINDOWS.with(|m| {
        m.borrow_mut().remove(&root_id);
    });
    DETAIL_METRICS_PTR_BY_ROOT.with(|m| {
        m.borrow_mut().remove(&root_id);
    });
    if root_id == LEGACY_ROOT_ID {
        MACOS_MAIN_WINDOW.with(|c| {
            *c.borrow_mut() = None;
        });
    }
}

pub fn window_for_root(root_id: RootId) -> Option<Retained<NSWindow>> {
    ROOT_WINDOWS.with(|m| m.borrow().get(&root_id).cloned())
}

pub fn register_sidebar_split_view_controller(
    root_id: RootId,
    split_vc: Retained<NSSplitViewController>,
) {
    SPLIT_VIEW_CONTROLLERS.with(|m| {
        m.borrow_mut().insert(root_id, split_vc);
    });
}

/// Toggles the first split item (sidebar) via AppKit; no-op if this root is not a sidebar shell.
pub fn toggle_split_sidebar(root_id: RootId) {
    let Some(svc) = SPLIT_VIEW_CONTROLLERS.with(|m| m.borrow().get(&root_id).cloned()) else {
        return;
    };
    unsafe {
        svc.toggleSidebar(None);
    }
}

/// `true` when the sidebar split item is collapsed; `false` if unknown or not a sidebar shell.
pub fn is_split_sidebar_collapsed(root_id: RootId) -> bool {
    let Some(svc) = SPLIT_VIEW_CONTROLLERS.with(|m| m.borrow().get(&root_id).cloned()) else {
        return false;
    };
    let items = svc.splitViewItems();
    if items.count() < 1 {
        return false;
    }
    let first = items.objectAtIndex(0);
    first.isCollapsed()
}

pub fn detail_metrics_ptr_for_root(root_id: RootId) -> usize {
    DETAIL_METRICS_PTR_BY_ROOT
        .with(|m| *m.borrow().get(&root_id).unwrap_or(&0))
}

pub fn set_detail_metrics_for_root(root_id: RootId, ptr: usize) {
    DETAIL_METRICS_PTR_BY_ROOT.with(|m| {
        let mut map = m.borrow_mut();
        if ptr == 0 {
            map.remove(&root_id);
        } else {
            map.insert(root_id, ptr);
        }
    });
}

pub fn clear_handlers_for_root(root_id: RootId) {
    clear_toolbar_handlers(root_id);
    clear_click_handlers_for_root(root_id);
    TEXT_CHANGE_HANDLERS.with(|c| {
        c.borrow_mut().remove(&root_id);
    });
    BOOL_HANDLERS.with(|c| {
        c.borrow_mut().remove(&root_id);
    });
    F64_HANDLERS.with(|c| {
        c.borrow_mut().remove(&root_id);
    });
    PICK_HANDLERS.with(|c| {
        c.borrow_mut().remove(&root_id);
    });
}


pub fn register_text_change_handler(root_id: RootId, f: Rc<dyn Fn(String)>) -> isize {
    TEXT_CHANGE_HANDLERS.with(|c| {
        let mut m = c.borrow_mut();
        let v = m.entry(root_id).or_default();
        let i = v.len();
        v.push(Some(f));
        encode_control_tag(root_id, i)
    })
}

pub fn register_bool_handler(root_id: RootId, f: Rc<dyn Fn(bool)>) -> isize {
    BOOL_HANDLERS.with(|c| {
        let mut m = c.borrow_mut();
        let v = m.entry(root_id).or_default();
        let i = v.len();
        v.push(Some(f));
        encode_control_tag(root_id, i)
    })
}

pub fn register_f64_handler(root_id: RootId, f: Rc<dyn Fn(f64)>) -> isize {
    F64_HANDLERS.with(|c| {
        let mut m = c.borrow_mut();
        let v = m.entry(root_id).or_default();
        let i = v.len();
        v.push(Some(f));
        encode_control_tag(root_id, i)
    })
}

pub fn register_pick_handler(root_id: RootId, f: Rc<dyn Fn(i64)>) -> isize {
    PICK_HANDLERS.with(|c| {
        let mut m = c.borrow_mut();
        let v = m.entry(root_id).or_default();
        let i = v.len();
        v.push(Some(f));
        encode_control_tag(root_id, i)
    })
}

fn ensure_vec_len<T>(v: &mut Vec<Option<T>>, len: usize) {
    while v.len() <= len {
        v.push(None);
    }
}

pub fn update_text_change_handler(
    root_id: RootId,
    idx: usize,
    f: Rc<dyn Fn(String)>,
) -> isize {
    TEXT_CHANGE_HANDLERS.with(|c| {
        let mut m = c.borrow_mut();
        let v = m.entry(root_id).or_default();
        ensure_vec_len(v, idx);
        v[idx] = Some(f);
    });
    encode_control_tag(root_id, idx)
}


pub fn update_bool_handler(root_id: RootId, idx: usize, f: Rc<dyn Fn(bool)>) -> isize {
    BOOL_HANDLERS.with(|c| {
        let mut m = c.borrow_mut();
        let v = m.entry(root_id).or_default();
        ensure_vec_len(v, idx);
        v[idx] = Some(f);
    });
    encode_control_tag(root_id, idx)
}

pub fn update_f64_handler(root_id: RootId, idx: usize, f: Rc<dyn Fn(f64)>) -> isize {
    F64_HANDLERS.with(|c| {
        let mut m = c.borrow_mut();
        let v = m.entry(root_id).or_default();
        ensure_vec_len(v, idx);
        v[idx] = Some(f);
    });
    encode_control_tag(root_id, idx)
}

pub fn update_pick_handler(root_id: RootId, idx: usize, f: Rc<dyn Fn(i64)>) -> isize {
    PICK_HANDLERS.with(|c| {
        let mut m = c.borrow_mut();
        let v = m.entry(root_id).or_default();
        ensure_vec_len(v, idx);
        v[idx] = Some(f);
    });
    encode_control_tag(root_id, idx)
}
