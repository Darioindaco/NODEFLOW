//! Rust-side OFX **host** implementation.
//!
//! The host is the side that *runs* plugins.  It must:
//! 1. Supply an [`OfxHost`] struct to plugins on load.
//! 2. Implement the suite functions (Property, ImageEffect, …) that plugins
//!    call back into.
//!
//! Our host stores per-handle property bags in a `HashMap` so plugin calls to
//! `propSet*` / `propGet*` work correctly.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::sync::{Arc, Mutex};

use libc::{c_char, c_int, c_void};

use crate::ffi::{self, OfxPropertySetHandle, OfxStatus};

// ── Property bag ─────────────────────────────────────────────────────────────

/// One value that can live inside an OFX property set.
#[derive(Debug, Clone)]
pub enum PropValue {
    Pointer(*mut c_void),
    Str(String),
    Double(f64),
    Int(i32),
}

// SAFETY: We never actually send these across threads – the bag is behind a
// Mutex and only accessed from the same thread that called the OFX action.
unsafe impl Send for PropValue {}
unsafe impl Sync for PropValue {}

/// A single OFX property set: `name → Vec<PropValue>` (OFX supports indexed
/// properties, e.g. an RGB triple stored at indices 0..2).
type PropBag = HashMap<String, Vec<PropValue>>;

// ── Shared host state ─────────────────────────────────────────────────────────

/// All mutable host-side state behind one `Mutex` so we can hand out a raw
/// `*mut c_void` handle to the C side while keeping Rust ownership.
pub(crate) struct HostState {
    /// Map from opaque handle value (cast from pointer) to its property bag.
    pub props: HashMap<usize, PropBag>,
    /// Our property suite struct – lives as long as HostState.
    pub prop_suite: Box<ffi::OfxPropertySuiteV1>,
}

impl HostState {
    fn new() -> Self {
        Self {
            props: HashMap::new(),
            prop_suite: Box::new(build_property_suite()),
        }
    }

    pub fn ensure_bag(&mut self, handle: OfxPropertySetHandle) -> &mut PropBag {
        self.props.entry(handle as usize).or_default()
    }

    pub fn get_bag(&self, handle: OfxPropertySetHandle) -> Option<&PropBag> {
        self.props.get(&(handle as usize))
    }
}

// ── OfxHost ───────────────────────────────────────────────────────────────────

/// The NodeFlow OFX host.
pub struct OfxHost {
    /// The C-ABI struct we pass to plugins.
    pub(crate) c_host: Box<ffi::OfxHost>,
    /// Shared mutable state accessed by the suite callbacks.
    pub(crate) state:  Arc<Mutex<HostState>>,
    /// Version of the host API.
    pub version: (u32, u32),
}

impl OfxHost {
    pub fn new() -> Arc<Self> {
        let state = Arc::new(Mutex::new(HostState::new()));

        // We store a raw pointer to `state` in the C host struct so that the
        // static suite functions can reach it via the `host` field.
        // SAFETY: `Arc::into_raw` keeps the allocation alive; we convert back
        // in `fetch_suite` without dropping the Arc.
        let state_ptr = Arc::as_ptr(&state) as *mut c_void;

        let c_host = Box::new(ffi::OfxHost {
            host:       state_ptr as OfxPropertySetHandle,
            fetchSuite: fetch_suite,
        });

        Arc::new(Self { c_host, state, version: (1, 4) })
    }

    /// Raw pointer to the C host struct; pass to `plugin.setHost()`.
    pub fn as_ptr(&self) -> *const ffi::OfxHost {
        self.c_host.as_ref() as *const _
    }
}

// OfxHost is always heap-allocated in an Arc; no Default impl needed.

// ── fetchSuite callback ───────────────────────────────────────────────────────

/// Called by plugins to obtain a pointer to a named suite.
///
/// # Safety
/// `host` must be the `Arc<Mutex<HostState>>` raw pointer we stored above.
unsafe extern "C" fn fetch_suite(
    host:          OfxPropertySetHandle,
    suite_name:    *const c_char,
    suite_version: c_int,
) -> *const c_void {
    // Reconstruct the Arc without dropping it.
    // SAFETY: The Arc was created with `Arc::into_raw`-equivalent semantics and
    // the host outlives all plugin calls.
    let state_arc = Arc::from_raw(host as *const Mutex<HostState>);
    let result = {
        let guard = state_arc.lock().unwrap();
        let name = CStr::from_ptr(suite_name).to_string_lossy();
        match name.as_ref() {
            "OfxPropertySuite" if suite_version == 1 => {
                guard.prop_suite.as_ref() as *const ffi::OfxPropertySuiteV1 as *const c_void
            }
            _ => {
                tracing::warn!("OFX: fetchSuite({name} v{suite_version}) – not implemented");
                std::ptr::null()
            }
        }
    };
    // Forget so the Arc refcount isn't decremented.
    std::mem::forget(state_arc);
    result
}

// ── Property suite implementation ─────────────────────────────────────────────

/// Build the V1 property suite struct with function pointers to our callbacks.
fn build_property_suite() -> ffi::OfxPropertySuiteV1 {
    ffi::OfxPropertySuiteV1 {
        propSetPointer:   prop_set_pointer,
        propSetString:    prop_set_string,
        propSetDouble:    prop_set_double,
        propSetInt:       prop_set_int,
        propGetPointer:   prop_get_pointer,
        propGetString:    prop_get_string,
        propGetDouble:    prop_get_double,
        propGetInt:       prop_get_int,
        propReset:        prop_reset,
        propGetDimension: prop_get_dimension,
    }
}

// ── Helper macro for property set callbacks ───────────────────────────────────

/// Shared global state for the property suite callbacks.
/// We use a `once_cell`-style static; in production you'd thread it through the
/// host pointer instead (see `fetch_suite` above for the pattern).
/// For simplicity here we use a process-global Mutex<HashMap>.
static GLOBAL_PROPS: std::sync::OnceLock<Mutex<HashMap<usize, PropBag>>> =
    std::sync::OnceLock::new();

fn global_props() -> &'static Mutex<HashMap<usize, PropBag>> {
    GLOBAL_PROPS.get_or_init(|| Mutex::new(HashMap::new()))
}

unsafe extern "C" fn prop_set_string(
    props: OfxPropertySetHandle,
    name:  *const c_char,
    index: c_int,
    value: *const c_char,
) -> OfxStatus {
    let key = CStr::from_ptr(name).to_string_lossy().into_owned();
    let val = CStr::from_ptr(value).to_string_lossy().into_owned();
    let mut guard = global_props().lock().unwrap();
    let bag = guard.entry(props as usize).or_default();
    let vec = bag.entry(key).or_default();
    let idx = index as usize;
    if vec.len() <= idx { vec.resize(idx + 1, PropValue::Str(String::new())); }
    vec[idx] = PropValue::Str(val);
    ffi::kOfxStatOK
}

unsafe extern "C" fn prop_get_string(
    props: OfxPropertySetHandle,
    name:  *const c_char,
    index: c_int,
    out:   *mut *const c_char,
) -> OfxStatus {
    let key   = CStr::from_ptr(name).to_string_lossy().into_owned();
    let guard = global_props().lock().unwrap();
    if let Some(bag) = guard.get(&(props as usize)) {
        if let Some(vec) = bag.get(&key) {
            if let Some(PropValue::Str(s)) = vec.get(index as usize) {
                // Leak a CString – in a real host you'd manage this lifetime.
                let cs = CString::new(s.as_str()).unwrap();
                *out = cs.into_raw();
                return ffi::kOfxStatOK;
            }
        }
    }
    ffi::kOfxStatErrBadIndex
}

unsafe extern "C" fn prop_set_double(
    props: OfxPropertySetHandle, name: *const c_char, index: c_int, value: f64,
) -> OfxStatus {
    let key = CStr::from_ptr(name).to_string_lossy().into_owned();
    let mut guard = global_props().lock().unwrap();
    let vec = guard.entry(props as usize).or_default()
                   .entry(key).or_default();
    let idx = index as usize;
    if vec.len() <= idx { vec.resize(idx + 1, PropValue::Double(0.0)); }
    vec[idx] = PropValue::Double(value);
    ffi::kOfxStatOK
}

unsafe extern "C" fn prop_get_double(
    props: OfxPropertySetHandle, name: *const c_char, index: c_int, out: *mut f64,
) -> OfxStatus {
    let key   = CStr::from_ptr(name).to_string_lossy().into_owned();
    let guard = global_props().lock().unwrap();
    if let Some(bag) = guard.get(&(props as usize)) {
        if let Some(vec) = bag.get(&key) {
            if let Some(PropValue::Double(v)) = vec.get(index as usize) {
                *out = *v; return ffi::kOfxStatOK;
            }
        }
    }
    ffi::kOfxStatErrBadIndex
}

unsafe extern "C" fn prop_set_int(
    props: OfxPropertySetHandle, name: *const c_char, index: c_int, value: c_int,
) -> OfxStatus {
    let key = CStr::from_ptr(name).to_string_lossy().into_owned();
    let mut guard = global_props().lock().unwrap();
    let vec = guard.entry(props as usize).or_default()
                   .entry(key).or_default();
    let idx = index as usize;
    if vec.len() <= idx { vec.resize(idx + 1, PropValue::Int(0)); }
    vec[idx] = PropValue::Int(value);
    ffi::kOfxStatOK
}

unsafe extern "C" fn prop_get_int(
    props: OfxPropertySetHandle, name: *const c_char, index: c_int, out: *mut c_int,
) -> OfxStatus {
    let key   = CStr::from_ptr(name).to_string_lossy().into_owned();
    let guard = global_props().lock().unwrap();
    if let Some(bag) = guard.get(&(props as usize)) {
        if let Some(vec) = bag.get(&key) {
            if let Some(PropValue::Int(v)) = vec.get(index as usize) {
                *out = *v; return ffi::kOfxStatOK;
            }
        }
    }
    ffi::kOfxStatErrBadIndex
}

unsafe extern "C" fn prop_set_pointer(
    props: OfxPropertySetHandle, name: *const c_char, index: c_int, value: *mut c_void,
) -> OfxStatus {
    let key = CStr::from_ptr(name).to_string_lossy().into_owned();
    let mut guard = global_props().lock().unwrap();
    let vec = guard.entry(props as usize).or_default()
                   .entry(key).or_default();
    let idx = index as usize;
    if vec.len() <= idx { vec.resize(idx + 1, PropValue::Pointer(std::ptr::null_mut())); }
    vec[idx] = PropValue::Pointer(value);
    ffi::kOfxStatOK
}

unsafe extern "C" fn prop_get_pointer(
    props: OfxPropertySetHandle, name: *const c_char, index: c_int,
    out: *mut *mut c_void,
) -> OfxStatus {
    let key   = CStr::from_ptr(name).to_string_lossy().into_owned();
    let guard = global_props().lock().unwrap();
    if let Some(bag) = guard.get(&(props as usize)) {
        if let Some(vec) = bag.get(&key) {
            if let Some(PropValue::Pointer(v)) = vec.get(index as usize) {
                *out = *v; return ffi::kOfxStatOK;
            }
        }
    }
    ffi::kOfxStatErrBadIndex
}

unsafe extern "C" fn prop_reset(
    props: OfxPropertySetHandle, name: *const c_char,
) -> OfxStatus {
    let key = CStr::from_ptr(name).to_string_lossy().into_owned();
    let mut guard = global_props().lock().unwrap();
    if let Some(bag) = guard.get_mut(&(props as usize)) { bag.remove(&key); }
    ffi::kOfxStatOK
}

unsafe extern "C" fn prop_get_dimension(
    props: OfxPropertySetHandle, name: *const c_char, out: *mut c_int,
) -> OfxStatus {
    let key   = CStr::from_ptr(name).to_string_lossy().into_owned();
    let guard = global_props().lock().unwrap();
    if let Some(bag) = guard.get(&(props as usize)) {
        if let Some(vec) = bag.get(&key) {
            *out = vec.len() as c_int;
            return ffi::kOfxStatOK;
        }
    }
    *out = 0;
    ffi::kOfxStatOK
}
