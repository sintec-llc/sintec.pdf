//! `Harness::render` builds a wgpu renderer (a Direct3D device on Windows) on every call. Several
//! tests doing that on parallel threads crash inside the WARP software rasterizer
//! (`d3d10warp.dll`, STATUS_ACCESS_VIOLATION), so every render goes through one process-wide lock:
//! `render_lock::locked(|| harness.render())`. A test that panics while rendering must not fail
//! the others, hence the poison tolerance.
#![allow(dead_code)] // each test crate uses only some of these helpers

use std::sync::Mutex;

static WGPU: Mutex<()> = Mutex::new(());

/// Runs `render` (anything that draws through wgpu) with no other thread of this process doing so.
pub fn locked<T>(render: impl FnOnce() -> T) -> T {
    let _guard = WGPU.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    render()
}
