//! Shared state and type definitions for the terminal.
//!
//! The C original keeps all of this in file-scope globals.  Here it lives in a
//! single `App` value owned by `main`; the XIM destroy callbacks reach it
//! through `APP`.

#![allow(non_upper_case_globals)]

use std::collections::VecDeque;
use std::ffi::CString;
use std::ptr;
use std::sync::atomic::{AtomicI32, AtomicPtr, Ordering};
use std::time::Instant;

use x11::xft::{XftColor, XftDraw, XftFont, XftGlyphFontSpec};
use x11::xlib::{
    Colormap, Display, Pixmap, Visual, Window, XPoint, XSetWindowAttributes, GC, XIC, XIM,
};
use x11::xrender::XRenderColor;

use crate::ffi::{FcFontSet, FcPattern, XVaNestedList};
use crate::x::Options;

// Arbitrary sizes
pub const UTF_INVALID: u32 = 0xFFFD;
pub const UTF_SIZ: usize = 4;
pub const ESC_BUF_SIZ: usize = 128 * UTF_SIZ;
pub const ESC_ARG_SIZ: usize = 16;
pub const STR_BUF_SIZ: usize = ESC_BUF_SIZ;
pub const STR_ARG_SIZ: usize = ESC_ARG_SIZ;

// term_mode
pub const MODE_WRAP: i32 = 1 << 0;
pub const MODE_INSERT: i32 = 1 << 1;
pub const MODE_ALTSCREEN: i32 = 1 << 2;
pub const MODE_CRLF: i32 = 1 << 3;
pub const MODE_ECHO: i32 = 1 << 4;
pub const MODE_PRINT: i32 = 1 << 5;
pub const MODE_UTF8: i32 = 1 << 6;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CursorMovement {
    Save,
    Load,
}

// cursor_state
pub const CURSOR_DEFAULT: i32 = 0;
pub const CURSOR_WRAPNEXT: i32 = 1;
pub const CURSOR_ORIGIN: i32 = 2;

// charset
pub const CS_GRAPHIC0: u8 = 0;
pub const CS_GRAPHIC1: u8 = 1;
pub const CS_UK: u8 = 2;
pub const CS_USA: u8 = 3;
pub const CS_MULTI: u8 = 4;
pub const CS_GER: u8 = 5;
pub const CS_FIN: u8 = 6;

// escape_state
pub const ESC_START: i32 = 1;
pub const ESC_CSI: i32 = 2;
pub const ESC_STR: i32 = 4; // DCS, OSC, PM, APC
pub const ESC_ALTCHARSET: i32 = 8;
pub const ESC_STR_END: i32 = 16; // a final string was encountered
pub const ESC_TEST: i32 = 32; // Enter in test mode
pub const ESC_UTF8: i32 = 64;

// win_mode
pub const MODE_VISIBLE: u32 = 1 << 0;
pub const MODE_FOCUSED: u32 = 1 << 1;
pub const MODE_APPKEYPAD: u32 = 1 << 2;
pub const MODE_MOUSEBTN: u32 = 1 << 3;
pub const MODE_MOUSEMOTION: u32 = 1 << 4;
pub const MODE_REVERSE: u32 = 1 << 5;
pub const MODE_KBDLOCK: u32 = 1 << 6;
pub const MODE_HIDE: u32 = 1 << 7;
pub const MODE_APPCURSOR: u32 = 1 << 8;
pub const MODE_MOUSESGR: u32 = 1 << 9;
pub const MODE_8BIT: u32 = 1 << 10;
pub const MODE_BLINK: u32 = 1 << 11;
pub const MODE_FBLINK: u32 = 1 << 12;
pub const MODE_FOCUS: u32 = 1 << 13;
pub const MODE_MOUSEX10: u32 = 1 << 14;
pub const MODE_MOUSEMANY: u32 = 1 << 15;
pub const MODE_BRCKTPASTE: u32 = 1 << 16;
pub const MODE_NUMLOCK: u32 = 1 << 17;
pub const MODE_MOUSE: u32 = MODE_MOUSEBTN | MODE_MOUSEMOTION | MODE_MOUSEX10 | MODE_MOUSEMANY;

// glyph_attribute
pub const ATTR_NULL: u16 = 0;
pub const ATTR_BOLD: u16 = 1 << 0;
pub const ATTR_FAINT: u16 = 1 << 1;
pub const ATTR_ITALIC: u16 = 1 << 2;
pub const ATTR_UNDERLINE: u16 = 1 << 3;
pub const ATTR_BLINK: u16 = 1 << 4;
pub const ATTR_REVERSE: u16 = 1 << 5;
pub const ATTR_INVISIBLE: u16 = 1 << 6;
pub const ATTR_STRUCK: u16 = 1 << 7;
pub const ATTR_WRAP: u16 = 1 << 8;
pub const ATTR_WIDE: u16 = 1 << 9;
pub const ATTR_WDUMMY: u16 = 1 << 10;
pub const ATTR_URL: u16 = 1 << 11; // underlined by the URL highlighter
pub const ATTR_BOLD_FAINT: u16 = ATTR_BOLD | ATTR_FAINT;

// selection_mode
pub const SEL_IDLE: i32 = 0;
pub const SEL_EMPTY: i32 = 1;
pub const SEL_READY: i32 = 2;

// selection_type
pub const SEL_REGULAR: i32 = 1;
pub const SEL_RECTANGULAR: i32 = 2;

// selection_snap
pub const SNAP_WORD: i32 = 1;
pub const SNAP_LINE: i32 = 2;

pub type Rune = u32;
pub type Line = Vec<Glyph>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Glyph {
    pub u: Rune,
    pub mode: u16,
    pub fg: u32,
    pub bg: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct Arg(pub ArgVal);

#[derive(Clone, Copy, Debug)]
pub enum ArgVal {
    I(i32),
    #[allow(dead_code)]
    U(u32),
    F(f32),
    S(&'static [u8]),
}

impl Arg {
    pub const fn i(v: i32) -> Arg {
        Arg(ArgVal::I(v))
    }
    pub const fn f(v: f32) -> Arg {
        Arg(ArgVal::F(v))
    }
    pub const fn s(v: &'static [u8]) -> Arg {
        Arg(ArgVal::S(v))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TCursor {
    pub attr: Glyph,
    pub x: i32,
    pub y: i32,
    pub state: i32,
}

impl Default for TCursor {
    fn default() -> Self {
        TCursor::new(0, 0)
    }
}

impl TCursor {
    pub const fn new(fg: u32, bg: u32) -> Self {
        TCursor {
            attr: Glyph {
                u: 0,
                mode: ATTR_NULL,
                fg,
                bg,
            },
            x: 0,
            y: 0,
            state: CURSOR_DEFAULT,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Coord {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct Selection {
    pub mode: i32,
    pub type_: i32,
    pub snap: i32,
    pub nb: Coord,
    pub ne: Coord,
    pub ob: Coord,
    pub oe: Coord,
    pub alt: i32,
}

impl Default for Selection {
    fn default() -> Self {
        Selection {
            mode: SEL_IDLE,
            type_: 0,
            snap: 0,
            nb: Coord::default(),
            ne: Coord::default(),
            ob: Coord::default(),
            oe: Coord::default(),
            alt: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Term {
    pub row: i32,
    pub col: i32,
    pub line: Vec<Line>,
    pub alt: Vec<Line>,
    /// Lines that scrolled off the top of the main screen, oldest first.
    pub hist: VecDeque<Line>,
    /// Number of history lines shown above the live screen (0 = bottom).
    pub scr: i32,
    pub dirty: Vec<bool>,
    pub c: TCursor,
    pub ocx: i32,
    pub ocy: i32,
    pub top: i32,
    pub bot: i32,
    pub mode: i32,
    pub esc: i32,
    pub trantbl: [u8; 4],
    pub charset: i32,
    pub icharset: i32,
    pub tabs: Vec<bool>,
    pub lastc: Rune,
}

impl Default for Term {
    fn default() -> Self {
        Term::new(0, 0)
    }
}

impl Term {
    pub fn new(fg: u32, bg: u32) -> Self {
        Term {
            row: 0,
            col: 0,
            line: Vec::new(),
            alt: Vec::new(),
            hist: VecDeque::new(),
            scr: 0,
            dirty: Vec::new(),
            c: TCursor::new(fg, bg),
            ocx: 0,
            ocy: 0,
            top: 0,
            bot: 0,
            mode: 0,
            esc: 0,
            trantbl: [0; 4],
            charset: 0,
            icharset: 0,
            tabs: Vec::new(),
            lastc: 0,
        }
    }
}

#[derive(Clone)]
pub struct CSIEscape {
    pub buf: [u8; ESC_BUF_SIZ],
    pub len: usize,
    pub priv_: u8,
    pub arg: [i32; ESC_ARG_SIZ],
    pub narg: usize,
    pub mode: [u8; 2],
}

impl Default for CSIEscape {
    fn default() -> Self {
        CSIEscape {
            buf: [0; ESC_BUF_SIZ],
            len: 0,
            priv_: 0,
            arg: [0; ESC_ARG_SIZ],
            narg: 0,
            mode: [0; 2],
        }
    }
}

#[derive(Clone)]
pub struct STREscape {
    pub type_: u8,
    pub buf: Vec<u8>,
    pub len: usize,
    pub args: [Vec<u8>; STR_ARG_SIZ],
    pub narg: usize,
}

impl Default for STREscape {
    fn default() -> Self {
        STREscape {
            type_: 0,
            buf: Vec::new(),
            len: 0,
            args: std::array::from_fn(|_| Vec::new()),
            narg: 0,
        }
    }
}

impl STREscape {
    pub fn reset(&mut self) {
        self.type_ = 0;
        self.len = 0;
        self.narg = 0;
        // The next sequence starts writing at the beginning of the buffer,
        // like `strescseq.len = 0` in the C original.
        self.buf.clear();
        for a in self.args.iter_mut() {
            a.clear();
        }
        // Keep the allocation, like xrealloc() in the original.
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TermWindow {
    pub tw: i32, // tty width
    pub th: i32, // tty height
    pub w: i32,  // window width
    pub h: i32,  // window height
    pub ch: i32, // char height
    pub cw: i32, // char width
    pub mode: u32,
    pub cursor: u32,
}

#[derive(Clone, Copy)]
pub struct Font {
    pub height: i32,
    pub width: i32,
    pub ascent: i32,
    pub descent: i32,
    pub badslant: bool,
    pub badweight: bool,
    pub lbearing: i16,
    pub rbearing: i16,
    pub match_: *mut XftFont,
    pub set: *mut FcFontSet,
    pub pattern: *mut FcPattern,
}

impl Default for Font {
    fn default() -> Self {
        Font {
            height: 0,
            width: 0,
            ascent: 0,
            descent: 0,
            badslant: false,
            badweight: false,
            lbearing: 0,
            rbearing: 0,
            match_: ptr::null_mut(),
            set: ptr::null_mut(),
            pattern: ptr::null_mut(),
        }
    }
}

pub struct DC {
    pub col: Vec<XftColor>,
    pub collen: usize,
    pub font: Font,
    pub bfont: Font,
    pub ifont: Font,
    pub ibfont: Font,
    pub gc: GC,
}

impl Default for DC {
    fn default() -> Self {
        DC {
            col: Vec::new(),
            collen: 0,
            font: Font::default(),
            bfont: Font::default(),
            ifont: Font::default(),
            ibfont: Font::default(),
            gc: ptr::null_mut(),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Ime {
    pub xim: XIM,
    pub xic: XIC,
    pub spot: XPoint,
    pub spotlist: XVaNestedList,
}

impl Default for Ime {
    fn default() -> Self {
        Ime {
            xim: ptr::null_mut(),
            xic: ptr::null_mut(),
            spot: XPoint { x: 0, y: 0 },
            spotlist: ptr::null_mut(),
        }
    }
}

pub struct XWindow {
    pub dpy: *mut Display,
    pub cmap: Colormap,
    pub win: Window,
    pub buf: Pixmap,
    pub specbuf: Vec<XftGlyphFontSpec>,
    pub xembed: u64,
    pub wmdeletewin: u64,
    pub netwmname: u64,
    pub netwmiconname: u64,
    pub netwmpid: u64,
    pub xdnd_type_list: u64,
    pub xdnd_selection: u64,
    pub xdnd_enter: u64,
    pub xdnd_position: u64,
    pub xdnd_status: u64,
    pub xdnd_drop: u64,
    pub xdnd_finished: u64,
    pub xdnd_action_copy: u64,
    pub xtext_uri_list: u64,
    pub xtext_plain: u64,
    pub xdnd_aware: u64,
    pub xdnd_source_win: Window,
    pub xdnd_source_version: i64,
    pub xdnd_source_format: u64,
    pub ime: Ime,
    pub draw: *mut XftDraw,
    pub vis: *mut Visual,
    pub attrs: XSetWindowAttributes,
    pub scr: i32,
    pub isfixed: bool,
    pub depth: i32, // bit depth
    pub l: i32,
    pub t: i32,
    pub gm: i32,
}

impl Default for XWindow {
    fn default() -> Self {
        XWindow {
            dpy: ptr::null_mut(),
            cmap: 0,
            win: 0,
            buf: 0,
            specbuf: Vec::new(),
            xembed: 0,
            wmdeletewin: 0,
            netwmname: 0,
            netwmiconname: 0,
            netwmpid: 0,
            xdnd_type_list: 0,
            xdnd_selection: 0,
            xdnd_enter: 0,
            xdnd_position: 0,
            xdnd_status: 0,
            xdnd_drop: 0,
            xdnd_finished: 0,
            xdnd_action_copy: 0,
            xtext_uri_list: 0,
            xtext_plain: 0,
            xdnd_aware: 0,
            xdnd_source_win: 0,
            xdnd_source_version: 0,
            xdnd_source_format: 0,
            ime: Ime::default(),
            draw: ptr::null_mut(),
            vis: ptr::null_mut(),
            attrs: unsafe { std::mem::zeroed() },
            scr: 0,
            isfixed: false,
            depth: 0,
            l: 0,
            t: 0,
            gm: 0,
        }
    }
}

pub struct XSelection {
    pub xtarget: u64,
    pub primary: Option<Vec<u8>>,
    pub clipboard: Option<Vec<u8>>,
    pub tclick1: Instant,
    pub tclick2: Instant,
}

impl Default for XSelection {
    fn default() -> Self {
        XSelection {
            xtarget: 0,
            primary: None,
            clipboard: None,
            tclick1: Instant::now(),
            tclick2: Instant::now(),
        }
    }
}

pub struct Fontcache {
    pub font: *mut XftFont,
    pub flags: i32,
    pub unicodep: Rune,
}

// Font ring cache flags
pub const FRC_NORMAL: i32 = 0;
pub const FRC_ITALIC: i32 = 1;
pub const FRC_BOLD: i32 = 2;
pub const FRC_ITALICBOLD: i32 = 3;

#[derive(Default)]
pub struct App {
    pub term: Term,
    pub sel: Selection,
    pub csiescseq: CSIEscape,
    pub strescseq: STREscape,
    pub saved_cursors: [TCursor; 2],
    pub iofd: i32,
    pub ttybuf: Vec<u8>,
    pub ttybuflen: usize,

    pub xw: XWindow,
    pub dc: DC,
    pub xsel: XSelection,
    pub win: TermWindow,
    pub frc: Vec<Fontcache>,

    pub usedfont: CString,
    pub usedfontsize: f64,
    pub defaultfontsize: f64,

    pub opts: Options,
    pub allowaltscreen: bool,
    pub allowwindowops: bool,

    /// Opacity applied to the default background, from `config::ALPHA` and
    /// the `-A` option.
    pub alpha: f32,

    pub buttons: u32,
    pub mouse_ox: i32,
    pub mouse_oy: i32,

    /// Current cursor style blinks (styles 0, 1, 3, 5 and 7).
    pub cursorblinks: bool,
}

impl App {
    pub fn new(opts: Options) -> Self {
        let mut app = App {
            term: Term::new(crate::config::DEFAULTFG, crate::config::DEFAULTBG),
            ..Default::default()
        };
        app.iofd = 1;
        app.ttybuf = vec![0; libc::BUFSIZ as usize];
        app.usedfont = CString::new(crate::config::FONT).unwrap();
        app.opts = opts;
        app.defaultfontsize = 0.0;
        app.usedfontsize = 0.0;
        app.allowaltscreen = crate::config::ALLOWALTSCREEN;
        app.allowwindowops = crate::config::ALLOWWINDOWOPS;
        app.alpha = crate::config::ALPHA;
        app.ttybuflen = 0;
        app
    }

    #[inline]
    pub fn is_set(&self, flag: i32) -> bool {
        self.term.mode & flag != 0
    }

    #[inline]
    pub fn win_is_set(&self, flag: u32) -> bool {
        self.win.mode & flag != 0
    }
}

// ---------------------------------------------------------------------------
// Global handles used by C callbacks / signal handlers
// ---------------------------------------------------------------------------

pub static APP: AtomicPtr<App> = AtomicPtr::new(ptr::null_mut());
pub static PID: AtomicI32 = AtomicI32::new(0);
pub static CMDFD: AtomicI32 = AtomicI32::new(-1);

/// Access the global `App`.  Only used from XIM callbacks and signal-safe
/// helpers, where passing `&mut App` is impossible.
///
/// # Safety
/// The pointer is installed by `main` before any callback can run and stays
/// valid for the lifetime of the process.
pub unsafe fn app() -> &'static mut App {
    &mut *APP.load(Ordering::Relaxed)
}

#[inline]
pub fn min(a: i32, b: i32) -> i32 {
    if a < b {
        a
    } else {
        b
    }
}

#[inline]
pub fn max(a: i32, b: i32) -> i32 {
    if a < b {
        b
    } else {
        a
    }
}

#[inline]
pub fn between(x: i32, a: i32, b: i32) -> bool {
    a <= x && x <= b
}

#[inline]
pub fn limit(x: i32, a: i32, b: i32) -> i32 {
    if x < a {
        a
    } else if x > b {
        b
    } else {
        x
    }
}

#[inline]
pub fn cdiv(n: i32, d: i32) -> i32 {
    (n + (d - 1)) / d
}

pub fn is_control_c0(c: u32) -> bool {
    between(c as i32, 0, 0x1f) || c == 0x7f
}

pub fn is_control_c1(c: u32) -> bool {
    between(c as i32, 0x80, 0x9f)
}

pub fn is_control(c: u32) -> bool {
    is_control_c0(c) || is_control_c1(c)
}

pub fn is_delim(u: u32) -> bool {
    u != 0 && crate::config::WORDDELIMITERS.contains(&u)
}

/// C's `atoi(3)`.
pub fn c_atoi(s: &[u8]) -> i32 {
    let mut i = 0;
    while i < s.len() && (s[i] == b' ' || (0x09..=0x0d).contains(&s[i])) {
        i += 1;
    }
    let mut sign = 1i64;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        if s[i] == b'-' {
            sign = -1;
        }
        i += 1;
    }
    let mut v: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        v = v.saturating_mul(10).saturating_add((s[i] - b'0') as i64);
        i += 1;
    }
    (sign * v) as i32
}

/// Make a `CString`, truncating at the first NUL like the C originals would.
pub fn cstring(s: &[u8]) -> CString {
    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    CString::new(&s[..end]).unwrap()
}

pub fn die(msg: &str) -> ! {
    eprint!("{}", msg);
    std::process::exit(1);
}

pub fn truecolor(r: u32, g: u32, b: u32) -> u32 {
    (1 << 24) | (r << 16) | (g << 8) | b
}

#[inline]
pub fn is_truecolor(x: u32) -> bool {
    (1 << 24) & x != 0
}

pub fn xrender_color(red: u16, green: u16, blue: u16, alpha: u16) -> XRenderColor {
    XRenderColor {
        red,
        green,
        blue,
        alpha,
    }
}
