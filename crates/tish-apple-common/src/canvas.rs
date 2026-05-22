//! Optional canvas RGBA provider registered by `tish-canvas` at link time.

use std::sync::OnceLock;

use tishlang_core::Value;

type CanvasRgbaFn = fn(&Value) -> Option<(u32, u32, Vec<u8>)>;

static PROVIDER: OnceLock<CanvasRgbaFn> = OnceLock::new();

pub fn register_canvas_rgba(f: CanvasRgbaFn) {
    let _ = PROVIDER.set(f);
}

pub fn canvas_rgba_bytes(canvas: &Value) -> Option<(u32, u32, Vec<u8>)> {
    PROVIDER.get().and_then(|f| f(canvas))
}
