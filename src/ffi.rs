//! Extra FFI declarations that the `x11` crate does not provide.
//!
//! fontconfig functions are not exposed by x11-rs, and a handful of Xlib
//! entry points are variadic (and therefore declared here so the arguments
//! can be passed correctly).

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use libc::{c_char, c_double, c_int, c_uint, c_void};
use x11::xlib::{Display, XPointer, XIC, XIM};

// ---------------------------------------------------------------------------
// fontconfig (opaque types mirror x11::xft's, pointers are interchangeable)
// ---------------------------------------------------------------------------

pub enum FcCharSet {}
pub enum FcFontSet {}
pub enum FcPattern {}
pub enum FcConfig {}

/// `FcResult`: Match, NoMatch, TypeMismatch, NoId, OutOfMemory
pub const FC_RESULT_MATCH: c_int = 0;

/// `FcMatchKind`: FcMatchPattern, FcMatchFont, FcMatchScan
pub const FC_MATCH_PATTERN: c_int = 0;

/// `FcFalse`/`FcTrue`
pub const FC_FALSE: c_int = 0;
pub const FC_TRUE: c_int = 1;

pub const FC_FAMILY: &[u8] = b"family\0";
pub const FC_SLANT: &[u8] = b"slant\0";
pub const FC_WEIGHT: &[u8] = b"weight\0";
pub const FC_SIZE: &[u8] = b"size\0";
pub const FC_PIXEL_SIZE: &[u8] = b"pixelsize\0";
pub const FC_CHARSET: &[u8] = b"charset\0";
pub const FC_SCALABLE: &[u8] = b"scalable\0";

pub const FC_SLANT_ROMAN: c_int = 0;
pub const FC_SLANT_ITALIC: c_int = 100;
pub const FC_WEIGHT_BOLD: c_int = 200;

#[link(name = "fontconfig")]
extern "C" {
    pub fn FcInit() -> c_int;
    pub fn FcPatternDuplicate(p: *const FcPattern) -> *mut FcPattern;
    pub fn FcPatternDestroy(p: *mut FcPattern);
    pub fn FcPatternDel(p: *mut FcPattern, object: *const c_char) -> c_int;
    pub fn FcPatternAddDouble(p: *mut FcPattern, object: *const c_char, d: c_double) -> c_int;
    pub fn FcPatternGetDouble(
        p: *mut FcPattern,
        object: *const c_char,
        n: c_int,
        d: *mut c_double,
    ) -> c_int;
    pub fn FcPatternAddInteger(p: *mut FcPattern, object: *const c_char, i: c_int) -> c_int;
    pub fn FcPatternGetInteger(
        p: *mut FcPattern,
        object: *const c_char,
        n: c_int,
        i: *mut c_int,
    ) -> c_int;
    pub fn FcPatternAddCharSet(
        p: *mut FcPattern,
        object: *const c_char,
        cs: *const FcCharSet,
    ) -> c_int;
    pub fn FcPatternAddBool(p: *mut FcPattern, object: *const c_char, b: c_int) -> c_int;
    pub fn FcConfigSubstitute(cfg: *mut FcConfig, p: *mut FcPattern, kind: c_int) -> c_int;
    pub fn FcDefaultSubstitute(p: *mut FcPattern);
    pub fn FcFontMatch(cfg: *mut FcConfig, p: *mut FcPattern, result: *mut c_int)
        -> *mut FcPattern;
    pub fn FcFontSort(
        cfg: *mut FcConfig,
        p: *mut FcPattern,
        trim: c_int,
        csp: *mut *mut FcCharSet,
        result: *mut c_int,
    ) -> *mut FcFontSet;
    pub fn FcFontSetDestroy(s: *mut FcFontSet);
    pub fn FcFontSetMatch(
        cfg: *mut FcConfig,
        sets: *mut *mut FcFontSet,
        nsets: c_int,
        p: *mut FcPattern,
        result: *mut c_int,
    ) -> *mut FcPattern;
    pub fn FcCharSetCreate() -> *mut FcCharSet;
    pub fn FcCharSetDestroy(s: *mut FcCharSet);
    pub fn FcCharSetAddChar(s: *mut FcCharSet, ucs4: c_uint) -> c_int;
    pub fn FcNameParse(name: *const c_char) -> *mut FcPattern;
}

// ---------------------------------------------------------------------------
// Variadic Xlib entry points
// ---------------------------------------------------------------------------

pub type XVaNestedList = *mut c_void;

pub const XNDestroyCallback: &[u8] = b"destroyCallback\0";
pub const XNInputStyle: &[u8] = b"inputStyle\0";
pub const XNClientWindow: &[u8] = b"clientWindow\0";
pub const XNPreeditAttributes: &[u8] = b"preeditAttributes\0";
pub const XNSpotLocation: &[u8] = b"spotLocation\0";

extern "C" {
    pub fn XVaCreateNestedList(unused: c_int, ...) -> XVaNestedList;
    pub fn XCreateIC(im: XIM, ...) -> XIC;
    pub fn XSetICValues(ic: XIC, ...) -> *mut c_char;
    pub fn XSetIMValues(im: XIM, ...) -> *mut c_char;
    pub fn XRegisterIMInstantiateCallback(
        dpy: *mut Display,
        rdatabase: *mut c_void,
        res_name: *mut c_char,
        res_class: *mut c_char,
        callback: Option<unsafe extern "C" fn(*mut Display, XPointer, XPointer)>,
        client_data: XPointer,
    ) -> c_int;
    pub fn XUnregisterIMInstantiateCallback(
        dpy: *mut Display,
        rdatabase: *mut c_void,
        res_name: *mut c_char,
        res_class: *mut c_char,
        callback: Option<unsafe extern "C" fn(*mut Display, XPointer, XPointer)>,
        client_data: XPointer,
    ) -> c_int;
}
