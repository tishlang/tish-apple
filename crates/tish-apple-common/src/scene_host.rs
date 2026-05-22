//! Optional SceneView factory registered by app crates (e.g. `tish-juke-scene`).

#[cfg(target_os = "ios")]
mod ios {
    use std::sync::Mutex;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_ui_kit::{UIColor, UIView};

    pub type SceneViewFactory =
        fn(MainThreadMarker, f64, f64, Option<&tishlang_core::ObjectMap>) -> Option<Retained<UIView>>;

    static FACTORY: Mutex<Option<SceneViewFactory>> = Mutex::new(None);

    pub fn register_scene_view_factory(factory: SceneViewFactory) {
        *FACTORY.lock().unwrap() = Some(factory);
    }

    pub fn create_scene_view(
        mtm: MainThreadMarker,
        width: f64,
        height: f64,
        props: Option<&tishlang_core::ObjectMap>,
    ) -> Retained<UIView> {
        if let Some(factory) = FACTORY.lock().unwrap().as_ref() {
            if let Some(view) = factory(mtm, width, height, props) {
                return view;
            }
        }
        fallback_scene_view(mtm)
    }

    fn fallback_scene_view(mtm: MainThreadMarker) -> Retained<UIView> {
        let view = UIView::new(mtm);
        view.setBackgroundColor(Some(&UIColor::systemGrayColor()));
        view
    }
}

#[cfg(target_os = "ios")]
pub use ios::*;
