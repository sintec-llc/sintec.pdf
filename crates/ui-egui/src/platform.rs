pub(super) mod drag_pointer;
pub(super) mod file_filter;
#[cfg(not(target_arch = "wasm32"))]
pub mod file_watch;
#[cfg(target_os = "macos")]
pub(super) mod macos_association;
pub mod motion;
#[cfg(not(target_arch = "wasm32"))]
pub(super) mod open_worker;
#[cfg(not(target_arch = "wasm32"))]
pub(super) mod pickers;
pub mod portable;
#[cfg(not(target_arch = "wasm32"))]
pub(super) mod system_fonts;
pub mod ui_scale;
pub mod updates;
pub mod window_controls;

/// Reads an attribute for the macOS tests. The `xattr` crate's `get` asks for the length with an
/// empty buffer, which rustix 1.1.6 answers with 0, so it returns empty values.
#[cfg(all(test, target_os = "macos"))]
pub(crate) mod test_xattr {
    pub(crate) fn get(path: impl AsRef<std::path::Path>, name: &str) -> std::io::Result<Option<Vec<u8>>> {
        let path = path.as_ref();
        let size = match rustix::fs::getxattr_query(path, name) {
            Ok(size) => size,
            Err(rustix::io::Errno::NOATTR) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let mut value = vec![0u8; size.max(1)];
        let len = rustix::fs::getxattr(path, name, &mut value[..])?;
        value.truncate(len);
        Ok(Some(value))
    }
}
