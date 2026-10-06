//! `macos.statusItem(options)`: an icon in the menu bar with a menu. Installed once the run loop
//! is live; calling it again updates the tooltip and menu.
//!
//! `options`: `{ image, title, tooltip, menu, onClick, onMenu }`
//! - `image`: an image name (`macos.icons.image(path, true)`) or an SF Symbol, drawn as a template
//!   so it follows the menu bar's colour; `title` is shown when there's no image
//! - `menu`: `[{ title, id, key, enabled } | { separator: true }]`; `onMenu(id)` when one is picked
//! - `onClick()`: with it, a click calls `onClick` and a right click (or Control-click) opens the
//!   menu; without it any click opens the menu

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSEventMask, NSEventModifierFlags, NSEventType, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem};
use objc2_foundation::NSString;
use tishlang_core::Value;

use super::call;

#[derive(Clone)]
struct Entry {
    title: String,
    id: String,
    key: String,
    enabled: bool,
    separator: bool,
}

struct Options {
    image: String,
    title: String,
    tooltip: String,
    menu: Vec<Entry>,
    on_click: Option<Value>,
    on_menu: Option<Value>,
}

struct Installed {
    item: Retained<NSStatusItem>,
    menu: Retained<NSMenu>,
    _target: Retained<MenuTarget>,
}

thread_local! {
    static OPTIONS: RefCell<Option<Options>> = const { RefCell::new(None) };
    static INSTALLED: RefCell<Option<Installed>> = const { RefCell::new(None) };
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishMacosStatusTarget"]
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(statusClicked:))]
        fn status_clicked(&self, _sender: Option<&AnyObject>) {
            let Some(mtm) = MainThreadMarker::new() else { return };
            let menu_click = NSApplication::sharedApplication(mtm)
                .currentEvent()
                .is_some_and(|ev| ev.r#type() == NSEventType::RightMouseUp || ev.modifierFlags().contains(NSEventModifierFlags::Control));
            let on_click = OPTIONS.with(|o| o.borrow().as_ref().and_then(|o| o.on_click.clone()));
            if let (Some(cb), false) = (on_click, menu_click) {
                call(&cb, &[]);
                return;
            }
            INSTALLED.with(|i| {
                let i = i.borrow();
                let Some(st) = i.as_ref() else { return };
                // A status item opens its menu below itself on click; set it just for this click.
                st.item.setMenu(Some(&st.menu));
                if let Some(button) = st.item.button(mtm) {
                    unsafe { button.performClick(None) };
                }
                st.item.setMenu(None);
            });
        }

        #[unsafe(method(menuPicked:))]
        fn menu_picked(&self, sender: Option<&AnyObject>) {
            let Some(sender) = sender else { return };
            let tag: isize = unsafe { msg_send![sender, tag] };
            let picked = OPTIONS.with(|o| {
                let o = o.borrow();
                let o = o.as_ref()?;
                Some((o.menu.get(tag as usize)?.id.clone(), o.on_menu.clone()?))
            });
            if let Some((id, cb)) = picked {
                call(&cb, &[Value::String(id.as_str().into())]);
            }
        }
    }
);

fn text(o: &tishlang_core::PropMap, k: &str) -> String {
    match o.get(k) {
        Some(Value::String(x)) => x.to_string(),
        _ => String::new(),
    }
}

fn parse(v: Option<&Value>) -> Options {
    let mut out = Options { image: String::new(), title: String::new(), tooltip: String::new(), menu: Vec::new(), on_click: None, on_menu: None };
    let Some(Value::Object(o)) = v else { return out };
    let o = o.borrow();
    out.image = text(&o.strings, "image");
    out.title = text(&o.strings, "title");
    out.tooltip = text(&o.strings, "tooltip");
    out.on_click = o.strings.get("onClick").filter(|v| matches!(v, Value::Function(_))).cloned();
    out.on_menu = o.strings.get("onMenu").filter(|v| matches!(v, Value::Function(_))).cloned();
    if let Some(Value::Array(items)) = o.strings.get("menu") {
        for it in items.borrow().iter() {
            let Value::Object(e) = it else { continue };
            let e = e.borrow();
            out.menu.push(Entry {
                title: text(&e.strings, "title"),
                id: text(&e.strings, "id"),
                key: text(&e.strings, "key"),
                enabled: !matches!(e.strings.get("enabled"), Some(Value::Bool(false))),
                separator: matches!(e.strings.get("separator"), Some(Value::Bool(true))),
            });
        }
    }
    out
}

fn build_menu(mtm: MainThreadMarker, target: &MenuTarget, entries: &[Entry]) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    for (i, e) in entries.iter().enumerate() {
        if e.separator {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            continue;
        }
        let action = if e.enabled { Some(sel!(menuPicked:)) } else { None };
        let mi = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(&e.title), action, &NSString::from_str(&e.key)) };
        mi.setTag(i as isize);
        mi.setEnabled(e.enabled);
        unsafe { mi.setTarget(Some(target)) };
        menu.addItem(&mi);
    }
    menu
}

fn entries() -> Vec<Entry> {
    OPTIONS.with(|o| o.borrow().as_ref().map(|o| o.menu.clone()).unwrap_or_default())
}

fn install() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some((image, title, tooltip)) = OPTIONS.with(|o| o.borrow().as_ref().map(|o| (o.image.clone(), o.title.clone(), o.tooltip.clone()))) else { return };
    let existing = INSTALLED.with(|i| i.borrow().is_some());
    if !existing {
        // NSVariableStatusItemLength
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(-1.0);
        let target: Retained<MenuTarget> = unsafe { msg_send![MenuTarget::alloc(mtm), init] };
        if let Some(button) = item.button(mtm) {
            let label = NSString::from_str(if title.is_empty() { "•" } else { &title });
            let ns_image = NSString::from_str(&image);
            let img = if image.is_empty() { None } else { NSImage::imageNamed(&ns_image).or_else(|| NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns_image, Some(&label))) };
            match img {
                Some(img) => {
                    img.setTemplate(true);
                    button.setImage(Some(&img));
                }
                None => button.setTitle(&label),
            }
            unsafe {
                button.setTarget(Some(&target));
                button.setAction(Some(sel!(statusClicked:)));
            }
            button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
        }
        let menu = build_menu(mtm, &target, &entries());
        INSTALLED.with(|i| *i.borrow_mut() = Some(Installed { item, menu, _target: target }));
    } else {
        INSTALLED.with(|i| {
            let mut i = i.borrow_mut();
            let Some(st) = i.as_mut() else { return };
            st.menu = build_menu(mtm, &st._target, &entries());
        });
    }
    INSTALLED.with(|i| {
        if let Some(button) = i.borrow().as_ref().and_then(|st| st.item.button(mtm)) {
            let tip = NSString::from_str(&tooltip);
            button.setToolTip(if tooltip.is_empty() { None } else { Some(&tip) });
        }
    });
}

pub(super) fn status_item(args: &[Value]) -> Value {
    OPTIONS.with(|o| *o.borrow_mut() = Some(parse(args.first())));
    // After the run loop starts: a status item made before then may not appear.
    dispatch2::DispatchQueue::main().exec_async(install);
    Value::Bool(true)
}
