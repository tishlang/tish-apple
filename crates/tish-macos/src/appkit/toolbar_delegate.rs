//! `NSToolbarDelegate`: item list is mutable so Tish can declare toolbar via `SidebarWindow` props.
//!
//! Supports standard identifiers plus declarative SF Symbol items from `toolbarItems` object entries
//! `{ symbol, id, label? }` and `onToolbarAction` on the shell vnode.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSImage, NSImageSymbolConfiguration, NSImageSymbolScale, NSToolbar, NSToolbarDelegate,
    NSToolbarItem, NSToolbarItemIdentifier, NSToolbarItemStyle,
};
use objc2_foundation::{NSArray, NSInteger, NSObject, NSObjectProtocol, NSString};

use tishlang_ui::runtime::RootId;

use super::handlers::encode_toolbar_tag;
use super::router::MacosControlRouter;

/// Declarative toolbar slot: system AppKit identifier or SF Symbol item (see `toolbar_entries_from_props`).
pub enum ToolbarEntry {
    System(&'static NSToolbarItemIdentifier),
    Custom {
        ident: Retained<NSToolbarItemIdentifier>,
        symbol: String,
        label: String,
        slot_idx: usize,
    },
}

/// Storage for [`TishToolbarDelegate`] (required visible for `objc2::define_class!`).
pub struct TishToolbarIvars {
    root_id: Cell<RootId>,
    router: RefCell<Option<Retained<MacosControlRouter>>>,
    entries: RefCell<Vec<ToolbarEntry>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TishToolbarDelegate"]
    #[ivars = TishToolbarIvars]
    pub struct TishToolbarDelegate;

    unsafe impl NSObjectProtocol for TishToolbarDelegate {}

    unsafe impl NSToolbarDelegate for TishToolbarDelegate {
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn toolbar_itemForItemIdentifier_willBeInsertedIntoToolbar(
            &self,
            _toolbar: &NSToolbar,
            item_identifier: &NSToolbarItemIdentifier,
            _will_be_inserted: bool,
        ) -> Retained<NSToolbarItem> {
            let mtm =
                MainThreadMarker::new().expect("toolbar item creation must run on the main thread");
            let custom = self.ivars().entries.borrow().iter().find_map(|e| {
                if let ToolbarEntry::Custom {
                    ident,
                    symbol,
                    label,
                    slot_idx,
                } = e
                {
                    if item_identifier.isEqualToString(ident.as_ref()) {
                        return Some(Self::make_symbol_toolbar_item(
                            mtm,
                            item_identifier,
                            symbol,
                            label,
                            self.ivars().root_id.get(),
                            *slot_idx,
                            self.ivars().router.borrow().as_ref(),
                        ));
                    }
                }
                None
            });
            if let Some(item) = custom {
                item
            } else {
                NSToolbarItem::initWithItemIdentifier(
                    NSToolbarItem::alloc(mtm),
                    item_identifier,
                )
            }
        }

        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn toolbarDefaultItemIdentifiers(
            &self,
            _toolbar: &NSToolbar,
        ) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            self.toolbar_identifiers_array()
        }

        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn toolbarAllowedItemIdentifiers(
            &self,
            _toolbar: &NSToolbar,
        ) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            self.toolbar_identifiers_array()
        }
    }
);

impl TishToolbarDelegate {
    fn make_symbol_toolbar_item(
        mtm: MainThreadMarker,
        item_identifier: &NSToolbarItemIdentifier,
        symbol: &str,
        label: &str,
        root_id: RootId,
        slot_idx: usize,
        router: Option<&Retained<MacosControlRouter>>,
    ) -> Retained<NSToolbarItem> {
        let item = NSToolbarItem::initWithItemIdentifier(
            NSToolbarItem::alloc(mtm),
            item_identifier,
        );
        let sym = NSString::from_str(symbol);
        if let Some(im) = NSImage::imageWithSystemSymbolName_accessibilityDescription(&sym, None)
        {
            let cfg = NSImageSymbolConfiguration::configurationWithScale(NSImageSymbolScale::Small);
            let im2 = im.imageWithSymbolConfiguration(&cfg).unwrap_or(im);
            item.setImage(Some(&im2));
        }
        item.setLabel(&NSString::from_str(""));
        let pal = if label.is_empty() {
            NSString::from_str(symbol)
        } else {
            NSString::from_str(label)
        };
        item.setPaletteLabel(&pal);
        if !label.is_empty() {
            item.setToolTip(Some(&NSString::from_str(label)));
        }
        item.setBordered(false);
        item.setStyle(NSToolbarItemStyle::Plain);
        let tag = encode_toolbar_tag(root_id, slot_idx);
        item.setTag(tag as NSInteger);
        if let Some(r) = router {
            let p = Retained::as_ptr(r).cast::<AnyObject>();
            unsafe {
                item.setTarget(Some(&*p));
                item.setAction(Some(sel!(tishToolbarClick:)));
            }
        }
        item
    }

    pub fn new(
        mtm: MainThreadMarker,
        root_id: RootId,
        router: Option<&Retained<MacosControlRouter>>,
        entries: Vec<ToolbarEntry>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TishToolbarIvars {
            root_id: Cell::new(root_id),
            router: RefCell::new(router.cloned()),
            entries: RefCell::new(entries),
        });
        unsafe { msg_send![super(this), init] }
    }

    /// Default toolbar for the sidebar shell when the vnode does not declare `toolbar` / `toolbarItems`.
    pub fn new_legacy(
        mtm: MainThreadMarker,
        show_sidebar_toggle: bool,
        show_sidebar_tracking_separator: bool,
    ) -> Retained<Self> {
        unsafe {
            let mut v = Vec::new();
            if show_sidebar_toggle {
                v.push(ToolbarEntry::System(
                    objc2_app_kit::NSToolbarToggleSidebarItemIdentifier,
                ));
            }
            if show_sidebar_tracking_separator {
                v.push(ToolbarEntry::System(
                    objc2_app_kit::NSToolbarSidebarTrackingSeparatorItemIdentifier,
                ));
            }
            Self::new(mtm, 0, None, v)
        }
    }

    pub fn set_entries(
        &self,
        root_id: RootId,
        router: Option<&Retained<MacosControlRouter>>,
        entries: Vec<ToolbarEntry>,
    ) {
        self.ivars().root_id.set(root_id);
        *self.ivars().router.borrow_mut() = router.cloned();
        *self.ivars().entries.borrow_mut() = entries;
    }

    /// After [`set_entries`](Self::set_entries), `validateVisibleItems` is not enough: the window may
    /// still show the **initial** default set from first layout. Re-apply the identifier list so
    /// `toolbarItems` / `toolbar` from `<SidebarWindow>` actually appear.
    pub fn reload_default_items_into_toolbar(&self, toolbar: &NSToolbar) {
        let ids = self.toolbar_identifiers_array();
        toolbar.setItemIdentifiers(&ids);
        toolbar.validateVisibleItems();
    }

    pub fn set_legacy_sidebar_flags(
        &self,
        root_id: RootId,
        router: Option<&Retained<MacosControlRouter>>,
        show_sidebar_toggle: bool,
        show_separator: bool,
    ) {
        unsafe {
            let mut v = Vec::new();
            if show_sidebar_toggle {
                v.push(ToolbarEntry::System(
                    objc2_app_kit::NSToolbarToggleSidebarItemIdentifier,
                ));
            }
            if show_separator {
                v.push(ToolbarEntry::System(
                    objc2_app_kit::NSToolbarSidebarTrackingSeparatorItemIdentifier,
                ));
            }
            self.set_entries(root_id, router, v);
        }
    }

    fn toolbar_identifiers_array(&self) -> Retained<NSArray<NSToolbarItemIdentifier>> {
        let v = self.ivars().entries.borrow();
        let refs: Vec<&NSToolbarItemIdentifier> = v
            .iter()
            .map(|e| match e {
                ToolbarEntry::System(p) => *p,
                ToolbarEntry::Custom { ident, .. } => ident.as_ref(),
            })
            .collect();
        NSArray::from_slice(&refs)
    }
}
