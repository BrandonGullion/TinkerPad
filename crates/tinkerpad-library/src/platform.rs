//! Platform-specific pieces of the library: moving folders to the trash.
//!
//! One of three modules allowed to use platform `cfg` (see
//! `scripts/lint-platform-cfg.sh` and `docs/ARCHITECTURE.md` §10).

use std::path::Path;

/// Move `path` to the OS recycle bin / trash. Never deletes permanently.
///
/// On macOS this uses `NSFileManager` rather than the `trash` crate's default
/// of scripting Finder, which prompts the user for automation permission and
/// plays a sound. The trade-off is that "Put Back" may not appear in Finder;
/// the folder can still be dragged out of the Trash.
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use trash::TrashContext;
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut context = TrashContext::default();
        context.set_delete_method(DeleteMethod::NsFileManager);
        context.delete(path).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        trash::delete(path).map_err(|e| e.to_string())
    }
}
