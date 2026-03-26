//! Hand-written FFI bindings for the OpenFX C API.
//!
//! We only define the subset required to load plugins and call their
//! `describe` / `describeInContext` / `render` actions.
//!
//! Full header: <https://github.com/ofxa/openfx/blob/master/include/ofxCore.h>

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use libc::{c_char, c_int, c_void};

// ── Fundamental OFX types ─────────────────────────────────────────────────────

/// Status codes returned by OFX functions.
pub type OfxStatus = c_int;

pub const kOfxStatOK:                OfxStatus =  0;
pub const kOfxStatFailed:            OfxStatus =  1;
pub const kOfxStatErrFatal:          OfxStatus =  2;
pub const kOfxStatErrUnknown:        OfxStatus =  3;
pub const kOfxStatErrMissingHostFeature: OfxStatus = 4;
pub const kOfxStatErrUnsupported:    OfxStatus =  5;
pub const kOfxStatErrExists:         OfxStatus =  6;
pub const kOfxStatErrFormat:         OfxStatus =  7;
pub const kOfxStatErrMemory:         OfxStatus =  8;
pub const kOfxStatErrBadHandle:      OfxStatus =  9;
pub const kOfxStatErrBadIndex:       OfxStatus = 10;
pub const kOfxStatErrValue:          OfxStatus = 11;
pub const kOfxStatReplyYes:          OfxStatus = 12;
pub const kOfxStatReplyNo:           OfxStatus = 13;
pub const kOfxStatReplyDefault:      OfxStatus = 14;

/// Opaque handle type used throughout the OFX API.
pub type OfxHandle = *mut c_void;

/// OFX property set handle.
pub type OfxPropertySetHandle = *mut c_void;

/// OFX image-effect handle.
pub type OfxImageEffectHandle = *mut c_void;

// ── Suite names (C string literals) ──────────────────────────────────────────

pub const kOfxPropertySuite:     &[u8] = b"OfxPropertySuite\0";
pub const kOfxImageEffectSuite:  &[u8] = b"OfxImageEffectSuite\0";
pub const kOfxParameterSuite:    &[u8] = b"OfxParameterSuite\0";
pub const kOfxMemorySuite:       &[u8] = b"OfxMemorySuite\0";
pub const kOfxMultiThreadSuite:  &[u8] = b"OfxMultiThreadSuite\0";
pub const kOfxMessageSuite:      &[u8] = b"OfxMessageSuite\0";

// ── OfxHost ───────────────────────────────────────────────────────────────────

/// The host struct passed to every plugin on load.
/// The plugin calls `host->fetchSuite(...)` to obtain suite pointers.
#[repr(C)]
pub struct OfxHost {
    /// Opaque host data pointer (unused by plugins, may carry host state).
    pub host: OfxPropertySetHandle,
    /// `fetchSuite(host, suiteName, suiteVersion) → *void`
    pub fetchSuite: unsafe extern "C" fn(
        host:         OfxPropertySetHandle,
        suiteName:    *const c_char,
        suiteVersion: c_int,
    ) -> *const c_void,
}

// ── OfxPlugin ─────────────────────────────────────────────────────────────────

/// Entry-point struct exported by every OFX plugin.
#[repr(C)]
pub struct OfxPlugin {
    /// String identifying the plugin API, e.g. `"OfxImageEffectPluginAPI"`.
    pub pluginApi:          *const c_char,
    /// API version implemented.
    pub apiVersion:         c_int,
    /// Unique identifier, e.g. `"com.example.MyBlur"`.
    pub pluginIdentifier:   *const c_char,
    /// Plugin version (major).
    pub pluginVersionMajor: c_int,
    /// Plugin version (minor).
    pub pluginVersionMinor: c_int,
    /// Called once after the library is loaded.
    pub setHost:            unsafe extern "C" fn(host: *const OfxHost),
    /// Called to dispatch actions (describe, render, …).
    pub mainEntry:          unsafe extern "C" fn(
        action:          *const c_char,
        handle:          OfxHandle,
        inArgs:          OfxPropertySetHandle,
        outArgs:         OfxPropertySetHandle,
    ) -> OfxStatus,
}

// ── Library entry-points ──────────────────────────────────────────────────────

/// `OfxGetNumberOfPlugins()` – exported by every .ofx binary.
pub type OfxGetNumberOfPluginsFn = unsafe extern "C" fn() -> c_int;

/// `OfxGetPlugin(nth)` – returns a pointer to the nth OfxPlugin struct.
pub type OfxGetPluginFn = unsafe extern "C" fn(nth: c_int) -> *mut OfxPlugin;

// ── Action name constants ─────────────────────────────────────────────────────

pub const kOfxActionLoad:              &[u8] = b"OfxActionLoad\0";
pub const kOfxActionUnload:            &[u8] = b"OfxActionUnload\0";
pub const kOfxActionDescribe:          &[u8] = b"OfxActionDescribe\0";
pub const kOfxImageEffectActionDescribeInContext: &[u8] =
    b"OfxImageEffectActionDescribeInContext\0";
pub const kOfxImageEffectActionRender: &[u8] = b"OfxImageEffectActionRender\0";
pub const kOfxActionCreateInstance:    &[u8] = b"OfxActionCreateInstance\0";
pub const kOfxActionDestroyInstance:   &[u8] = b"OfxActionDestroyInstance\0";

// ── Property suite ────────────────────────────────────────────────────────────

/// The minimal property suite we implement host-side.
#[repr(C)]
pub struct OfxPropertySuiteV1 {
    pub propSetPointer:   unsafe extern "C" fn(
        properties: OfxPropertySetHandle, property: *const c_char,
        index: c_int, value: *mut c_void,
    ) -> OfxStatus,
    pub propSetString:    unsafe extern "C" fn(
        properties: OfxPropertySetHandle, property: *const c_char,
        index: c_int, value: *const c_char,
    ) -> OfxStatus,
    pub propSetDouble:    unsafe extern "C" fn(
        properties: OfxPropertySetHandle, property: *const c_char,
        index: c_int, value: f64,
    ) -> OfxStatus,
    pub propSetInt:       unsafe extern "C" fn(
        properties: OfxPropertySetHandle, property: *const c_char,
        index: c_int, value: c_int,
    ) -> OfxStatus,
    pub propGetPointer:   unsafe extern "C" fn(
        properties: OfxPropertySetHandle, property: *const c_char,
        index: c_int, value: *mut *mut c_void,
    ) -> OfxStatus,
    pub propGetString:    unsafe extern "C" fn(
        properties: OfxPropertySetHandle, property: *const c_char,
        index: c_int, value: *mut *const c_char,
    ) -> OfxStatus,
    pub propGetDouble:    unsafe extern "C" fn(
        properties: OfxPropertySetHandle, property: *const c_char,
        index: c_int, value: *mut f64,
    ) -> OfxStatus,
    pub propGetInt:       unsafe extern "C" fn(
        properties: OfxPropertySetHandle, property: *const c_char,
        index: c_int, value: *mut c_int,
    ) -> OfxStatus,
    pub propReset:        unsafe extern "C" fn(
        properties: OfxPropertySetHandle, property: *const c_char,
    ) -> OfxStatus,
    pub propGetDimension: unsafe extern "C" fn(
        properties: OfxPropertySetHandle, property: *const c_char,
        count: *mut c_int,
    ) -> OfxStatus,
}
