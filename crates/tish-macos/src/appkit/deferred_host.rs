//! First `commit_root` picks sidebar vs single-pane shell from the root vnode tag.

use tishlang_core::Value;
use tishlang_ui::runtime::Host;
use tishlang_ui::runtime::RootId;

use super::{bootstrap_macos_real_host, MacosRealHost};

/// Until the first committed vnode, defers building `MacosHost` vs `MacosSidebarHost`.
pub struct DeferredMacosHost {
    pub open_options: Option<Value>,
    root_id: RootId,
    inner: Option<MacosRealHost>,
}

impl DeferredMacosHost {
    pub fn new(open_options: Option<Value>, root_id: RootId) -> Self {
        Self {
            open_options,
            root_id,
            inner: None,
        }
    }
}

impl Host for DeferredMacosHost {
    fn commit_root(&mut self, vnode: &Value) {
        if self.inner.is_none() {
            let opts = self.open_options.as_ref();
            self.inner = bootstrap_macos_real_host(vnode, opts, self.root_id);
        }
        match self.inner.as_mut() {
            Some(MacosRealHost::Content(h)) => h.commit_root(vnode),
            Some(MacosRealHost::Sidebar(h)) => h.commit_root(vnode),
            None => {}
        }
    }

    fn content_width_changed(&mut self, width: f64) {
        match self.inner.as_mut() {
            Some(MacosRealHost::Content(h)) => h.content_width_changed(width),
            Some(MacosRealHost::Sidebar(h)) => h.content_width_changed(width),
            None => {}
        }
    }

    fn after_window_shown(&mut self) {
        match self.inner.as_mut() {
            Some(MacosRealHost::Content(h)) => h.after_window_shown(),
            Some(MacosRealHost::Sidebar(h)) => h.after_window_shown(),
            None => {}
        }
    }

    fn detach_native_actions(&mut self) {
        match self.inner.as_mut() {
            Some(MacosRealHost::Content(h)) => h.detach_native_actions(),
            Some(MacosRealHost::Sidebar(h)) => h.detach_native_actions(),
            None => {}
        }
    }
}
