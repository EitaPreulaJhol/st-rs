//! Configuration, mirroring the original `config.h`.
//!
//! Edit this file (and rebuild) to change fonts, colours, key bindings and
//! behaviour, exactly as with the C version.

#![allow(dead_code)]

use x11::xlib::*;

use crate::app::{App, Arg, Rune};

pub const XK_ANY_MOD: u32 = u32::MAX;
pub const XK_NO_MOD: u32 = 0;
pub const XK_SWITCH_MOD: u32 = (1 << 13) | (1 << 14);

/// Font used for the terminal.
pub const FONT: &str = "Iosevka:pixelsize=22:antialias=true:autohint=true";

/// Optional custom fonts for the bold, italic and bold-italic attributes.
///
/// Leave an entry empty to derive that style from `FONT` via fontconfig (the
/// original st behaviour).  When set, the font is used at the same pixel size
/// as `FONT`; only the family/style should differ.  Any fontconfig pattern or
/// XLFD string is accepted.  Examples:
///
/// ```ignore
/// pub const FONT_BOLD: &str = "DejaVu Sans Mono:style=Bold";
/// pub const FONT_ITALIC: &str = "DejaVu Sans Mono:style=Oblique";
/// pub const FONT_BOLD_ITALIC: &str = "DejaVu Sans Mono:style=Bold Oblique";
/// ```
pub const FONT_BOLD: &str = "";
pub const FONT_ITALIC: &str = "";
pub const FONT_BOLD_ITALIC: &str = "";

/// Runtime font zoom step (Ctrl+Plus / Ctrl+Minus) and limits, in pixels.
pub const FONT_SIZE_STEP: f32 = 1.0;
pub const FONT_SIZE_MIN: f32 = 4.0;
pub const FONT_SIZE_MAX: f32 = 72.0;
pub const BORDERPX: i32 = 2;

/// Program exec'd by st, in order of precedence: -e, scroll/utmp, SHELL,
/// passwd shell, this value.
pub const SHELL: &str = "/bin/sh";
pub const UTMP: Option<&str> = None;
pub const SCROLL: Option<&str> = None;
pub const STTY_ARGS: &str = "stty raw pass8 nl -echo -iexten -cstopb 38400";

/// Identification sequence returned in DA and DECID.
pub const VTIDEN: &[u8] = b"\x1b[?6c";

/// Kerning / character bounding-box multipliers.
pub const CWSCALE: f32 = 1.0;
pub const CHSCALE: f32 = 1.0;

/// Word delimiters (unicode code points).
pub const WORDDELIMITERS: &[u32] = &[0x20];

/// Selection timeouts, in milliseconds.
pub const DOUBLECLICKTIMEOUT: u64 = 300;
pub const TRIPLECLICKTIMEOUT: u64 = 600;

/// Program the URL under the cursor is passed to on Ctrl+Click.
pub const URLHANDLER: &str = "xdg-open";

/// Characters a URL may contain.  Highlighting and following stop at the
/// first cell holding anything else.
pub const URLCHARS: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~:/?#@!$&'*+,;=%";

/// Prefixes that start a URL.  Add e.g. `b"ftp://"` or `b"mailto:"` here.
pub static URLPREFIXES: &[&[u8]] = &[b"http://", b"https://"];

pub const ALLOWALTSCREEN: bool = true;
pub const ALLOWWINDOWOPS: bool = false;

/// Draw latency range in ms.
pub const MINLATENCY: f64 = 2.0;
pub const MAXLATENCY: f64 = 33.0;

pub const BLINKTIMEOUT: u64 = 800;
pub const CURSORTHICKNESS: u32 = 2;
pub const BELLVOLUME: i32 = 0;
pub const TERMNAME: &str = "st-256color";
pub const TABSPACES: u32 = 8;

/// Opacity of the default background (0.0 transparent, 1.0 opaque).
/// Only visible with a compositing manager.
pub const ALPHA: f32 = 0.8;

/// Vertical background gradient: the part of the opacity that fades from the
/// top of the window to the bottom, and the constant part added to it.  Cells
/// are drawn with `GRAD_ALPHA * (h - y*ch) / h + STAT_ALPHA`.
pub const GRAD_ALPHA: f32 = 0.54;
pub const STAT_ALPHA: f32 = 0.46;

/// Characters escaped with a backslash when a dropped file path is pasted.
pub const XDNDESCCHAR: &str = " !\"#$&'()*;<>?[\\]^`{|}~";

/// A terminal colour scheme.
///
/// `palette` holds the 16 ANSI colours.  The cursor, reverse cursor and the
/// default foreground/background are kept separately because they become
/// colour indices 256-259.
pub struct Theme {
    pub palette: [&'static str; 16],
    pub cursor: &'static str,
    pub reverse_cursor: &'static str,
    pub foreground: &'static str,
    pub background: &'static str,
    pub mouse_fg: usize,
    pub mouse_bg: usize,
}

/// White (light) theme, based on One Half Light.
pub const WHITE_THEME: Theme = Theme {
    palette: [
        "#383a42", // 0 black
        "#e45649", // 1 red
        "#50a14f", // 2 green
        "#c18401", // 3 yellow
        "#0184bc", // 4 blue
        "#a626a4", // 5 magenta
        "#0997b3", // 6 cyan
        "#a0a1a7", // 7 white
        "#4f525e", // 8 bright black
        "#e06c75", // 9 bright red
        "#98c379", // 10 bright green
        "#e5c07b", // 11 bright yellow
        "#61afef", // 12 bright blue
        "#c678dd", // 13 bright magenta
        "#56b6c2", // 14 bright cyan
        "#ffffff", // 15 bright white
    ],
    cursor: "#383a42",
    reverse_cursor: "#d0d0d0",
    foreground: "#383a42",
    background: "#ffffff",
    mouse_fg: 0,
    mouse_bg: 15,
};

/// The original st palette (dark).
pub const DARK_THEME: Theme = Theme {
    palette: [
        "black",    // 0
        "red3",     // 1
        "green3",   // 2
        "yellow3",  // 3
        "blue2",    // 4
        "magenta3", // 5
        "cyan3",    // 6
        "gray90",   // 7
        "gray50",   // 8
        "red",      // 9
        "green",    // 10
        "yellow",   // 11
        "#5c5cff",  // 12
        "magenta",  // 13
        "cyan",     // 14
        "white",    // 15
    ],
    cursor: "#cccccc",
    reverse_cursor: "#555555",
    foreground: "gray90",
    background: "black",
    mouse_fg: 7,
    mouse_bg: 0,
};

/// Active colour scheme.  Set this to `DARK_THEME` for the original look.
pub const THEME: Theme = WHITE_THEME;

/// Terminal colours (16 first used in escape sequence).
pub const COLORNAME: [Option<&str>; 260] = build_colorname(&THEME);

const fn build_colorname(theme: &Theme) -> [Option<&'static str>; 260] {
    let mut names: [Option<&'static str>; 260] = [None; 260];
    let mut i = 0;
    while i < 16 {
        names[i] = Some(theme.palette[i]);
        i += 1;
    }
    names[256] = Some(theme.cursor);
    names[257] = Some(theme.reverse_cursor);
    names[258] = Some(theme.foreground);
    names[259] = Some(theme.background);
    names
}

/// Default colours (colorname index).
pub const DEFAULTFG: u32 = 258;
pub const DEFAULTBG: u32 = 259;
pub const DEFAULTCS: u32 = 256;
pub const DEFAULTRCS: u32 = 257;

/// Default style of cursor, following xterm's DECSCUSR (`CSI Ps SP q`):
/// 0: blinking block
/// 1: blinking block (default)
/// 2: steady block ("█")
/// 3: blinking underline
/// 4: steady underline ("_")
/// 5: blinking bar
/// 6: steady bar ("|")
/// 7: blinking st cursor
/// 8: steady st cursor
pub const CURSORSTYLE: u32 = 1;

/// Rune drawn for cursor styles 7 and 8 (snowman).
pub const STCURSOR: Rune = 0x2603;

pub const COLS: u32 = 80;
pub const ROWS: u32 = 24;

/// Default colour and shape of the mouse cursor.
pub const MOUSESHAPE: u32 = 152; // XC_xterm
pub const MOUSEFG: usize = THEME.mouse_fg;
pub const MOUSEBG: usize = THEME.mouse_bg;

pub const DEFAULTATTR: u32 = 11;

/// Force mouse select/shortcuts while this mask is active.
pub const FORCEMOUSEMOD: u32 = ShiftMask;

pub struct Shortcut {
    pub mask: u32,
    pub keysym: u64,
    pub func: fn(&mut App, &Arg),
    pub arg: Arg,
}

pub struct MouseShortcut {
    pub mask: u32,
    pub button: u32,
    pub func: fn(&mut App, &Arg),
    pub arg: Arg,
    pub release: bool,
}

pub struct Key {
    pub k: u64,
    pub mask: u32,
    pub s: &'static [u8],
    /// 0: no value, >0: appkey enabled, 2: numlock, <0: disabled
    pub appkey: i8,
    /// 0: no value, >0: appcursor enabled, <0: disabled
    pub appcursor: i8,
}

/// Special keys. Keep XK_ANY_MOD entries last for each keysym.
pub static KEY: &[Key] = &[
    Key {
        k: x11::keysym::XK_KP_Home as u64,
        mask: ShiftMask,
        s: b"\x1b[2J",
        appkey: 0,
        appcursor: -1,
    },
    Key {
        k: x11::keysym::XK_KP_Home as u64,
        mask: ShiftMask,
        s: b"\x1b[1;2H",
        appkey: 0,
        appcursor: 1,
    },
    Key {
        k: x11::keysym::XK_KP_Home as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[H",
        appkey: 0,
        appcursor: -1,
    },
    Key {
        k: x11::keysym::XK_KP_Home as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[1~",
        appkey: 0,
        appcursor: 1,
    },
    Key {
        k: x11::keysym::XK_KP_Up as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOx",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Up as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[A",
        appkey: 0,
        appcursor: -1,
    },
    Key {
        k: x11::keysym::XK_KP_Up as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOA",
        appkey: 0,
        appcursor: 1,
    },
    Key {
        k: x11::keysym::XK_KP_Down as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOr",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Down as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[B",
        appkey: 0,
        appcursor: -1,
    },
    Key {
        k: x11::keysym::XK_KP_Down as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOB",
        appkey: 0,
        appcursor: 1,
    },
    Key {
        k: x11::keysym::XK_KP_Left as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOt",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Left as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[D",
        appkey: 0,
        appcursor: -1,
    },
    Key {
        k: x11::keysym::XK_KP_Left as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOD",
        appkey: 0,
        appcursor: 1,
    },
    Key {
        k: x11::keysym::XK_KP_Right as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOv",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Right as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[C",
        appkey: 0,
        appcursor: -1,
    },
    Key {
        k: x11::keysym::XK_KP_Right as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOC",
        appkey: 0,
        appcursor: 1,
    },
    Key {
        k: x11::keysym::XK_KP_Prior as u64,
        mask: ShiftMask,
        s: b"\x1b[5;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Prior as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[5~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Begin as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[E",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_End as u64,
        mask: ControlMask,
        s: b"\x1b[J",
        appkey: -1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_End as u64,
        mask: ControlMask,
        s: b"\x1b[1;5F",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_End as u64,
        mask: ShiftMask,
        s: b"\x1b[K",
        appkey: -1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_End as u64,
        mask: ShiftMask,
        s: b"\x1b[1;2F",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_End as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[4~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Next as u64,
        mask: ShiftMask,
        s: b"\x1b[6;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Next as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[6~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Insert as u64,
        mask: ShiftMask,
        s: b"\x1b[2;2~",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Insert as u64,
        mask: ShiftMask,
        s: b"\x1b[4l",
        appkey: -1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Insert as u64,
        mask: ControlMask,
        s: b"\x1b[L",
        appkey: -1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Insert as u64,
        mask: ControlMask,
        s: b"\x1b[2;5~",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Insert as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[4h",
        appkey: -1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Insert as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[2~",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Delete as u64,
        mask: ControlMask,
        s: b"\x1b[M",
        appkey: -1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Delete as u64,
        mask: ControlMask,
        s: b"\x1b[3;5~",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Delete as u64,
        mask: ShiftMask,
        s: b"\x1b[2K",
        appkey: -1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Delete as u64,
        mask: ShiftMask,
        s: b"\x1b[3;2~",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Delete as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[P",
        appkey: -1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Delete as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[3~",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Multiply as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOj",
        appkey: 2,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Add as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOk",
        appkey: 2,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Enter as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOM",
        appkey: 2,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Enter as u64,
        mask: XK_ANY_MOD,
        s: b"\r",
        appkey: -1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Subtract as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOm",
        appkey: 2,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Decimal as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOn",
        appkey: 2,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_Divide as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOo",
        appkey: 2,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_0 as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOp",
        appkey: 2,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_1 as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOq",
        appkey: 2,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_2 as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOr",
        appkey: 2,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_3 as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOs",
        appkey: 2,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_4 as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOt",
        appkey: 2,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_5 as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOu",
        appkey: 2,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_6 as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOv",
        appkey: 2,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_7 as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOw",
        appkey: 2,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_8 as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOx",
        appkey: 2,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_KP_9 as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOy",
        appkey: 2,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Up as u64,
        mask: ShiftMask,
        s: b"\x1b[1;2A",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Up as u64,
        mask: Mod1Mask,
        s: b"\x1b[1;3A",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Up as u64,
        mask: ShiftMask | Mod1Mask,
        s: b"\x1b[1;4A",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Up as u64,
        mask: ControlMask,
        s: b"\x1b[1;5A",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Up as u64,
        mask: ShiftMask | ControlMask,
        s: b"\x1b[1;6A",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Up as u64,
        mask: ControlMask | Mod1Mask,
        s: b"\x1b[1;7A",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Up as u64,
        mask: ShiftMask | ControlMask | Mod1Mask,
        s: b"\x1b[1;8A",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Up as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[A",
        appkey: 0,
        appcursor: -1,
    },
    Key {
        k: x11::keysym::XK_Up as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOA",
        appkey: 0,
        appcursor: 1,
    },
    Key {
        k: x11::keysym::XK_Down as u64,
        mask: ShiftMask,
        s: b"\x1b[1;2B",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Down as u64,
        mask: Mod1Mask,
        s: b"\x1b[1;3B",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Down as u64,
        mask: ShiftMask | Mod1Mask,
        s: b"\x1b[1;4B",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Down as u64,
        mask: ControlMask,
        s: b"\x1b[1;5B",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Down as u64,
        mask: ShiftMask | ControlMask,
        s: b"\x1b[1;6B",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Down as u64,
        mask: ControlMask | Mod1Mask,
        s: b"\x1b[1;7B",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Down as u64,
        mask: ShiftMask | ControlMask | Mod1Mask,
        s: b"\x1b[1;8B",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Down as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[B",
        appkey: 0,
        appcursor: -1,
    },
    Key {
        k: x11::keysym::XK_Down as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOB",
        appkey: 0,
        appcursor: 1,
    },
    Key {
        k: x11::keysym::XK_Left as u64,
        mask: ShiftMask,
        s: b"\x1b[1;2D",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Left as u64,
        mask: Mod1Mask,
        s: b"\x1b[1;3D",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Left as u64,
        mask: ShiftMask | Mod1Mask,
        s: b"\x1b[1;4D",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Left as u64,
        mask: ControlMask,
        s: b"\x1b[1;5D",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Left as u64,
        mask: ShiftMask | ControlMask,
        s: b"\x1b[1;6D",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Left as u64,
        mask: ControlMask | Mod1Mask,
        s: b"\x1b[1;7D",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Left as u64,
        mask: ShiftMask | ControlMask | Mod1Mask,
        s: b"\x1b[1;8D",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Left as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[D",
        appkey: 0,
        appcursor: -1,
    },
    Key {
        k: x11::keysym::XK_Left as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOD",
        appkey: 0,
        appcursor: 1,
    },
    Key {
        k: x11::keysym::XK_Right as u64,
        mask: ShiftMask,
        s: b"\x1b[1;2C",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Right as u64,
        mask: Mod1Mask,
        s: b"\x1b[1;3C",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Right as u64,
        mask: ShiftMask | Mod1Mask,
        s: b"\x1b[1;4C",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Right as u64,
        mask: ControlMask,
        s: b"\x1b[1;5C",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Right as u64,
        mask: ShiftMask | ControlMask,
        s: b"\x1b[1;6C",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Right as u64,
        mask: ControlMask | Mod1Mask,
        s: b"\x1b[1;7C",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Right as u64,
        mask: ShiftMask | ControlMask | Mod1Mask,
        s: b"\x1b[1;8C",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Right as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[C",
        appkey: 0,
        appcursor: -1,
    },
    Key {
        k: x11::keysym::XK_Right as u64,
        mask: XK_ANY_MOD,
        s: b"\x1bOC",
        appkey: 0,
        appcursor: 1,
    },
    Key {
        k: x11::keysym::XK_ISO_Left_Tab as u64,
        mask: ShiftMask,
        s: b"\x1b[Z",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Return as u64,
        mask: Mod1Mask,
        s: b"\x1b\r",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Return as u64,
        mask: XK_ANY_MOD,
        s: b"\r",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Insert as u64,
        mask: ShiftMask,
        s: b"\x1b[4l",
        appkey: -1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Insert as u64,
        mask: ShiftMask,
        s: b"\x1b[2;2~",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Insert as u64,
        mask: ControlMask,
        s: b"\x1b[L",
        appkey: -1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Insert as u64,
        mask: ControlMask,
        s: b"\x1b[2;5~",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Insert as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[4h",
        appkey: -1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Insert as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[2~",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Delete as u64,
        mask: ControlMask,
        s: b"\x1b[M",
        appkey: -1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Delete as u64,
        mask: ControlMask,
        s: b"\x1b[3;5~",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Delete as u64,
        mask: ShiftMask,
        s: b"\x1b[2K",
        appkey: -1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Delete as u64,
        mask: ShiftMask,
        s: b"\x1b[3;2~",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Delete as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[P",
        appkey: -1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Delete as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[3~",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_BackSpace as u64,
        mask: 0,
        s: b"\x7f",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_BackSpace as u64,
        mask: Mod1Mask,
        s: b"\x1b\x7f",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Home as u64,
        mask: ShiftMask,
        s: b"\x1b[2J",
        appkey: 0,
        appcursor: -1,
    },
    Key {
        k: x11::keysym::XK_Home as u64,
        mask: ShiftMask,
        s: b"\x1b[1;2H",
        appkey: 0,
        appcursor: 1,
    },
    Key {
        k: x11::keysym::XK_Home as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[H",
        appkey: 0,
        appcursor: -1,
    },
    Key {
        k: x11::keysym::XK_Home as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[1~",
        appkey: 0,
        appcursor: 1,
    },
    Key {
        k: x11::keysym::XK_End as u64,
        mask: ControlMask,
        s: b"\x1b[J",
        appkey: -1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_End as u64,
        mask: ControlMask,
        s: b"\x1b[1;5F",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_End as u64,
        mask: ShiftMask,
        s: b"\x1b[K",
        appkey: -1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_End as u64,
        mask: ShiftMask,
        s: b"\x1b[1;2F",
        appkey: 1,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_End as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[4~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Prior as u64,
        mask: ControlMask,
        s: b"\x1b[5;5~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Prior as u64,
        mask: ShiftMask,
        s: b"\x1b[5;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Prior as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[5~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Next as u64,
        mask: ControlMask,
        s: b"\x1b[6;5~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Next as u64,
        mask: ShiftMask,
        s: b"\x1b[6;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_Next as u64,
        mask: XK_ANY_MOD,
        s: b"\x1b[6~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F1 as u64,
        mask: 0,
        s: b"\x1bOP",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F1 as u64,
        mask: ShiftMask,
        s: b"\x1b[1;2P",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F1 as u64,
        mask: ControlMask,
        s: b"\x1b[1;5P",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F1 as u64,
        mask: Mod4Mask,
        s: b"\x1b[1;6P",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F1 as u64,
        mask: Mod1Mask,
        s: b"\x1b[1;3P",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F1 as u64,
        mask: Mod3Mask,
        s: b"\x1b[1;4P",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F2 as u64,
        mask: 0,
        s: b"\x1bOQ",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F2 as u64,
        mask: ShiftMask,
        s: b"\x1b[1;2Q",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F2 as u64,
        mask: ControlMask,
        s: b"\x1b[1;5Q",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F2 as u64,
        mask: Mod4Mask,
        s: b"\x1b[1;6Q",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F2 as u64,
        mask: Mod1Mask,
        s: b"\x1b[1;3Q",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F2 as u64,
        mask: Mod3Mask,
        s: b"\x1b[1;4Q",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F3 as u64,
        mask: 0,
        s: b"\x1bOR",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F3 as u64,
        mask: ShiftMask,
        s: b"\x1b[1;2R",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F3 as u64,
        mask: ControlMask,
        s: b"\x1b[1;5R",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F3 as u64,
        mask: Mod4Mask,
        s: b"\x1b[1;6R",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F3 as u64,
        mask: Mod1Mask,
        s: b"\x1b[1;3R",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F3 as u64,
        mask: Mod3Mask,
        s: b"\x1b[1;4R",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F4 as u64,
        mask: 0,
        s: b"\x1bOS",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F4 as u64,
        mask: ShiftMask,
        s: b"\x1b[1;2S",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F4 as u64,
        mask: ControlMask,
        s: b"\x1b[1;5S",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F4 as u64,
        mask: Mod4Mask,
        s: b"\x1b[1;6S",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F4 as u64,
        mask: Mod1Mask,
        s: b"\x1b[1;3S",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F5 as u64,
        mask: 0,
        s: b"\x1b[15~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F5 as u64,
        mask: ShiftMask,
        s: b"\x1b[15;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F5 as u64,
        mask: ControlMask,
        s: b"\x1b[15;5~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F5 as u64,
        mask: Mod4Mask,
        s: b"\x1b[15;6~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F5 as u64,
        mask: Mod1Mask,
        s: b"\x1b[15;3~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F6 as u64,
        mask: 0,
        s: b"\x1b[17~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F6 as u64,
        mask: ShiftMask,
        s: b"\x1b[17;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F6 as u64,
        mask: ControlMask,
        s: b"\x1b[17;5~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F6 as u64,
        mask: Mod4Mask,
        s: b"\x1b[17;6~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F6 as u64,
        mask: Mod1Mask,
        s: b"\x1b[17;3~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F7 as u64,
        mask: 0,
        s: b"\x1b[18~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F7 as u64,
        mask: ShiftMask,
        s: b"\x1b[18;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F7 as u64,
        mask: ControlMask,
        s: b"\x1b[18;5~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F7 as u64,
        mask: Mod4Mask,
        s: b"\x1b[18;6~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F7 as u64,
        mask: Mod1Mask,
        s: b"\x1b[18;3~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F8 as u64,
        mask: 0,
        s: b"\x1b[19~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F8 as u64,
        mask: ShiftMask,
        s: b"\x1b[19;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F8 as u64,
        mask: ControlMask,
        s: b"\x1b[19;5~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F8 as u64,
        mask: Mod4Mask,
        s: b"\x1b[19;6~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F8 as u64,
        mask: Mod1Mask,
        s: b"\x1b[19;3~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F9 as u64,
        mask: 0,
        s: b"\x1b[20~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F9 as u64,
        mask: ShiftMask,
        s: b"\x1b[20;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F9 as u64,
        mask: ControlMask,
        s: b"\x1b[20;5~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F9 as u64,
        mask: Mod4Mask,
        s: b"\x1b[20;6~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F9 as u64,
        mask: Mod1Mask,
        s: b"\x1b[20;3~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F10 as u64,
        mask: 0,
        s: b"\x1b[21~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F10 as u64,
        mask: ShiftMask,
        s: b"\x1b[21;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F10 as u64,
        mask: ControlMask,
        s: b"\x1b[21;5~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F10 as u64,
        mask: Mod4Mask,
        s: b"\x1b[21;6~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F10 as u64,
        mask: Mod1Mask,
        s: b"\x1b[21;3~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F11 as u64,
        mask: 0,
        s: b"\x1b[23~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F11 as u64,
        mask: ShiftMask,
        s: b"\x1b[23;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F11 as u64,
        mask: ControlMask,
        s: b"\x1b[23;5~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F11 as u64,
        mask: Mod4Mask,
        s: b"\x1b[23;6~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F11 as u64,
        mask: Mod1Mask,
        s: b"\x1b[23;3~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F12 as u64,
        mask: 0,
        s: b"\x1b[24~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F12 as u64,
        mask: ShiftMask,
        s: b"\x1b[24;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F12 as u64,
        mask: ControlMask,
        s: b"\x1b[24;5~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F12 as u64,
        mask: Mod4Mask,
        s: b"\x1b[24;6~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F12 as u64,
        mask: Mod1Mask,
        s: b"\x1b[24;3~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F13 as u64,
        mask: 0,
        s: b"\x1b[1;2P",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F14 as u64,
        mask: 0,
        s: b"\x1b[1;2Q",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F15 as u64,
        mask: 0,
        s: b"\x1b[1;2R",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F16 as u64,
        mask: 0,
        s: b"\x1b[1;2S",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F17 as u64,
        mask: 0,
        s: b"\x1b[15;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F18 as u64,
        mask: 0,
        s: b"\x1b[17;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F19 as u64,
        mask: 0,
        s: b"\x1b[18;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F20 as u64,
        mask: 0,
        s: b"\x1b[19;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F21 as u64,
        mask: 0,
        s: b"\x1b[20;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F22 as u64,
        mask: 0,
        s: b"\x1b[21;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F23 as u64,
        mask: 0,
        s: b"\x1b[23;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F24 as u64,
        mask: 0,
        s: b"\x1b[24;2~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F25 as u64,
        mask: 0,
        s: b"\x1b[1;5P",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F26 as u64,
        mask: 0,
        s: b"\x1b[1;5Q",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F27 as u64,
        mask: 0,
        s: b"\x1b[1;5R",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F28 as u64,
        mask: 0,
        s: b"\x1b[1;5S",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F29 as u64,
        mask: 0,
        s: b"\x1b[15;5~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F30 as u64,
        mask: 0,
        s: b"\x1b[17;5~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F31 as u64,
        mask: 0,
        s: b"\x1b[18;5~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F32 as u64,
        mask: 0,
        s: b"\x1b[19;5~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F33 as u64,
        mask: 0,
        s: b"\x1b[20;5~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F34 as u64,
        mask: 0,
        s: b"\x1b[21;5~",
        appkey: 0,
        appcursor: 0,
    },
    Key {
        k: x11::keysym::XK_F35 as u64,
        mask: 0,
        s: b"\x1b[23;5~",
        appkey: 0,
        appcursor: 0,
    },
];
/// Keys other than X11 function keys (0xFD00 - 0xFFFF) that may be mapped.
pub static MAPPEDKEYS: &[u64] = &[u64::MAX];

/// State bits ignored when matching key or button events.
pub const IGNOREMOD: u32 = Mod2Mask | XK_SWITCH_MOD;

/// Internal mouse shortcuts.
pub static MSHORTCUTS: &[MouseShortcut] = &[
    MouseShortcut {
        mask: XK_ANY_MOD,
        button: Button2,
        func: App::selpaste,
        arg: Arg::i(0),
        release: true,
    },
    MouseShortcut {
        mask: ShiftMask,
        button: Button4,
        func: App::ttysend,
        arg: Arg::s(b"\x1b[5;2~"),
        release: false,
    },
    MouseShortcut {
        mask: XK_ANY_MOD,
        button: Button4,
        func: App::ttysend,
        arg: Arg::s(b"\x19"),
        release: false,
    },
    MouseShortcut {
        mask: ShiftMask,
        button: Button5,
        func: App::ttysend,
        arg: Arg::s(b"\x1b[6;2~"),
        release: false,
    },
    MouseShortcut {
        mask: XK_ANY_MOD,
        button: Button5,
        func: App::ttysend,
        arg: Arg::s(b"\x05"),
        release: false,
    },
];

/// Internal keyboard shortcuts.
pub static SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        mask: XK_ANY_MOD,
        keysym: x11::keysym::XK_Break as u64,
        func: App::sendbreak,
        arg: Arg::i(0),
    },
    Shortcut {
        mask: ControlMask,
        keysym: x11::keysym::XK_Print as u64,
        func: App::toggleprinter,
        arg: Arg::i(0),
    },
    Shortcut {
        mask: ShiftMask,
        keysym: x11::keysym::XK_Print as u64,
        func: App::printscreen,
        arg: Arg::i(0),
    },
    Shortcut {
        mask: XK_ANY_MOD,
        keysym: x11::keysym::XK_Print as u64,
        func: App::printsel,
        arg: Arg::i(0),
    },
    Shortcut {
        mask: (ControlMask | ShiftMask),
        keysym: x11::keysym::XK_Prior as u64,
        func: App::zoom,
        arg: Arg::f(1f32),
    },
    Shortcut {
        mask: (ControlMask | ShiftMask),
        keysym: x11::keysym::XK_Next as u64,
        func: App::zoom,
        arg: Arg::f(-1f32),
    },
    Shortcut {
        mask: (ControlMask | ShiftMask),
        keysym: x11::keysym::XK_Home as u64,
        func: App::zoomreset,
        arg: Arg::f(0f32),
    },
    // Runtime font size: Ctrl+'+' / Ctrl+'-', including the Shift+equal
    // spelling of '+', Shift+minus ('_') and the keypad keys. Ctrl+0 resets.
    Shortcut {
        mask: ControlMask,
        keysym: x11::keysym::XK_plus as u64,
        func: App::zoom,
        arg: Arg::f(1f32),
    },
    Shortcut {
        mask: (ControlMask | ShiftMask),
        keysym: x11::keysym::XK_plus as u64,
        func: App::zoom,
        arg: Arg::f(1f32),
    },
    Shortcut {
        mask: ControlMask,
        keysym: x11::keysym::XK_equal as u64,
        func: App::zoom,
        arg: Arg::f(1f32),
    },
    Shortcut {
        mask: ControlMask,
        keysym: x11::keysym::XK_KP_Add as u64,
        func: App::zoom,
        arg: Arg::f(1f32),
    },
    Shortcut {
        mask: ControlMask,
        keysym: x11::keysym::XK_minus as u64,
        func: App::zoom,
        arg: Arg::f(-1f32),
    },
    Shortcut {
        mask: (ControlMask | ShiftMask),
        keysym: x11::keysym::XK_underscore as u64,
        func: App::zoom,
        arg: Arg::f(-1f32),
    },
    Shortcut {
        mask: ControlMask,
        keysym: x11::keysym::XK_KP_Subtract as u64,
        func: App::zoom,
        arg: Arg::f(-1f32),
    },
    Shortcut {
        mask: ControlMask,
        keysym: x11::keysym::XK_0 as u64,
        func: App::zoomreset,
        arg: Arg::f(0f32),
    },
    Shortcut {
        mask: (ControlMask | ShiftMask),
        keysym: x11::keysym::XK_C as u64,
        func: App::clipcopy,
        arg: Arg::i(0),
    },
    Shortcut {
        mask: (ControlMask | ShiftMask),
        keysym: x11::keysym::XK_V as u64,
        func: App::clippaste,
        arg: Arg::i(0),
    },
    Shortcut {
        mask: (ControlMask | ShiftMask),
        keysym: x11::keysym::XK_Y as u64,
        func: App::selpaste,
        arg: Arg::i(0),
    },
    Shortcut {
        mask: ShiftMask,
        keysym: x11::keysym::XK_Insert as u64,
        func: App::selpaste,
        arg: Arg::i(0),
    },
    Shortcut {
        mask: (ControlMask | ShiftMask),
        keysym: x11::keysym::XK_Num_Lock as u64,
        func: App::numlock,
        arg: Arg::i(0),
    },
];

/// Selection types' masks. Button1Mask is always unset.
pub static SELMASKS: [u32; 3] = [0, 0, Mod1Mask];

/// Printable characters in ASCII, used to estimate advance width of wide chars.
pub const ASCII_PRINTABLE: &[u8] =
    b" !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";
