//! Dynamic loading of `.ofx` shared libraries and enumeration of their plugins.
//!
//! On Linux, `.ofx` files are plain shared objects (`.so`).
//! On macOS, they are `.dylib` files inside a bundle.
//! On Windows, they are `.dll` files.
//!
//! We use the [`libloading`] crate for cross-platform `dlopen` / `LoadLibrary`.

use std::ffi::CStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use libloading::{Library, Symbol};
use tracing::{debug, info, warn};

use crate::ffi::{OfxPlugin, OfxGetNumberOfPluginsFn, OfxGetPluginFn};
use crate::host::OfxHost;
use crate::error::OfxError;

/// Metadata extracted from an OFX plugin entry without needing to call into it.
#[derive(Debug, Clone)]
pub struct OfxPluginInfo {
    pub api:            String,
    pub api_version:    i32,
    pub identifier:     String,
    pub version_major:  i32,
    pub version_minor:  i32,
    /// Index within the library (for `OfxGetPlugin(nth)`).
    pub library_index:  usize,
}

/// A loaded `.ofx` shared library with all its plugin entries enumerated.
pub struct OfxPluginLibrary {
    /// The underlying shared library handle; kept alive as long as this struct
    /// exists so that all function pointers remain valid.
    _lib:    Library,
    pub path: PathBuf,
    plugins: Vec<(OfxPluginInfo, *mut OfxPlugin)>,
}

// SAFETY: We never share raw `OfxPlugin` pointers across threads.  All access
// goes through the single `render` call which is serialised at the DAG level.
unsafe impl Send for OfxPluginLibrary {}
unsafe impl Sync for OfxPluginLibrary {}

impl OfxPluginLibrary {
    /// Load an `.ofx` file, call `setHost` on each plugin, and return the
    /// populated library handle.
    ///
    /// # Safety
    /// Loading arbitrary shared libraries is inherently unsafe.  Only load
    /// `.ofx` files from trusted sources.
    pub fn load(path: &Path, host: &Arc<OfxHost>) -> Result<Self, OfxError> {
        info!("Loading OFX library: {}", path.display());

        // SAFETY: libloading opens the library with RTLD_LOCAL | RTLD_NOW.
        let lib = unsafe {
            Library::new(path).map_err(|e| OfxError::LibraryLoad {
                path:   path.to_string_lossy().into(),
                reason: e.to_string(),
            })?
        };

        // ── Enumerate plugins ─────────────────────────────────────────────────
        let n_plugins: c_int = unsafe {
            let sym: Symbol<OfxGetNumberOfPluginsFn> = lib
                .get(b"OfxGetNumberOfPlugins\0")
                .map_err(|_| OfxError::MissingSymbol("OfxGetNumberOfPlugins".into()))?;
            sym()
        };

        info!("  Found {n_plugins} plugin(s)");

        let get_plugin: Symbol<OfxGetPluginFn> = unsafe {
            lib.get(b"OfxGetPlugin\0")
                .map_err(|_| OfxError::MissingSymbol("OfxGetPlugin".into()))?
        };

        let mut plugins = Vec::with_capacity(n_plugins as usize);

        for i in 0..n_plugins {
            // SAFETY: OFX contract says `OfxGetPlugin(i)` returns a pointer
            // to a statically-allocated OfxPlugin struct inside the library.
            let plugin_ptr: *mut OfxPlugin = unsafe { get_plugin(i) };
            if plugin_ptr.is_null() {
                warn!("  Plugin #{i}: OfxGetPlugin returned null, skipping");
                continue;
            }

            let plugin = unsafe { &*plugin_ptr };

            let api = unsafe { CStr::from_ptr(plugin.pluginApi) }
                .to_string_lossy().into_owned();
            let identifier = unsafe { CStr::from_ptr(plugin.pluginIdentifier) }
                .to_string_lossy().into_owned();

            debug!("  Plugin #{i}: {identifier} (API {api} v{})", plugin.apiVersion);

            let info = OfxPluginInfo {
                api,
                api_version:   plugin.apiVersion,
                identifier,
                version_major: plugin.pluginVersionMajor,
                version_minor: plugin.pluginVersionMinor,
                library_index: i as usize,
            };

            // Call setHost so the plugin knows about our host.
            unsafe { (plugin.setHost)(host.as_ptr()) };

            plugins.push((info, plugin_ptr));
        }

        Ok(Self { _lib: lib, path: path.to_path_buf(), plugins })
    }

    pub fn plugin_count(&self) -> usize { self.plugins.len() }

    pub fn plugin_info(&self, index: usize) -> Option<&OfxPluginInfo> {
        self.plugins.get(index).map(|(info, _)| info)
    }

    /// Call the `OfxActionLoad` action on plugin `index`.
    pub fn action_load(&self, index: usize) -> Result<(), OfxError> {
        self.call_action(index, crate::ffi::kOfxActionLoad)
    }

    /// Call the `OfxActionUnload` action on plugin `index`.
    pub fn action_unload(&self, index: usize) -> Result<(), OfxError> {
        self.call_action(index, crate::ffi::kOfxActionUnload)
    }

    /// Call the `OfxActionDescribe` action on plugin `index`.
    pub fn action_describe(&self, index: usize) -> Result<(), OfxError> {
        self.call_action(index, crate::ffi::kOfxActionDescribe)
    }

    // ── Internal ──────────────────────────────────────────────────────────────

    fn call_action(&self, index: usize, action: &[u8]) -> Result<(), OfxError> {
        let (info, plugin_ptr) = self.plugins.get(index)
            .ok_or_else(|| OfxError::NullHandle)?;
        let plugin = unsafe { &**plugin_ptr };

        // SAFETY: `action` is a null-terminated byte string; `plugin` is valid
        // for the lifetime of `_lib`.
        let status = unsafe {
            (plugin.mainEntry)(
                action.as_ptr() as *const _,
                std::ptr::null_mut(), // handle (none for global actions)
                std::ptr::null_mut(), // inArgs
                std::ptr::null_mut(), // outArgs
            )
        };

        if status != crate::ffi::kOfxStatOK && status != crate::ffi::kOfxStatReplyDefault {
            return Err(OfxError::ActionFailed {
                action: String::from_utf8_lossy(action).into_owned(),
                code:   status,
            });
        }
        Ok(())
    }
}

use libc::c_int;
