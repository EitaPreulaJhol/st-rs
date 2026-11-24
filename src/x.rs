//! X11 frontend: window, events, fonts, drawing and selection.
//! This is a direct port of the original `x.c`.

use std::ffi::{CStr, CString, OsStr};
use std::os::raw::{c_char, c_int, c_long, c_uint, c_ulong, c_ushort, c_void};
use std::os::unix::ffi::OsStrExt;
use std::ptr;
use std::time::Instant;

use x11::xft::*;
use x11::xlib::*;
use x11::xrender::{XGlyphInfo, XRenderColor};

use crate::app::Font; // disambiguate from xlib::Font
use crate::app::*;
use crate::config;
use crate::config::{
    DEFAULTATTR, DEFAULTBG, DEFAULTFG, DOUBLECLICKTIMEOUT, MAXLATENCY, MINLATENCY,
    TRIPLECLICKTIMEOUT,
};
use crate::ffi::FcPattern;
use crate::ffi::*; // disambiguate from xft::FcPattern
                   // Prefer our variadic declarations over x11-rs' non-variadic stubs.
use crate::ffi::{
    XCreateIC, XNClientWindow, XNDestroyCallback, XNInputStyle, XNPreeditAttributes,
    XNSpotLocation, XRegisterIMInstantiateCallback, XSetICValues, XSetIMValues,
    XUnregisterIMInstantiateCallback, XVaCreateNestedList,
};
use crate::term::utf8encode;

// XEMBED messages
const XEMBED_FOCUS_IN: c_long = 4;
const XEMBED_FOCUS_OUT: c_long = 5;

#[derive(Default)]
pub struct Options {
    pub class: Option<CString>,
    pub cmd: Vec<CString>,
    pub embed: Option<CString>,
    pub font: Option<CString>,
    pub io: Option<CString>,
    pub line: Option<CString>,
    pub name: Option<CString>,
    pub title: Option<CString>,
}

fn xevent_type(ev: &XEvent) -> c_int {
    unsafe { ev.type_ }
}

fn os_to_cstring(s: &OsStr) -> CString {
    CString::new(s.as_bytes()).unwrap_or_else(|_| CString::new("").unwrap())
}

fn etype(ev: &XEvent) -> c_int {
    xevent_type(ev)
}

#[inline]
fn modbit_i(v: &mut i32, set: bool, bit: i32) {
    if set {
        *v |= bit;
    } else {
        *v &= !bit;
    }
}

#[inline]
fn modbit_u(v: &mut u32, set: bool, bit: u32) {
    if set {
        *v |= bit;
    } else {
        *v &= !bit;
    }
}

#[inline]
fn modbit_l(v: &mut c_long, set: bool, bit: c_long) {
    if set {
        *v |= bit;
    } else {
        *v &= !bit;
    }
}

fn errno_message(e: i32) -> String {
    unsafe {
        CStr::from_ptr(libc::strerror(e))
            .to_string_lossy()
            .into_owned()
    }
}

fn last_errno_message() -> String {
    errno_message(unsafe { *libc::__errno_location() })
}

fn cstring_opt(v: Option<&CString>) -> CString {
    v.cloned().unwrap_or_else(|| CString::new("").unwrap())
}

// ---------------------------------------------------------------------------
// Configuration callbacks
// ---------------------------------------------------------------------------

impl App {
    pub fn clipcopy(&mut self, _dummy: &Arg) {
        self.xsel.clipboard = None;

        if let Some(primary) = self.xsel.primary.clone() {
            self.xsel.clipboard = Some(primary);
            let clipboard = unsafe { XInternAtom(self.xw.dpy, c"CLIPBOARD".as_ptr(), False) };
            unsafe {
                XSetSelectionOwner(self.xw.dpy, clipboard, self.xw.win, CurrentTime);
            }
        }
    }

    pub fn clippaste(&mut self, _dummy: &Arg) {
        let clipboard = unsafe { XInternAtom(self.xw.dpy, c"CLIPBOARD".as_ptr(), False) };
        unsafe {
            XConvertSelection(
                self.xw.dpy,
                clipboard,
                self.xsel.xtarget,
                clipboard,
                self.xw.win,
                CurrentTime,
            );
        }
    }

    pub fn selpaste(&mut self, _dummy: &Arg) {
        unsafe {
            XConvertSelection(
                self.xw.dpy,
                XA_PRIMARY,
                self.xsel.xtarget,
                XA_PRIMARY,
                self.xw.win,
                CurrentTime,
            );
        }
    }

    pub fn numlock(&mut self, _dummy: &Arg) {
        self.win.mode ^= MODE_NUMLOCK;
    }

    pub fn zoom(&mut self, arg: &Arg) {
        if let ArgVal::F(f) = arg.0 {
            let target = (self.usedfontsize + config::FONT_SIZE_STEP as f64 * f as f64)
                .clamp(config::FONT_SIZE_MIN as f64, config::FONT_SIZE_MAX as f64);
            self.zoomabs(target);
        }
    }

    pub fn zoomabs(&mut self, fontsize: f64) {
        if fontsize <= 0.0 || (fontsize - self.usedfontsize).abs() < f64::EPSILON {
            return;
        }
        let font = self.usedfont.clone();
        self.xunloadfonts();
        self.xloadfonts(font.as_bytes(), fontsize);
        self.cresize(0, 0);
        self.redraw();
        self.xhints();
    }

    pub fn zoomreset(&mut self, _arg: &Arg) {
        if self.defaultfontsize > 0.0 {
            self.zoomabs(self.defaultfontsize);
        }
    }

    pub fn ttysend(&mut self, arg: &Arg) {
        if let ArgVal::S(s) = arg.0 {
            self.ttywrite(s, true);
        }
    }
}

// ---------------------------------------------------------------------------
// Mouse / selection events
// ---------------------------------------------------------------------------

impl App {
    fn evcol_xy(&self, x: c_int) -> i32 {
        let x = x - config::BORDERPX;
        limit(x, 0, self.win.tw - 1) / self.win.cw
    }

    fn evrow_xy(&self, y: c_int) -> i32 {
        let y = y - config::BORDERPX;
        limit(y, 0, self.win.th - 1) / self.win.ch
    }

    pub fn mousesel(&mut self, ev: &XEvent, done: bool) {
        let e = unsafe { ev.button };
        let mut seltype = SEL_REGULAR;
        let state = e.state & !(Button1Mask | config::FORCEMOUSEMOD);

        for type_ in 1..config::SELMASKS.len() {
            if self.match_mask(config::SELMASKS[type_], state) {
                seltype = type_ as i32;
                break;
            }
        }

        let col = self.evcol_xy(e.x);
        let row = self.evrow_xy(e.y);
        self.selextend(col, row, seltype, done);
        if done {
            let s = self.getsel();
            self.setsel(s, e.time);
        }
    }

    pub fn mousereport(&mut self, ev: &XEvent) {
        let e = unsafe { ev.button };
        let x = self.evcol_xy(e.x);
        let y = self.evrow_xy(e.y);
        let state = e.state;
        let etype = etype(ev);

        let (btn, mut code): (i32, i32);

        if etype == MotionNotify {
            if x == self.mouse_ox && y == self.mouse_oy {
                return;
            }
            if !self.win_is_set(MODE_MOUSEMOTION) && !self.win_is_set(MODE_MOUSEMANY) {
                return;
            }
            if self.win_is_set(MODE_MOUSEMOTION) && self.buttons == 0 {
                return;
            }
            let mut b = 1i32;
            while b <= 11 && self.buttons & (1 << (b - 1)) == 0 {
                b += 1;
            }
            btn = b; // 12 if none pressed
            code = 32;
        } else {
            let b = e.button as i32;
            if !(1..=11).contains(&b) {
                return;
            }
            if etype == ButtonRelease {
                if self.win_is_set(MODE_MOUSEX10) {
                    return;
                }
                if b == 4 || b == 5 {
                    return;
                }
            }
            btn = b;
            code = 0;
        }

        self.mouse_ox = x;
        self.mouse_oy = y;

        if (!self.win_is_set(MODE_MOUSESGR) && etype == ButtonRelease) || btn == 12 {
            code += 3;
        } else if btn >= 8 {
            code += 128 + btn - 8;
        } else if btn >= 4 {
            code += 64 + btn - 4;
        } else {
            code += btn - 1;
        }

        if !self.win_is_set(MODE_MOUSEX10) {
            code += if state & ShiftMask != 0 { 4 } else { 0 }
                + if state & Mod1Mask != 0 { 8 } else { 0 }
                + if state & ControlMask != 0 { 16 } else { 0 };
        }

        if self.win_is_set(MODE_MOUSESGR) {
            let buf = format!(
                "\x1b[<{};{};{}{}",
                code,
                x + 1,
                y + 1,
                if etype == ButtonRelease { 'm' } else { 'M' }
            );
            self.ttywrite(buf.as_bytes(), false);
        } else if x < 223 && y < 223 {
            let buf = [(32 + code) as u8, (32 + x + 1) as u8, (32 + y + 1) as u8];
            let mut out = Vec::with_capacity(6);
            out.extend_from_slice(b"\x1b[M");
            out.extend_from_slice(&buf);
            self.ttywrite(&out, false);
        }
    }

    pub fn bpress(&mut self, ev: &XEvent) {
        let e = unsafe { ev.button };
        let btn = e.button;

        if (1..=11).contains(&(btn as i32)) {
            self.buttons |= 1 << (btn - 1);
        }

        // On the alternate screen the wheel belongs to the application:
        // scrollback can only show the hidden main screen there.  Even the
        // force-mouse modifier must not take the wheel away from full-screen
        // programs (they use it to scroll their own transcript).
        let wheel = btn == Button4 || btn == Button5;
        let forced = e.state & config::FORCEMOUSEMOD != 0;
        if self.win_is_set(MODE_MOUSE)
            && (!forced || (wheel && self.is_set(MODE_ALTSCREEN)))
        {
            self.mousereport(ev);
            return;
        }

        if self.mouseaction(ev, false) {
            return;
        }

        if btn == Button1 {
            let now = Instant::now();
            let snap = if now.saturating_duration_since(self.xsel.tclick2).as_millis()
                <= TRIPLECLICKTIMEOUT as u128
            {
                SNAP_LINE
            } else if now.saturating_duration_since(self.xsel.tclick1).as_millis()
                <= DOUBLECLICKTIMEOUT as u128
            {
                SNAP_WORD
            } else {
                0
            };
            self.xsel.tclick2 = self.xsel.tclick1;
            self.xsel.tclick1 = now;

            let col = self.evcol_xy(e.x);
            let row = self.evrow_xy(e.y);
            self.selstart(col, row, snap);
        }
    }

    pub fn brelease(&mut self, ev: &XEvent) {
        let e = unsafe { ev.button };
        let btn = e.button;

        if (1..=11).contains(&(btn as i32)) {
            self.buttons &= !(1 << (btn - 1));
        }

        let wheel = btn == Button4 || btn == Button5;
        let forced = e.state & config::FORCEMOUSEMOD != 0;
        if self.win_is_set(MODE_MOUSE)
            && (!forced || (wheel && self.is_set(MODE_ALTSCREEN)))
        {
            self.mousereport(ev);
            return;
        }

        if self.mouseaction(ev, true) {
            return;
        }
        if btn == Button1 {
            self.mousesel(ev, true);
        }
    }

    pub fn bmotion(&mut self, ev: &XEvent) {
        let e = unsafe { ev.button };
        if self.win_is_set(MODE_MOUSE) && e.state & config::FORCEMOUSEMOD == 0 {
            self.mousereport(ev);
            return;
        }
        self.mousesel(ev, false);
    }

    fn buttonmask(button: u32) -> u32 {
        match button {
            Button1 => Button1Mask,
            Button2 => Button2Mask,
            Button3 => Button3Mask,
            Button4 => Button4Mask,
            Button5 => Button5Mask,
            _ => 0,
        }
    }

    fn mouseaction(&mut self, ev: &XEvent, release: bool) -> bool {
        let e = unsafe { ev.button };
        let state = e.state & !Self::buttonmask(e.button);

        // Ctrl+Click on a URL opens it (Ctrl-hover underlines URLs).
        if !release
            && e.button == Button1
            && (self.match_mask(ControlMask, state)
                || self.match_mask(ControlMask, state & !config::FORCEMOUSEMOD))
        {
            let row = self.evrow_xy(e.y);
            let col = self.evcol_xy(e.x);
            self.followurl(row, col);
            return true;
        }

        let altscreen = self.is_set(MODE_ALTSCREEN);
        for ms in config::MSHORTCUTS {
            if ms.release == release
                && ms.button == e.button
                && (ms.altscrn == 0
                    || (ms.altscrn > 0 && altscreen)
                    || (ms.altscrn < 0 && !altscreen))
                && (self.match_mask(ms.mask, state)
                    || self.match_mask(ms.mask, state & !config::FORCEMOUSEMOD))
            {
                (ms.func)(self, &ms.arg);
                return true;
            }
        }
        false
    }

    pub fn propnotify(&mut self, ev: &XEvent) {
        let xpev = unsafe { ev.property };
        let clipboard = unsafe { XInternAtom(self.xw.dpy, c"CLIPBOARD".as_ptr(), False) };
        if xpev.state == PropertyNewValue && (xpev.atom == XA_PRIMARY || xpev.atom == clipboard) {
            self.selnotify(ev);
        }
    }

    pub fn selnotify(&mut self, ev: &XEvent) {
        let incratom = unsafe { XInternAtom(self.xw.dpy, c"INCR".as_ptr(), False) };
        let mut ofs: c_ulong = 0;

        let property = if etype(ev) == SelectionNotify {
            unsafe { ev.selection.property }
        } else if etype(ev) == PropertyNotify {
            unsafe { ev.property.atom }
        } else {
            0
        };

        if property == 0 {
            return;
        }

        if property == self.xw.xdnd_selection {
            self.xdndsel(ev);
            return;
        }

        loop {
            let mut actual_type: Atom = 0;
            let mut actual_format: c_int = 0;
            let mut nitems: c_ulong = 0;
            let mut rem: c_ulong = 0;
            let mut data: *mut libc::c_uchar = ptr::null_mut();

            let rc = unsafe {
                XGetWindowProperty(
                    self.xw.dpy,
                    self.xw.win,
                    property,
                    ofs as c_long,
                    (libc::BUFSIZ / 4) as c_long,
                    False,
                    AnyPropertyType as Atom,
                    &mut actual_type,
                    &mut actual_format,
                    &mut nitems,
                    &mut rem,
                    &mut data,
                )
            };
            if rc != 0 {
                eprintln!("Clipboard allocation failed");
                return;
            }

            if etype(ev) == PropertyNotify && nitems == 0 && rem == 0 {
                // The selection owner signals that all data was transferred.
                modbit_l(
                    &mut self.xw.attrs.event_mask,
                    false,
                    PropertyChangeMask as c_long,
                );
                unsafe {
                    XChangeWindowAttributes(
                        self.xw.dpy,
                        self.xw.win,
                        CWEventMask,
                        &mut self.xw.attrs,
                    );
                }
            }

            if actual_type == incratom {
                // Activate PropertyNotify events for the next chunk.
                modbit_l(
                    &mut self.xw.attrs.event_mask,
                    true,
                    PropertyChangeMask as c_long,
                );
                unsafe {
                    XChangeWindowAttributes(
                        self.xw.dpy,
                        self.xw.win,
                        CWEventMask,
                        &mut self.xw.attrs,
                    );
                    // Deleting the property is the transfer start signal.
                    XDeleteProperty(self.xw.dpy, self.xw.win, property);
                }
                continue;
            }

            let len = (nitems * actual_format as c_ulong / 8) as usize;
            if !data.is_null() && len > 0 {
                // Replace '\n' with '\r' (consistent copy/paste line endings).
                let slice = unsafe { std::slice::from_raw_parts_mut(data, len) };
                for b in slice.iter_mut() {
                    if *b == b'\n' {
                        *b = b'\r';
                    }
                }

                if self.win_is_set(MODE_BRCKTPASTE) && ofs == 0 {
                    self.ttywrite(b"\x1b[200~", false);
                }
                self.ttywrite(slice, true);
                if self.win_is_set(MODE_BRCKTPASTE) && rem == 0 {
                    self.ttywrite(b"\x1b[201~", false);
                }
            }

            unsafe { XFree(data as *mut c_void) };
            ofs += nitems * actual_format as c_ulong / 32;
            if rem == 0 {
                break;
            }
        }

        // Tell the selection owner to send the next chunk.
        unsafe { XDeleteProperty(self.xw.dpy, self.xw.win, property) };
    }

    pub fn xclipcopy(&mut self) {
        self.clipcopy(&Arg::i(0));
    }

    pub fn selclear_(&mut self, _ev: &XEvent) {
        self.selclear();
    }

    pub fn selrequest(&mut self, ev: &XEvent) {
        let xsre = unsafe { ev.selection_request };
        let mut property = xsre.property;
        if property == 0 {
            property = xsre.target;
        }

        let mut xev: XEvent = unsafe { std::mem::zeroed() };
        let sel_ev = XSelectionEvent {
            type_: SelectionNotify,
            serial: 0,
            send_event: 0,
            display: xsre.display,
            requestor: xsre.requestor,
            selection: xsre.selection,
            target: xsre.target,
            property: 0, // reject
            time: xsre.time,
        };
        xev.selection = sel_ev;

        let xa_targets = unsafe { XInternAtom(self.xw.dpy, c"TARGETS".as_ptr(), False) };

        if xsre.target == xa_targets {
            // respond with the supported type
            let string = self.xsel.xtarget;
            unsafe {
                XChangeProperty(
                    xsre.display,
                    xsre.requestor,
                    property,
                    XA_ATOM,
                    32,
                    PropModeReplace,
                    &string as *const u64 as *const libc::c_uchar,
                    1,
                );
            }
            xev.selection.property = property;
        } else if xsre.target == self.xsel.xtarget || xsre.target == XA_STRING {
            let clipboard = unsafe { XInternAtom(self.xw.dpy, c"CLIPBOARD".as_ptr(), False) };
            let seltext: Option<&Vec<u8>> = if xsre.selection == XA_PRIMARY {
                self.xsel.primary.as_ref()
            } else if xsre.selection == clipboard {
                self.xsel.clipboard.as_ref()
            } else {
                eprintln!("Unhandled clipboard selection 0x{:x}", xsre.selection);
                return;
            };
            if let Some(text) = seltext {
                let len = text.iter().position(|&b| b == 0).unwrap_or(text.len());
                unsafe {
                    XChangeProperty(
                        xsre.display,
                        xsre.requestor,
                        property,
                        xsre.target,
                        8,
                        PropModeReplace,
                        text.as_ptr(),
                        len as c_int,
                    );
                }
                xev.selection.property = property;
            }
        }

        let rc = unsafe { XSendEvent(xsre.display, xsre.requestor, 1, 0, &mut xev) };
        if rc == 0 {
            eprintln!("Error sending SelectionNotify event");
        }
    }

    pub fn setsel(&mut self, s: Option<Vec<u8>>, t: Time) {
        if let Some(s) = s {
            self.xsel.primary = Some(s);
            unsafe {
                XSetSelectionOwner(self.xw.dpy, XA_PRIMARY, self.xw.win, t);
                if XGetSelectionOwner(self.xw.dpy, XA_PRIMARY) != self.xw.win {
                    self.selclear();
                }
            }
        }
    }

    pub fn xsetsel(&mut self, s: Vec<u8>) {
        self.setsel(Some(s), CurrentTime);
    }
}

// ---------------------------------------------------------------------------
// Window sizing
// ---------------------------------------------------------------------------

impl App {
    pub fn cresize(&mut self, width: i32, height: i32) {
        if width != 0 {
            self.win.w = width;
        }
        if height != 0 {
            self.win.h = height;
        }

        let mut col = (self.win.w - 2 * config::BORDERPX) / self.win.cw;
        let mut row = (self.win.h - 2 * config::BORDERPX) / self.win.ch;
        col = max(1, col);
        row = max(1, row);

        self.tresize(col, row);
        self.xresize(col, row);
        self.ttyresize(self.win.tw, self.win.th);
    }

    pub fn xresize(&mut self, col: i32, row: i32) {
        self.win.tw = col * self.win.cw;
        self.win.th = row * self.win.ch;

        unsafe {
            XFreePixmap(self.xw.dpy, self.xw.buf);
            self.xw.buf = XCreatePixmap(
                self.xw.dpy,
                self.xw.win,
                self.win.w as c_uint,
                self.win.h as c_uint,
                self.xw.depth as c_uint,
            );
            XftDrawChange(self.xw.draw, self.xw.buf);
        }
        self.xclear(0, 0, self.win.w, self.win.h);
        self.xw
            .specbuf
            .resize(col as usize, unsafe { std::mem::zeroed() });
    }
}

// ---------------------------------------------------------------------------
// Colours
// ---------------------------------------------------------------------------

fn sixd_to_16bit(x: c_int) -> c_ushort {
    if x == 0 {
        0
    } else {
        (0x3737 + 0x2828 * x) as c_ushort
    }
}

/// Opacity as the two places it has to live: the 16-bit Xft render colour
/// alpha and the alpha byte of an ARGB pixel value.
fn alpha_components(alpha: f32) -> (c_ushort, c_ulong) {
    let a16 = (0xffff as f32 * alpha) as c_ushort;
    let a8 = (0xff as f32 * alpha) as u8;
    (a16, (a8 as c_ulong) << 24)
}

/// Opacity of the cell background on screen row `y`, as a vertical ramp: the
/// top row gets `GRAD_ALPHA + STAT_ALPHA`, the bottom row about `STAT_ALPHA`.
fn gradient_alpha(y: i32, ch: i32, win_h: i32) -> c_ushort {
    if win_h <= 0 {
        return 0xffff;
    }
    (config::GRAD_ALPHA * 0xffff as f32 * (win_h - y * ch) as f32 / win_h as f32
        + config::STAT_ALPHA * 0xffff as f32) as c_ushort
}

impl App {
    fn xloadcolor(&mut self, i: i32, name: Option<&[u8]>, ncolor: &mut XftColor) -> bool {
        let mut color = XRenderColor {
            red: 0,
            green: 0,
            blue: 0,
            alpha: 0xffff,
        };

        let mut name = name;
        if name.is_none() {
            if between(i, 16, 255) {
                // 256 colour palette
                if i < 6 * 6 * 6 + 16 {
                    color.red = sixd_to_16bit(((i - 16) / 36) % 6);
                    color.green = sixd_to_16bit(((i - 16) / 6) % 6);
                    color.blue = sixd_to_16bit((i - 16) % 6);
                } else {
                    let v = (0x0808 + 0x0a0a * (i - (6 * 6 * 6 + 16))) as c_ushort;
                    color.red = v;
                    color.green = v;
                    color.blue = v;
                }
                return (unsafe {
                    XftColorAllocValue(self.xw.dpy, self.xw.vis, self.xw.cmap, &color, ncolor)
                }) != 0;
            }
            if !(0..config::COLORNAME.len() as i32).contains(&i) {
                return false;
            }
            name = config::COLORNAME[i as usize].map(|s| s.as_bytes());
            if name.is_none() {
                return false;
            }
        }

        let cname = CString::new(name.unwrap()).unwrap();
        (unsafe {
            XftColorAllocName(
                self.xw.dpy,
                self.xw.vis,
                self.xw.cmap,
                cname.as_ptr(),
                ncolor,
            )
        }) != 0
    }

    pub fn xloadcols(&mut self) {
        let loaded = !self.dc.col.is_empty();

        if loaded {
            for cp in self.dc.col.iter_mut() {
                unsafe {
                    XftColorFree(self.xw.dpy, self.xw.vis, self.xw.cmap, cp);
                }
            }
        } else {
            self.dc.collen = std::cmp::max(config::COLORNAME.len(), 256);
            self.dc.col = Vec::with_capacity(self.dc.collen);
            for _ in 0..self.dc.collen {
                self.dc.col.push(unsafe { std::mem::zeroed() });
            }
        }

        for i in 0..self.dc.collen {
            let mut c: XftColor = unsafe { std::mem::zeroed() };
            if !self.xloadcolor(i as i32, None, &mut c) {
                if config::COLORNAME
                    .get(i)
                    .map(|v| v.is_some())
                    .unwrap_or(false)
                {
                    die(&format!(
                        "could not allocate color '{}'\n",
                        config::COLORNAME[i].unwrap()
                    ));
                } else {
                    die(&format!("could not allocate color {}\n", i));
                }
            }
            self.dc.col[i] = c;
        }

        // The default background carries the configured opacity; everything
        // else stays fully opaque.
        let (a16, apix) = alpha_components(self.alpha);
        let bg = &mut self.dc.col[DEFAULTBG as usize];
        bg.color.alpha = a16;
        bg.pixel &= 0x00ff_ffff;
        bg.pixel |= apix;
    }

    pub fn xgetcolor(&self, x: i32) -> Option<(u8, u8, u8)> {
        if !between(x, 0, self.dc.collen as i32 - 1) {
            return None;
        }
        let c = &self.dc.col[x as usize];
        Some((
            (c.color.red >> 8) as u8,
            (c.color.green >> 8) as u8,
            (c.color.blue >> 8) as u8,
        ))
    }

    pub fn xsetcolorname(&mut self, x: i32, name: Option<&[u8]>) -> i32 {
        if !between(x, 0, self.dc.collen as i32 - 1) {
            return 1;
        }

        let mut ncolor: XftColor = unsafe { std::mem::zeroed() };
        if !self.xloadcolor(x, name, &mut ncolor) {
            return 1;
        }

        unsafe {
            XftColorFree(
                self.xw.dpy,
                self.xw.vis,
                self.xw.cmap,
                &mut self.dc.col[x as usize],
            );
        }
        self.dc.col[x as usize] = ncolor;

        if x == DEFAULTBG as i32 {
            let (a16, apix) = alpha_components(self.alpha);
            let bg = &mut self.dc.col[DEFAULTBG as usize];
            bg.color.alpha = a16;
            bg.pixel &= 0x00ff_ffff;
            bg.pixel |= apix;
        }
        0
    }

    /// Absolute coordinates.
    pub fn xclear(&mut self, x1: i32, y1: i32, x2: i32, y2: i32) {
        let idx = if self.win_is_set(MODE_REVERSE) {
            DEFAULTFG
        } else {
            DEFAULTBG
        } as usize;
        unsafe {
            XftDrawRect(
                self.xw.draw,
                &self.dc.col[idx],
                x1,
                y1,
                (x2 - x1) as c_uint,
                (y2 - y1) as c_uint,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Window properties
// ---------------------------------------------------------------------------

impl App {
    pub fn xhints(&mut self) {
        let name = self
            .opts
            .name
            .clone()
            .unwrap_or_else(|| CString::new(config::TERMNAME).unwrap());
        let class = self
            .opts
            .class
            .clone()
            .unwrap_or_else(|| CString::new(config::TERMNAME).unwrap());
        let mut classhint = XClassHint {
            res_name: name.as_ptr() as *mut c_char,
            res_class: class.as_ptr() as *mut c_char,
        };
        let mut wm = XWMHints {
            flags: InputHint as c_long,
            input: 1,
            initial_state: 0,
            icon_pixmap: 0,
            icon_window: 0,
            icon_x: 0,
            icon_y: 0,
            icon_mask: 0,
            window_group: 0,
        };

        let sizeh = unsafe { XAllocSizeHints() };
        unsafe {
            (*sizeh).flags = PSize | PResizeInc | PBaseSize | PMinSize;
            (*sizeh).height = self.win.h;
            (*sizeh).width = self.win.w;
            (*sizeh).height_inc = self.win.ch;
            (*sizeh).width_inc = self.win.cw;
            (*sizeh).base_height = 2 * config::BORDERPX;
            (*sizeh).base_width = 2 * config::BORDERPX;
            (*sizeh).min_height = self.win.ch + 2 * config::BORDERPX;
            (*sizeh).min_width = self.win.cw + 2 * config::BORDERPX;
            if self.xw.isfixed {
                (*sizeh).flags |= PMaxSize;
                (*sizeh).min_width = self.win.w;
                (*sizeh).max_width = self.win.w;
                (*sizeh).min_height = self.win.h;
                (*sizeh).max_height = self.win.h;
            }
            if self.xw.gm & (XValue | YValue) != 0 {
                (*sizeh).flags |= USPosition | PWinGravity;
                (*sizeh).x = self.xw.l;
                (*sizeh).y = self.xw.t;
                (*sizeh).win_gravity = xgeommasktogravity(self.xw.gm);
            }

            XSetWMProperties(
                self.xw.dpy,
                self.xw.win,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                sizeh,
                &mut wm,
                &mut classhint,
            );
            XFree(sizeh as *mut c_void);
        }
    }
}

fn xgeommasktogravity(mask: c_int) -> c_int {
    match mask & (XNegative | YNegative) {
        0 => NorthWestGravity,
        XNegative => NorthEastGravity,
        YNegative => SouthWestGravity,
        _ => SouthEastGravity,
    }
}

// ---------------------------------------------------------------------------
// Fonts
// ---------------------------------------------------------------------------

unsafe fn xloadfont(dpy: *mut Display, scr: c_int, f: &mut Font, pattern: *mut FcPattern) -> c_int {
    let configured = FcPatternDuplicate(pattern);
    if configured.is_null() {
        return 1;
    }
    FcConfigSubstitute(ptr::null_mut(), configured, FC_MATCH_PATTERN);
    XftDefaultSubstitute(dpy, scr, configured as *mut x11::xft::FcPattern);

    let mut result: c_int = 0;
    let match_ = FcFontMatch(ptr::null_mut(), configured, &mut result);
    if match_.is_null() {
        FcPatternDestroy(configured);
        return 1;
    }

    let xftfont = XftFontOpenPattern(dpy, match_ as *mut x11::xft::FcPattern);
    if xftfont.is_null() {
        FcPatternDestroy(configured);
        FcPatternDestroy(match_);
        return 1;
    }
    f.match_ = xftfont;

    // Check whether xft silently picked a different slant/weight.
    let mut wantattr: c_int = 0;
    if FcPatternGetInteger(
        pattern,
        FC_SLANT.as_ptr() as *const c_char,
        0,
        &mut wantattr,
    ) == FC_RESULT_MATCH
    {
        let mut haveattr: c_int = 0;
        if FcPatternGetInteger(
            (*xftfont).pattern as *mut FcPattern,
            FC_SLANT.as_ptr() as *const c_char,
            0,
            &mut haveattr,
        ) != FC_RESULT_MATCH
            || haveattr < wantattr
        {
            f.badslant = true;
            eprintln!("font slant does not match");
        }
    }

    if FcPatternGetInteger(
        pattern,
        FC_WEIGHT.as_ptr() as *const c_char,
        0,
        &mut wantattr,
    ) == FC_RESULT_MATCH
    {
        let mut haveattr: c_int = 0;
        if FcPatternGetInteger(
            (*xftfont).pattern as *mut FcPattern,
            FC_WEIGHT.as_ptr() as *const c_char,
            0,
            &mut haveattr,
        ) != FC_RESULT_MATCH
            || haveattr != wantattr
        {
            f.badweight = true;
            eprintln!("font weight does not match");
        }
    }

    let mut extents: XGlyphInfo = std::mem::zeroed();
    XftTextExtentsUtf8(
        dpy,
        xftfont,
        config::ASCII_PRINTABLE.as_ptr(),
        config::ASCII_PRINTABLE.len() as c_int,
        &mut extents,
    );

    f.set = ptr::null_mut();
    f.pattern = configured;

    f.ascent = (*xftfont).ascent;
    f.descent = (*xftfont).descent;
    f.lbearing = 0;
    f.rbearing = (*xftfont).max_advance_width as i16;

    f.height = f.ascent + f.descent;
    f.width = cdiv(extents.xOff as i32, config::ASCII_PRINTABLE.len() as i32);

    0
}

unsafe fn font_pattern(fontstr: &[u8]) -> *mut FcPattern {
    let c = CString::new(fontstr).unwrap_or_else(|_| CString::new("").unwrap());
    if fontstr.first() == Some(&b'-') {
        XftXlfdParse(c.as_ptr(), False, False) as *mut FcPattern
    } else {
        FcNameParse(c.as_ptr() as *const c_char)
    }
}

unsafe fn set_font_pixel_size(pattern: *mut FcPattern, size: f64) {
    FcPatternDel(pattern, FC_PIXEL_SIZE.as_ptr() as *const c_char);
    FcPatternDel(pattern, FC_SIZE.as_ptr() as *const c_char);
    FcPatternAddDouble(pattern, FC_PIXEL_SIZE.as_ptr() as *const c_char, size);
}

/// Load one of the optional per-style fonts, forcing it to the base font's
/// pixel size so that all four styles scale together.
unsafe fn load_custom_font(
    dpy: *mut Display,
    scr: c_int,
    f: &mut Font,
    fontstr: &[u8],
    pixel_size: f64,
) -> bool {
    let pattern = font_pattern(fontstr);
    if pattern.is_null() {
        return false;
    }
    if pixel_size > 0.0 {
        set_font_pixel_size(pattern, pixel_size);
    }
    let rc = xloadfont(dpy, scr, f, pattern);
    FcPatternDestroy(pattern);
    rc == 0
}

impl App {
    pub fn xloadfonts(&mut self, fontstr: &[u8], fontsize: f64) {
        let pattern: *mut FcPattern = unsafe { font_pattern(fontstr) };

        if pattern.is_null() {
            die(&format!(
                "can't open font {}\n",
                String::from_utf8_lossy(fontstr)
            ));
        }

        let mut fontval: f64 = 0.0;
        unsafe {
            if fontsize > 1.0 {
                set_font_pixel_size(pattern, fontsize);
                self.usedfontsize = fontsize;
            } else {
                if FcPatternGetDouble(
                    pattern,
                    FC_PIXEL_SIZE.as_ptr() as *const c_char,
                    0,
                    &mut fontval,
                ) == FC_RESULT_MATCH
                {
                    self.usedfontsize = fontval;
                } else if FcPatternGetDouble(
                    pattern,
                    FC_SIZE.as_ptr() as *const c_char,
                    0,
                    &mut fontval,
                ) == FC_RESULT_MATCH
                {
                    self.usedfontsize = -1.0;
                } else {
                    // Default font size is 12 if none given.
                    FcPatternAddDouble(pattern, FC_PIXEL_SIZE.as_ptr() as *const c_char, 12.0);
                    self.usedfontsize = 12.0;
                }
                self.defaultfontsize = self.usedfontsize;
            }

            if xloadfont(self.xw.dpy, self.xw.scr, &mut self.dc.font, pattern) != 0 {
                die(&format!(
                    "can't open font {}\n",
                    String::from_utf8_lossy(fontstr)
                ));
            }

            if self.usedfontsize < 0.0 {
                FcPatternGetDouble(
                    (*self.dc.font.match_).pattern as *mut FcPattern,
                    FC_PIXEL_SIZE.as_ptr() as *const c_char,
                    0,
                    &mut fontval,
                );
                self.usedfontsize = fontval;
                if fontsize == 0.0 {
                    self.defaultfontsize = fontval;
                }
            }

            // character width and height
            self.win.cw = (self.dc.font.width as f32 * config::CWSCALE).ceil() as i32;
            self.win.ch = (self.dc.font.height as f32 * config::CHSCALE).ceil() as i32;

            let dpy = self.xw.dpy;
            let scr = self.xw.scr;
            let pixel_size = self.usedfontsize;

            // Keep the base pattern's style flags up to date even when a
            // custom font is supplied, so that an empty slot still derives
            // correctly from FONT.
            FcPatternDel(pattern, FC_SLANT.as_ptr() as *const c_char);
            FcPatternAddInteger(pattern, FC_SLANT.as_ptr() as *const c_char, FC_SLANT_ITALIC);

            // Italic: custom font when configured, otherwise derived from the
            // base pattern exactly like the original.
            if config::FONT_ITALIC.is_empty() {
                if xloadfont(dpy, scr, &mut self.dc.ifont, pattern) != 0 {
                    die(&format!(
                        "can't open font {}\n",
                        String::from_utf8_lossy(fontstr)
                    ));
                }
            } else if !load_custom_font(
                dpy,
                scr,
                &mut self.dc.ifont,
                config::FONT_ITALIC.as_bytes(),
                pixel_size,
            ) {
                die(&format!("can't open font {}\n", config::FONT_ITALIC));
            }

            // Bold italic.
            FcPatternDel(pattern, FC_WEIGHT.as_ptr() as *const c_char);
            FcPatternAddInteger(pattern, FC_WEIGHT.as_ptr() as *const c_char, FC_WEIGHT_BOLD);
            if config::FONT_BOLD_ITALIC.is_empty() {
                if xloadfont(dpy, scr, &mut self.dc.ibfont, pattern) != 0 {
                    die(&format!(
                        "can't open font {}\n",
                        String::from_utf8_lossy(fontstr)
                    ));
                }
            } else if !load_custom_font(
                dpy,
                scr,
                &mut self.dc.ibfont,
                config::FONT_BOLD_ITALIC.as_bytes(),
                pixel_size,
            ) {
                die(&format!("can't open font {}\n", config::FONT_BOLD_ITALIC));
            }

            // Bold.
            FcPatternDel(pattern, FC_SLANT.as_ptr() as *const c_char);
            FcPatternAddInteger(pattern, FC_SLANT.as_ptr() as *const c_char, FC_SLANT_ROMAN);
            if config::FONT_BOLD.is_empty() {
                if xloadfont(dpy, scr, &mut self.dc.bfont, pattern) != 0 {
                    die(&format!(
                        "can't open font {}\n",
                        String::from_utf8_lossy(fontstr)
                    ));
                }
            } else if !load_custom_font(
                dpy,
                scr,
                &mut self.dc.bfont,
                config::FONT_BOLD.as_bytes(),
                pixel_size,
            ) {
                die(&format!("can't open font {}\n", config::FONT_BOLD));
            }

            FcPatternDestroy(pattern);
        }
    }

    pub fn xunloadfonts(&mut self) {
        unsafe {
            while let Some(rc) = self.frc.pop() {
                XftFontClose(self.xw.dpy, rc.font);
            }
            xunloadfont(self.xw.dpy, &mut self.dc.font);
            xunloadfont(self.xw.dpy, &mut self.dc.bfont);
            xunloadfont(self.xw.dpy, &mut self.dc.ifont);
            xunloadfont(self.xw.dpy, &mut self.dc.ibfont);
        }
    }
}

unsafe fn xunloadfont(dpy: *mut Display, f: &mut Font) {
    if !f.match_.is_null() {
        XftFontClose(dpy, f.match_);
    }
    if !f.pattern.is_null() {
        FcPatternDestroy(f.pattern);
    }
    if !f.set.is_null() {
        FcFontSetDestroy(f.set);
    }
}

// ---------------------------------------------------------------------------
// Input methods
// ---------------------------------------------------------------------------

impl App {
    pub fn ximopen(&mut self, dpy: *mut Display) -> bool {
        let mut imdestroy = XIMCallback {
            client_data: ptr::null_mut(),
            callback: Some(ximdestroy_cb),
        };
        let mut icdestroy = XICCallback {
            client_data: ptr::null_mut(),
            callback: Some(xicdestroy_cb),
        };

        unsafe {
            self.xw.ime.xim = XOpenIM(dpy, ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
            if self.xw.ime.xim.is_null() {
                return false;
            }

            let rc = XSetIMValues(
                self.xw.ime.xim,
                XNDestroyCallback.as_ptr(),
                &mut imdestroy as *mut XIMCallback,
                ptr::null::<c_char>(),
            );
            if !rc.is_null() {
                eprintln!("XSetIMValues: Could not set XNDestroyCallback.");
            }

            self.xw.ime.spotlist = XVaCreateNestedList(
                0,
                XNSpotLocation.as_ptr(),
                &mut self.xw.ime.spot as *mut XPoint,
                ptr::null::<c_char>(),
            );

            if self.xw.ime.xic.is_null() {
                self.xw.ime.xic = XCreateIC(
                    self.xw.ime.xim,
                    XNInputStyle.as_ptr(),
                    (XIMPreeditNothing | XIMStatusNothing) as c_ulong,
                    XNClientWindow.as_ptr(),
                    self.xw.win,
                    XNDestroyCallback.as_ptr(),
                    &mut icdestroy as *mut XICCallback,
                    ptr::null::<c_char>(),
                );
            }
            if self.xw.ime.xic.is_null() {
                eprintln!("XCreateIC: Could not create input context.");
            }
        }
        true
    }
}

unsafe extern "C" fn ximinstantiate_cb(dpy: *mut Display, _client: XPointer, _call: XPointer) {
    let app = app();
    if app.ximopen(dpy) {
        XUnregisterIMInstantiateCallback(
            app.xw.dpy,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            Some(ximinstantiate_cb),
            ptr::null_mut(),
        );
    }
}

unsafe extern "C" fn ximdestroy_cb(_xim: XIM, _client: XPointer, _call: XPointer) {
    let app = app();
    app.xw.ime.xim = ptr::null_mut();
    XRegisterIMInstantiateCallback(
        app.xw.dpy,
        ptr::null_mut(),
        ptr::null_mut(),
        ptr::null_mut(),
        Some(ximinstantiate_cb),
        ptr::null_mut(),
    );
    XFree(app.xw.ime.spotlist);
}

unsafe extern "C" fn xicdestroy_cb(_xim: XIC, _client: XPointer, _call: XPointer) -> c_int {
    let app = app();
    app.xw.ime.xic = ptr::null_mut();
    1
}

// ---------------------------------------------------------------------------
// Initialisation
// ---------------------------------------------------------------------------

impl App {
    pub fn xinit(&mut self, cols: i32, rows: i32) {
        unsafe {
            self.xw.dpy = XOpenDisplay(ptr::null());
            if self.xw.dpy.is_null() {
                die("can't open display\n");
            }
            self.xw.scr = XDefaultScreen(self.xw.dpy);

            // A top-level window uses a 32-bit ARGB visual so the compositor
            // can blend the background using the configured opacity.  An
            // embedded window has to match its parent's depth.
            let root = XRootWindow(self.xw.dpy, self.xw.scr);
            let parent = self
                .opts
                .embed
                .as_ref()
                .and_then(|e| {
                    let v = libc::strtol(e.as_ptr(), ptr::null_mut(), 0);
                    if v != 0 {
                        Some(v as c_ulong)
                    } else {
                        None
                    }
                })
                .unwrap_or(root);
            let mut depth = 32;
            let mut parent_visual: *mut Visual = ptr::null_mut();
            if parent != root {
                let mut attr: XWindowAttributes = std::mem::zeroed();
                if XGetWindowAttributes(self.xw.dpy, parent, &mut attr) != 0 {
                    depth = attr.depth;
                    parent_visual = attr.visual;
                } else {
                    depth = XDefaultDepth(self.xw.dpy, self.xw.scr);
                }
            }

            let mut vis_info: XVisualInfo = std::mem::zeroed();
            if XMatchVisualInfo(self.xw.dpy, self.xw.scr, depth, TrueColor, &mut vis_info) != 0 {
                self.xw.depth = vis_info.depth;
                self.xw.vis = vis_info.visual;
            } else if !parent_visual.is_null() {
                self.xw.depth = depth;
                self.xw.vis = parent_visual;
            } else {
                // The server has no ARGB TrueColor visual: stay opaque.
                self.xw.depth = XDefaultDepth(self.xw.dpy, self.xw.scr);
                self.xw.vis = XDefaultVisual(self.xw.dpy, self.xw.scr);
            }

            // font
            if FcInit() == 0 {
                die("could not init fontconfig.\n");
            }

            self.usedfont = if let Some(f) = &self.opts.font {
                f.clone()
            } else {
                CString::new(config::FONT).unwrap()
            };
            let usedfont = self.usedfont.clone();
            self.xloadfonts(usedfont.as_bytes(), 0.0);

            // colours
            self.xw.cmap = XCreateColormap(self.xw.dpy, parent, self.xw.vis, AllocNone);
            self.xloadcols();

            // adjust fixed window geometry
            self.win.w = 2 * config::BORDERPX + cols * self.win.cw;
            self.win.h = 2 * config::BORDERPX + rows * self.win.ch;
            if self.xw.gm & XNegative != 0 {
                self.xw.l += XDisplayWidth(self.xw.dpy, self.xw.scr) - self.win.w - 2;
            }
            if self.xw.gm & YNegative != 0 {
                self.xw.t += XDisplayHeight(self.xw.dpy, self.xw.scr) - self.win.h - 2;
            }

            // events
            self.xw.attrs = std::mem::zeroed();
            self.xw.attrs.background_pixel = self.dc.col[DEFAULTBG as usize].pixel;
            self.xw.attrs.border_pixel = self.dc.col[DEFAULTBG as usize].pixel;
            self.xw.attrs.bit_gravity = NorthWestGravity;
            self.xw.attrs.event_mask = (FocusChangeMask
                | KeyPressMask
                | KeyReleaseMask
                | ExposureMask
                | VisibilityChangeMask
                | StructureNotifyMask
                | ButtonMotionMask
                | ButtonPressMask
                | ButtonReleaseMask) as c_long;
            self.xw.attrs.colormap = self.xw.cmap;

            self.xw.win = XCreateWindow(
                self.xw.dpy,
                root,
                self.xw.l,
                self.xw.t,
                self.win.w as c_uint,
                self.win.h as c_uint,
                0,
                self.xw.depth,
                InputOutput as c_uint,
                self.xw.vis,
                CWBackPixel | CWBorderPixel | CWBitGravity | CWEventMask | CWColormap,
                &mut self.xw.attrs,
            );
            if parent != root {
                XReparentWindow(self.xw.dpy, self.xw.win, parent, self.xw.l, self.xw.t);
            }

            self.xw.buf = XCreatePixmap(
                self.xw.dpy,
                self.xw.win,
                self.win.w as c_uint,
                self.win.h as c_uint,
                self.xw.depth as c_uint,
            );

            let mut gcvalues: XGCValues = std::mem::zeroed();
            gcvalues.graphics_exposures = False;
            self.dc.gc = XCreateGC(
                self.xw.dpy,
                self.xw.buf,
                GCGraphicsExposures as c_ulong,
                &mut gcvalues,
            );
            XSetForeground(
                self.xw.dpy,
                self.dc.gc,
                self.dc.col[DEFAULTBG as usize].pixel,
            );
            XFillRectangle(
                self.xw.dpy,
                self.xw.buf,
                self.dc.gc,
                0,
                0,
                self.win.w as c_uint,
                self.win.h as c_uint,
            );

            // font spec buffer
            self.xw.specbuf.resize(cols as usize, std::mem::zeroed());

            // Xft rendering context
            self.xw.draw = XftDrawCreate(self.xw.dpy, self.xw.buf, self.xw.vis, self.xw.cmap);

            // input methods
            if !self.ximopen(self.xw.dpy) {
                XRegisterIMInstantiateCallback(
                    self.xw.dpy,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    Some(ximinstantiate_cb),
                    ptr::null_mut(),
                );
            }

            // dark cursor with a light outline
            let cursor = XCreateFontCursor(self.xw.dpy, config::MOUSESHAPE);
            XDefineCursor(self.xw.dpy, self.xw.win, cursor);

            let mut xmousefg: XColor = std::mem::zeroed();
            let mut xmousebg: XColor = std::mem::zeroed();
            let fgname = CString::new(config::COLORNAME[config::MOUSEFG].unwrap()).unwrap();
            let bgname = CString::new(config::COLORNAME[config::MOUSEBG].unwrap()).unwrap();
            if XParseColor(self.xw.dpy, self.xw.cmap, fgname.as_ptr(), &mut xmousefg) == 0 {
                xmousefg.red = 0xffff;
                xmousefg.green = 0xffff;
                xmousefg.blue = 0xffff;
            }
            if XParseColor(self.xw.dpy, self.xw.cmap, bgname.as_ptr(), &mut xmousebg) == 0 {
                xmousebg.red = 0x0000;
                xmousebg.green = 0x0000;
                xmousebg.blue = 0x0000;
            }
            XRecolorCursor(self.xw.dpy, cursor, &mut xmousefg, &mut xmousebg);

            self.xw.xembed = XInternAtom(self.xw.dpy, c"_XEMBED".as_ptr(), False);
            self.xw.wmdeletewin = XInternAtom(self.xw.dpy, c"WM_DELETE_WINDOW".as_ptr(), False);
            self.xw.netwmname = XInternAtom(self.xw.dpy, c"_NET_WM_NAME".as_ptr(), False);
            self.xw.netwmiconname = XInternAtom(self.xw.dpy, c"_NET_WM_ICON_NAME".as_ptr(), False);
            let netwmicon = XInternAtom(self.xw.dpy, c"_NET_WM_ICON".as_ptr(), False);
            // Format 32 properties are arrays of C `long`, not `int`.
            let icon: Vec<c_ulong> = crate::icon::net_wm_icon()
                .into_iter()
                .map(|pixel| pixel as c_ulong)
                .collect();
            XChangeProperty(
                self.xw.dpy,
                self.xw.win,
                netwmicon,
                XA_CARDINAL,
                32,
                PropModeReplace,
                icon.as_ptr() as *const libc::c_uchar,
                icon.len() as c_int,
            );
            XSetWMProtocols(self.xw.dpy, self.xw.win, &mut self.xw.wmdeletewin, 1);

            self.xw.netwmpid = XInternAtom(self.xw.dpy, c"_NET_WM_PID".as_ptr(), False);
            let thispid: c_ulong = libc::getpid() as c_ulong;
            XChangeProperty(
                self.xw.dpy,
                self.xw.win,
                self.xw.netwmpid,
                XA_CARDINAL,
                32,
                PropModeReplace,
                &thispid as *const c_ulong as *const libc::c_uchar,
                1,
            );

            // Drag and drop (XDND).
            self.xw.xdnd_type_list = XInternAtom(self.xw.dpy, c"XdndTypeList".as_ptr(), False);
            self.xw.xdnd_selection = XInternAtom(self.xw.dpy, c"XdndSelection".as_ptr(), False);
            self.xw.xdnd_enter = XInternAtom(self.xw.dpy, c"XdndEnter".as_ptr(), False);
            self.xw.xdnd_position = XInternAtom(self.xw.dpy, c"XdndPosition".as_ptr(), False);
            self.xw.xdnd_status = XInternAtom(self.xw.dpy, c"XdndStatus".as_ptr(), False);
            self.xw.xdnd_drop = XInternAtom(self.xw.dpy, c"XdndDrop".as_ptr(), False);
            self.xw.xdnd_finished = XInternAtom(self.xw.dpy, c"XdndFinished".as_ptr(), False);
            self.xw.xdnd_action_copy = XInternAtom(self.xw.dpy, c"XdndActionCopy".as_ptr(), False);
            self.xw.xtext_uri_list = XInternAtom(self.xw.dpy, c"text/uri-list".as_ptr(), False);
            self.xw.xtext_plain = XInternAtom(self.xw.dpy, c"text/plain".as_ptr(), False);
            self.xw.xdnd_aware = XInternAtom(self.xw.dpy, c"XdndAware".as_ptr(), False);
            let xdnd_version: c_ulong = 5;
            XChangeProperty(
                self.xw.dpy,
                self.xw.win,
                self.xw.xdnd_aware,
                XA_ATOM,
                32,
                PropModeReplace,
                &xdnd_version as *const c_ulong as *const libc::c_uchar,
                1,
            );

            self.win.mode = MODE_NUMLOCK;
            self.resettitle();
            self.xhints();
            XMapWindow(self.xw.dpy, self.xw.win);
            XSync(self.xw.dpy, False);

            self.xsel.tclick1 = Instant::now();
            self.xsel.tclick2 = Instant::now();
            self.xsel.primary = None;
            self.xsel.clipboard = None;
            self.xsel.xtarget = XInternAtom(self.xw.dpy, c"UTF8_STRING".as_ptr(), False);
            if self.xsel.xtarget == 0 {
                self.xsel.xtarget = XA_STRING;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Glyph rendering
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum FontKind {
    Normal,
    Italic,
    Bold,
    ItalicBold,
}

impl App {
    fn font_ptr(&self, kind: FontKind) -> *mut XftFont {
        match kind {
            FontKind::Normal => self.dc.font.match_,
            FontKind::Italic => self.dc.ifont.match_,
            FontKind::Bold => self.dc.bfont.match_,
            FontKind::ItalicBold => self.dc.ibfont.match_,
        }
    }

    fn font_ascent(&self, kind: FontKind) -> i32 {
        match kind {
            FontKind::Normal => self.dc.font.ascent,
            FontKind::Italic => self.dc.ifont.ascent,
            FontKind::Bold => self.dc.bfont.ascent,
            FontKind::ItalicBold => self.dc.ibfont.ascent,
        }
    }

    fn font_mut(&mut self, kind: FontKind) -> &mut Font {
        match kind {
            FontKind::Normal => &mut self.dc.font,
            FontKind::Italic => &mut self.dc.ifont,
            FontKind::Bold => &mut self.dc.bfont,
            FontKind::ItalicBold => &mut self.dc.ibfont,
        }
    }

    fn xmakeglyphfontspecs(
        &mut self,
        specs: &mut [XftGlyphFontSpec],
        glyphs: &[Glyph],
        x: i32,
        y: i32,
    ) -> usize {
        let winx = config::BORDERPX as f32 + (x * self.win.cw) as f32;
        let winy = config::BORDERPX as f32 + (y * self.win.ch) as f32;
        let mut xp = winx;
        let mut yp = winy + self.dc.font.ascent as f32;
        let mut prevmode: u16 = u16::MAX;
        let mut fontkind = FontKind::Normal;
        let mut frcflags = FRC_NORMAL;
        let mut runewidth = self.win.cw as f32;
        let mut numspecs = 0usize;

        for glyph in glyphs.iter() {
            let rune = glyph.u;
            let mode = glyph.mode;

            // Skip dummy wide-character spacing.
            if mode == ATTR_WDUMMY {
                continue;
            }

            // Determine font for glyph if different from previous glyph.
            if prevmode != mode {
                prevmode = mode;
                fontkind = FontKind::Normal;
                frcflags = FRC_NORMAL;
                runewidth = self.win.cw as f32 * if mode & ATTR_WIDE != 0 { 2.0 } else { 1.0 };
                if mode & ATTR_ITALIC != 0 && mode & ATTR_BOLD != 0 {
                    fontkind = FontKind::ItalicBold;
                    frcflags = FRC_ITALICBOLD;
                } else if mode & ATTR_ITALIC != 0 {
                    fontkind = FontKind::Italic;
                    frcflags = FRC_ITALIC;
                } else if mode & ATTR_BOLD != 0 {
                    fontkind = FontKind::Bold;
                    frcflags = FRC_BOLD;
                }
                yp = winy + self.font_ascent(fontkind) as f32;
            }

            // Lookup character index with default font.
            let mut glyphidx = unsafe { XftCharIndex(self.xw.dpy, self.font_ptr(fontkind), rune) };

            let mut found: Option<usize> = None;
            if glyphidx != 0 {
                found = Some(usize::MAX); // marker: use selected font
            } else {
                // Fallback on font cache.
                for f in 0..self.frc.len() {
                    let g = unsafe { XftCharIndex(self.xw.dpy, self.frc[f].font, rune) };
                    if g != 0 && self.frc[f].flags == frcflags {
                        glyphidx = g;
                        found = Some(f);
                        break;
                    }
                    if g == 0 && self.frc[f].flags == frcflags && self.frc[f].unicodep == rune {
                        glyphidx = g;
                        found = Some(f);
                        break;
                    }
                }
            }

            if found.is_none() {
                // Nothing found: use fontconfig to find a matching font.
                let font = self.font_mut(fontkind);
                unsafe {
                    if font.set.is_null() {
                        let mut fcres: c_int = 0;
                        font.set = FcFontSort(
                            ptr::null_mut(),
                            font.pattern,
                            True,
                            ptr::null_mut(),
                            &mut fcres,
                        );
                    }
                    let mut fcsets: [*mut FcFontSet; 1] = [font.set];

                    let fcpattern = FcPatternDuplicate(font.pattern);
                    let fccharset = FcCharSetCreate();
                    FcCharSetAddChar(fccharset, rune);
                    FcPatternAddCharSet(fcpattern, FC_CHARSET.as_ptr() as *const c_char, fccharset);
                    FcPatternAddBool(fcpattern, FC_SCALABLE.as_ptr() as *const c_char, FC_TRUE);
                    FcConfigSubstitute(ptr::null_mut(), fcpattern, FC_MATCH_PATTERN);
                    FcDefaultSubstitute(fcpattern);

                    let mut fcres: c_int = 0;
                    let fontpattern = FcFontSetMatch(
                        ptr::null_mut(),
                        fcsets.as_mut_ptr(),
                        1,
                        fcpattern,
                        &mut fcres,
                    );

                    let xftfont =
                        XftFontOpenPattern(self.xw.dpy, fontpattern as *mut x11::xft::FcPattern);
                    if xftfont.is_null() {
                        die(&format!(
                            "XftFontOpenPattern failed seeking fallback font: {}\n",
                            last_errno_message()
                        ));
                    }
                    self.frc.push(Fontcache {
                        font: xftfont,
                        flags: frcflags,
                        unicodep: rune,
                    });

                    glyphidx = XftCharIndex(self.xw.dpy, xftfont, rune);
                    let f = self.frc.len() - 1;
                    found = Some(f);

                    FcPatternDestroy(fcpattern);
                    FcCharSetDestroy(fccharset);
                }
            }

            let font = match found.unwrap() {
                usize::MAX => self.font_ptr(fontkind),
                f => self.frc[f].font,
            };

            specs[numspecs] = XftGlyphFontSpec {
                font,
                glyph: glyphidx,
                x: xp as i16,
                y: yp as i16,
            };
            xp += runewidth;
            numspecs += 1;
        }

        numspecs
    }

    fn xdrawglyphfontspecs(
        &mut self,
        specs: &[XftGlyphFontSpec],
        base: Glyph,
        len: usize,
        x: i32,
        y: i32,
    ) {
        let charlen = len * if base.mode & ATTR_WIDE != 0 { 2 } else { 1 };
        let winx = config::BORDERPX + x * self.win.cw;
        let winy = config::BORDERPX + y * self.win.ch;
        let width = (charlen as i32) * self.win.cw;
        let mut base = base;

        // Fallback on colour display for attributes not supported by the font.
        if base.mode & ATTR_ITALIC != 0 && base.mode & ATTR_BOLD != 0 {
            if self.dc.ibfont.badslant || self.dc.ibfont.badweight {
                base.fg = DEFAULTATTR;
            }
        } else if (base.mode & ATTR_ITALIC != 0 && self.dc.ifont.badslant)
            || (base.mode & ATTR_BOLD != 0 && self.dc.bfont.badweight)
        {
            base.fg = DEFAULTATTR;
        }

        unsafe fn alloc_color(
            dpy: *mut Display,
            vis: *mut Visual,
            cmap: Colormap,
            r: c_ushort,
            g: c_ushort,
            b: c_ushort,
            a: c_ushort,
        ) -> XftColor {
            let rc = XRenderColor {
                red: r,
                green: g,
                blue: b,
                alpha: a,
            };
            let mut c: XftColor = std::mem::zeroed();
            XftColorAllocValue(dpy, vis, cmap, &rc, &mut c);
            c
        }

        let mut fg_col: XftColor;
        let mut fg_idx: Option<usize>;
        if is_truecolor(base.fg) {
            fg_col = unsafe {
                alloc_color(
                    self.xw.dpy,
                    self.xw.vis,
                    self.xw.cmap,
                    ((base.fg & 0xff0000) >> 8) as c_ushort,
                    (base.fg & 0xff00) as c_ushort,
                    ((base.fg & 0xff) << 8) as c_ushort,
                    0xffff,
                )
            };
            fg_idx = None;
        } else {
            fg_idx = Some(base.fg as usize);
            fg_col = self.dc.col[base.fg as usize];
        }

        let mut bg_col: XftColor;
        let mut bg_idx: Option<usize>;
        if is_truecolor(base.bg) {
            bg_col = unsafe {
                alloc_color(
                    self.xw.dpy,
                    self.xw.vis,
                    self.xw.cmap,
                    ((base.bg & 0xff0000) >> 8) as c_ushort,
                    (base.bg & 0xff00) as c_ushort,
                    ((base.bg & 0xff) << 8) as c_ushort,
                    0xffff,
                )
            };
            bg_idx = None;
        } else {
            bg_idx = Some(base.bg as usize);
            bg_col = self.dc.col[base.bg as usize];
        }

        // Change basic system colours [0-7] to bright system colours [8-15].
        if base.mode & ATTR_BOLD_FAINT == ATTR_BOLD && between(base.fg as i32, 0, 7) {
            fg_idx = Some(base.fg as usize + 8);
            fg_col = self.dc.col[base.fg as usize + 8];
        }

        if self.win_is_set(MODE_REVERSE) {
            if fg_idx == Some(DEFAULTFG as usize) {
                fg_idx = Some(DEFAULTBG as usize);
                fg_col = self.dc.col[DEFAULTBG as usize];
            } else {
                fg_col = unsafe {
                    alloc_color(
                        self.xw.dpy,
                        self.xw.vis,
                        self.xw.cmap,
                        !fg_col.color.red,
                        !fg_col.color.green,
                        !fg_col.color.blue,
                        fg_col.color.alpha,
                    )
                };
                fg_idx = None;
            }

            if bg_idx == Some(DEFAULTBG as usize) {
                bg_idx = Some(DEFAULTFG as usize);
                bg_col = self.dc.col[DEFAULTFG as usize];
            } else {
                bg_col = unsafe {
                    alloc_color(
                        self.xw.dpy,
                        self.xw.vis,
                        self.xw.cmap,
                        !bg_col.color.red,
                        !bg_col.color.green,
                        !bg_col.color.blue,
                        bg_col.color.alpha,
                    )
                };
                bg_idx = None;
            }
        }

        if base.mode & ATTR_BOLD_FAINT == ATTR_FAINT {
            fg_col = unsafe {
                alloc_color(
                    self.xw.dpy,
                    self.xw.vis,
                    self.xw.cmap,
                    fg_col.color.red / 2,
                    fg_col.color.green / 2,
                    fg_col.color.blue / 2,
                    fg_col.color.alpha,
                )
            };
            fg_idx = None;
        }

        if base.mode & ATTR_REVERSE != 0 {
            std::mem::swap(&mut fg_col, &mut bg_col);
            std::mem::swap(&mut fg_idx, &mut bg_idx);
        }

        if base.mode & ATTR_BLINK != 0 && self.win_is_set(MODE_BLINK) {
            fg_col = bg_col;
            fg_idx = bg_idx;
        }

        if base.mode & ATTR_INVISIBLE != 0 {
            fg_col = bg_col;
            fg_idx = bg_idx;
        }
        let _ = (fg_idx, bg_idx);

        // Vertical gradient over the cell background.  To invert it, use
        // `y * ch` instead of `win_h - y * ch` in `gradient_alpha`.
        bg_col.color.alpha = gradient_alpha(y, self.win.ch, self.win.h);

        // Intelligent cleaning up of the borders.
        if x == 0 {
            self.xclear(
                0,
                if y == 0 { 0 } else { winy },
                config::BORDERPX,
                winy + self.win.ch
                    + if winy + self.win.ch >= config::BORDERPX + self.win.th {
                        self.win.h
                    } else {
                        0
                    },
            );
        }
        if winx + width >= config::BORDERPX + self.win.tw {
            self.xclear(
                winx + width,
                if y == 0 { 0 } else { winy },
                self.win.w,
                if winy + self.win.ch >= config::BORDERPX + self.win.th {
                    self.win.h
                } else {
                    winy + self.win.ch
                },
            );
        }
        if y == 0 {
            self.xclear(winx, 0, winx + width, config::BORDERPX);
        }
        if winy + self.win.ch >= config::BORDERPX + self.win.th {
            self.xclear(winx, winy + self.win.ch, winx + width, self.win.h);
        }

        // Clean up the region we want to draw to.
        unsafe {
            XftDrawRect(
                self.xw.draw,
                &bg_col,
                winx,
                winy,
                width as c_uint,
                self.win.ch as c_uint,
            );
        }

        // Set the clip region because Xft is sometimes dirty.
        let r = XRectangle {
            x: 0,
            y: 0,
            height: self.win.ch as c_ushort,
            width: width as c_ushort,
        };
        unsafe {
            XftDrawSetClipRectangles(self.xw.draw, winx, winy, &r, 1);
            XftDrawGlyphFontSpec(self.xw.draw, &fg_col, specs.as_ptr(), len as c_int);
        }

        // Underline and strikethrough.
        if base.mode & (ATTR_UNDERLINE | ATTR_URL) != 0 {
            unsafe {
                XftDrawRect(
                    self.xw.draw,
                    &fg_col,
                    winx,
                    winy + (self.dc.font.ascent as f32 * config::CHSCALE) as i32 + 1,
                    width as c_uint,
                    1,
                );
            }
        }
        if base.mode & ATTR_STRUCK != 0 {
            unsafe {
                XftDrawRect(
                    self.xw.draw,
                    &fg_col,
                    winx,
                    winy + (2.0 * self.dc.font.ascent as f32 * config::CHSCALE / 3.0) as i32,
                    width as c_uint,
                    1,
                );
            }
        }

        unsafe {
            XftDrawSetClip(self.xw.draw, ptr::null_mut());
        }
    }

    pub fn xdrawglyph(&mut self, g: Glyph, x: i32, y: i32) {
        let mut spec: [XftGlyphFontSpec; 1] = unsafe { std::mem::zeroed() };
        let numspecs = self.xmakeglyphfontspecs(&mut spec, &[g], x, y);
        self.xdrawglyphfontspecs(&spec[..numspecs], g, numspecs, x, y);
    }

    pub fn xdrawcursor(&mut self, cx: i32, cy: i32, mut g: Glyph, ox: i32, oy: i32, mut og: Glyph) {
        // remove the old cursor
        if self.selected(ox, oy) {
            og.mode ^= ATTR_REVERSE;
        }
        self.xdrawglyph(og, ox, oy);

        if self.win_is_set(MODE_HIDE) {
            return;
        }

        let drawcol: XftColor;

        // Select the right colour for the right mode.
        g.mode &= ATTR_BOLD | ATTR_ITALIC | ATTR_UNDERLINE | ATTR_STRUCK | ATTR_WIDE;

        if self.win_is_set(MODE_REVERSE) {
            g.mode |= ATTR_REVERSE;
            g.bg = DEFAULTFG;
            if self.selected(cx, cy) {
                drawcol = self.dc.col[config::DEFAULTCS as usize];
                g.fg = config::DEFAULTRCS;
            } else {
                drawcol = self.dc.col[config::DEFAULTRCS as usize];
                g.fg = config::DEFAULTCS;
            }
        } else {
            if self.selected(cx, cy) {
                g.fg = DEFAULTFG;
                g.bg = config::DEFAULTRCS;
            } else {
                g.fg = DEFAULTBG;
                g.bg = config::DEFAULTCS;
            }
            drawcol = self.dc.col[g.bg as usize];
        }

        // draw the new one
        if self.win_is_set(MODE_FOCUSED) {
            // Styles 0, 3, 5 and 7 are hidden during the blink-off half of
            // the cycle, which is tracked with MODE_BLINK by the run loop.
            let hidden = self.win_is_set(MODE_BLINK);
            match self.win.cursor {
                0 | 1 => {
                    if !hidden {
                        self.xdrawglyph(g, cx, cy);
                    }
                }
                2 => self.xdrawglyph(g, cx, cy),
                3 | 4 => {
                    if self.win.cursor == 4 || !hidden {
                        unsafe {
                            XftDrawRect(
                                self.xw.draw,
                                &drawcol,
                                config::BORDERPX + cx * self.win.cw,
                                config::BORDERPX + (cy + 1) * self.win.ch
                                    - config::CURSORTHICKNESS as i32,
                                self.win.cw as c_uint,
                                config::CURSORTHICKNESS,
                            );
                        }
                    }
                }
                5 | 6 => {
                    if self.win.cursor == 6 || !hidden {
                        unsafe {
                            XftDrawRect(
                                self.xw.draw,
                                &drawcol,
                                config::BORDERPX + cx * self.win.cw,
                                config::BORDERPX + cy * self.win.ch,
                                config::CURSORTHICKNESS,
                                self.win.ch as c_uint,
                            );
                        }
                    }
                }
                7 | 8 => {
                    if self.win.cursor == 8 || !hidden {
                        g.u = config::STCURSOR;
                        self.xdrawglyph(g, cx, cy);
                    }
                }
                _ => {}
            }
        } else {
            unsafe {
                XftDrawRect(
                    self.xw.draw,
                    &drawcol,
                    config::BORDERPX + cx * self.win.cw,
                    config::BORDERPX + cy * self.win.ch,
                    (self.win.cw - 1) as c_uint,
                    1,
                );
                XftDrawRect(
                    self.xw.draw,
                    &drawcol,
                    config::BORDERPX + cx * self.win.cw,
                    config::BORDERPX + cy * self.win.ch,
                    1,
                    (self.win.ch - 1) as c_uint,
                );
                XftDrawRect(
                    self.xw.draw,
                    &drawcol,
                    config::BORDERPX + (cx + 1) * self.win.cw - 1,
                    config::BORDERPX + cy * self.win.ch,
                    1,
                    (self.win.ch - 1) as c_uint,
                );
                XftDrawRect(
                    self.xw.draw,
                    &drawcol,
                    config::BORDERPX + cx * self.win.cw,
                    config::BORDERPX + (cy + 1) * self.win.ch - 1,
                    self.win.cw as c_uint,
                    1,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Titles, env, drawing
// ---------------------------------------------------------------------------

impl App {
    pub fn xsetenv(&mut self) {
        let buf = format!("{}", self.xw.win);
        let k = CString::new("WINDOWID").unwrap();
        let v = CString::new(buf).unwrap();
        unsafe {
            libc::setenv(k.as_ptr(), v.as_ptr(), 1);
        }
    }

    fn title_bytes(&self, p: Option<&[u8]>) -> Vec<u8> {
        let mut out = match p {
            Some(v) => v.to_vec(),
            None => self
                .opts
                .title
                .as_ref()
                .map(|v| v.as_bytes().to_vec())
                .unwrap_or_default(),
        };
        if out.is_empty() {
            if let Some(t) = &self.opts.title {
                out = t.as_bytes().to_vec();
            }
        }
        out
    }

    pub fn xseticontitle(&mut self, p: Option<&[u8]>) {
        let bytes = self.title_bytes(p);
        let c = CString::new(bytes).unwrap_or_else(|_| CString::new("").unwrap());
        let mut ptr = c.as_ptr() as *mut c_char;
        let mut prop: XTextProperty = unsafe { std::mem::zeroed() };
        let rc = unsafe {
            Xutf8TextListToTextProperty(
                self.xw.dpy,
                &mut ptr as *mut *mut c_char,
                1,
                XUTF8StringStyle,
                &mut prop,
            )
        };
        if rc != Success as c_int {
            return;
        }
        unsafe {
            XSetWMIconName(self.xw.dpy, self.xw.win, &mut prop);
            XSetTextProperty(self.xw.dpy, self.xw.win, &mut prop, self.xw.netwmiconname);
            XFree(prop.value as *mut c_void);
        }
    }

    pub fn xsettitle(&mut self, p: Option<&[u8]>) {
        let bytes = self.title_bytes(p);
        let c = CString::new(bytes).unwrap_or_else(|_| CString::new("").unwrap());
        let mut ptr = c.as_ptr() as *mut c_char;
        let mut prop: XTextProperty = unsafe { std::mem::zeroed() };
        let rc = unsafe {
            Xutf8TextListToTextProperty(
                self.xw.dpy,
                &mut ptr as *mut *mut c_char,
                1,
                XUTF8StringStyle,
                &mut prop,
            )
        };
        if rc != Success as c_int {
            return;
        }
        unsafe {
            XSetWMName(self.xw.dpy, self.xw.win, &mut prop);
            XSetTextProperty(self.xw.dpy, self.xw.win, &mut prop, self.xw.netwmname);
            XFree(prop.value as *mut c_void);
        }
    }

    pub fn xstartdraw(&self) -> bool {
        self.win_is_set(MODE_VISIBLE)
    }

    pub fn xdrawline(&mut self, line: &[Glyph], x1: i32, y1: i32, x2: i32) {
        let mut specs_buf = std::mem::take(&mut self.xw.specbuf);
        let numspecs =
            self.xmakeglyphfontspecs(&mut specs_buf, &line[x1 as usize..x2 as usize], x1, y1);

        let mut i = 0usize;
        let mut ox = 0i32;
        let mut off = 0usize;
        let mut remaining = numspecs;
        let mut base = Glyph::default();

        let mut x = x1;
        while x < x2 && i < remaining {
            let mut new = line[x as usize];
            if new.mode == ATTR_WDUMMY {
                x += 1;
                continue;
            }
            if self.selected(x, y1) {
                new.mode ^= ATTR_REVERSE;
            }
            if i > 0 && (base.mode != new.mode || base.fg != new.fg || base.bg != new.bg) {
                self.xdrawglyphfontspecs(&specs_buf[off..off + i], base, i, ox, y1);
                off += i;
                remaining -= i;
                i = 0;
            }
            if i == 0 {
                ox = x;
                base = new;
            }
            i += 1;
            x += 1;
        }
        if i > 0 {
            self.xdrawglyphfontspecs(&specs_buf[off..off + i], base, i, ox, y1);
        }

        self.xw.specbuf = specs_buf;
    }

    pub fn xfinishdraw(&mut self) {
        unsafe {
            XCopyArea(
                self.xw.dpy,
                self.xw.buf,
                self.xw.win,
                self.dc.gc,
                0,
                0,
                self.win.w as c_uint,
                self.win.h as c_uint,
                0,
                0,
            );
            let idx = if self.win_is_set(MODE_REVERSE) {
                DEFAULTFG
            } else {
                DEFAULTBG
            } as usize;
            XSetForeground(self.xw.dpy, self.dc.gc, self.dc.col[idx].pixel);
        }
    }

    pub fn xximspot(&mut self, x: i32, y: i32) {
        if self.xw.ime.xic.is_null() {
            return;
        }
        self.xw.ime.spot.x = (config::BORDERPX + x * self.win.cw) as i16;
        self.xw.ime.spot.y = (config::BORDERPX + (y + 1) * self.win.ch) as i16;
        unsafe {
            XSetICValues(
                self.xw.ime.xic,
                XNPreeditAttributes.as_ptr(),
                self.xw.ime.spotlist,
                ptr::null::<c_char>(),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Event handlers
// ---------------------------------------------------------------------------

impl App {
    pub fn expose(&mut self, _ev: &XEvent) {
        self.redraw();
    }

    pub fn visibility(&mut self, ev: &XEvent) {
        let e = unsafe { ev.visibility };
        modbit_u(
            &mut self.win.mode,
            e.state != VisibilityFullyObscured,
            MODE_VISIBLE,
        );
    }

    pub fn unmap(&mut self, _ev: &XEvent) {
        self.win.mode &= !MODE_VISIBLE;
    }

    pub fn xsetpointermotion(&mut self, set: bool) {
        modbit_l(
            &mut self.xw.attrs.event_mask,
            set,
            PointerMotionMask as c_long,
        );
        unsafe {
            XChangeWindowAttributes(self.xw.dpy, self.xw.win, CWEventMask, &mut self.xw.attrs);
        }
    }

    pub fn xsetmode(&mut self, set: bool, flags: u32) {
        let mode = self.win.mode;
        modbit_u(&mut self.win.mode, set, flags);
        if (self.win.mode & MODE_REVERSE) != (mode & MODE_REVERSE) {
            self.redraw();
        }
    }

    pub fn xsetcursor(&mut self, cursor: i32) -> bool {
        if !(0..=8).contains(&cursor) {
            return true;
        }
        self.win.cursor = cursor as u32;
        self.cursorblinks = matches!(self.win.cursor, 0 | 1 | 3 | 5 | 7);
        false
    }

    pub fn xseturgency(&mut self, add: bool) {
        unsafe {
            let h = XGetWMHints(self.xw.dpy, self.xw.win);
            if !h.is_null() {
                modbit_l(&mut (*h).flags, add, XUrgencyHint as c_long);
                XSetWMHints(self.xw.dpy, self.xw.win, h);
                XFree(h as *mut c_void);
            }
        }
    }

    pub fn xbell(&mut self) {
        if !self.win_is_set(MODE_FOCUSED) {
            self.xseturgency(true);
        }
        if config::BELLVOLUME != 0 {
            unsafe {
                XkbBell(self.xw.dpy, self.xw.win, config::BELLVOLUME, 0);
            }
        }
    }

    pub fn focus(&mut self, ev: &XEvent) {
        let e = unsafe { ev.focus_change };
        if e.mode == NotifyGrab {
            return;
        }

        if etype(ev) == FocusIn {
            unsafe {
                if !self.xw.ime.xic.is_null() {
                    XSetICFocus(self.xw.ime.xic);
                }
            }
            self.win.mode |= MODE_FOCUSED;
            self.xseturgency(false);
            if self.win_is_set(MODE_FOCUS) {
                self.ttywrite(b"\x1b[I", false);
            }
        } else {
            unsafe {
                if !self.xw.ime.xic.is_null() {
                    XUnsetICFocus(self.xw.ime.xic);
                }
            }
            self.win.mode &= !MODE_FOCUSED;
            if self.win_is_set(MODE_FOCUS) {
                self.ttywrite(b"\x1b[O", false);
            }
        }
    }

    pub fn match_mask(&self, mask: u32, state: u32) -> bool {
        mask == config::XK_ANY_MOD || mask == (state & !config::IGNOREMOD)
    }

    /// Bytes configured in `HOTKEYS` for this key event, if any.
    ///
    /// Caps Lock is ignored, so e.g. Shift+Enter keeps working with it on.
    pub fn hotkey(&self, k: KeySym, state: u32) -> Option<&'static [u8]> {
        let state = state & !(config::IGNOREMOD | LockMask);
        config::HOTKEYS
            .iter()
            .find(|hk| hk.keysym == k && (hk.mask == config::XK_ANY_MOD || hk.mask == state))
            .map(|hk| hk.seq)
    }

    pub fn kmap(&self, k: KeySym, state: u32) -> Option<&'static [u8]> {
        let mut i = 0usize;
        while i < config::MAPPEDKEYS.len() {
            if config::MAPPEDKEYS[i] == k {
                break;
            }
            i += 1;
        }
        if i == config::MAPPEDKEYS.len() && (k & 0xFFFF) < 0xFD00 {
            return None;
        }

        for kp in config::KEY {
            if kp.k != k {
                continue;
            }
            if !self.match_mask(kp.mask, state) {
                continue;
            }
            if if self.win_is_set(MODE_APPKEYPAD) {
                kp.appkey < 0
            } else {
                kp.appkey > 0
            } {
                continue;
            }
            if self.win_is_set(MODE_NUMLOCK) && kp.appkey == 2 {
                continue;
            }
            if if self.win_is_set(MODE_APPCURSOR) {
                kp.appcursor < 0
            } else {
                kp.appcursor > 0
            } {
                continue;
            }
            return Some(kp.s);
        }
        None
    }

    pub fn kpress(&mut self, ev: &XEvent) {
        let mut e = unsafe { ev.key };
        let mut ksym: KeySym = 0;
        let mut buf = [0u8; 64];
        let mut status: c_int = 0;
        let len: c_int;

        if self.win_is_set(MODE_KBDLOCK) {
            return;
        }

        unsafe {
            if !self.xw.ime.xic.is_null() {
                len = XmbLookupString(
                    self.xw.ime.xic,
                    &mut e,
                    buf.as_mut_ptr() as *mut c_char,
                    buf.len() as c_int,
                    &mut ksym,
                    &mut status,
                );
                if status == XBufferOverflow {
                    return;
                }
            } else {
                len = XLookupString(
                    &mut e,
                    buf.as_mut_ptr() as *mut c_char,
                    buf.len() as c_int,
                    &mut ksym,
                    ptr::null_mut(),
                );
            }
        }

        // Highlight URLs while Ctrl is held; Ctrl+Click follows them.
        if self.is_ctrl_key(e.keycode as c_uint) {
            self.highlighturls();
        }

        // 1. user-defined hotkeys (they may override the built-in bindings)
        if let Some(seq) = self.hotkey(ksym, e.state) {
            self.ttywrite(seq, true);
            return;
        }

        // 2. shortcuts
        for bp in config::SHORTCUTS {
            if ksym == bp.keysym && self.match_mask(bp.mask, e.state) {
                (bp.func)(self, &bp.arg);
                return;
            }
        }

        // 3. custom keys from config
        if let Some(customkey) = self.kmap(ksym, e.state) {
            self.ttywrite(customkey, true);
            return;
        }

        // 4. composed string from input method
        if len == 0 {
            return;
        }
        let mut outlen = len as usize;
        if len == 1 && e.state & Mod1Mask != 0 {
            if self.win_is_set(MODE_8BIT) {
                if buf[0] < 0o177 {
                    let c = buf[0] as u32 | 0x80;
                    let mut enc = [0u8; UTF_SIZ];
                    outlen = utf8encode(c, &mut enc);
                    buf[..outlen].copy_from_slice(&enc[..outlen]);
                }
            } else {
                buf[1] = buf[0];
                buf[0] = 0x1b;
                outlen = 2;
            }
        }
        self.ttywrite(&buf[..outlen], true);
    }

    /// True when `keycode` is one of the Control keys.  Deriving this from the
    /// keycode rather than the looked-up string also works with an input
    /// method, which need not return modifier keysyms.
    fn is_ctrl_key(&self, keycode: c_uint) -> bool {
        unsafe {
            let ks = XkbKeycodeToKeysym(self.xw.dpy, keycode as u8, 0, 0);
            ks == x11::keysym::XK_Control_L as KeySym || ks == x11::keysym::XK_Control_R as KeySym
        }
    }

    /// Ctrl released: drop the URL underlines.
    pub fn krelease(&mut self, ev: &XEvent) {
        let e = unsafe { ev.key };
        if self.is_ctrl_key(e.keycode as c_uint) {
            self.unhighlighturls();
        }
    }

    pub fn cmessage(&mut self, ev: &XEvent) {
        let e = unsafe { ev.client_message };
        if e.message_type == self.xw.xembed && e.format == 32 {
            if e.data.get_long(1) == XEMBED_FOCUS_IN {
                self.win.mode |= MODE_FOCUSED;
                self.xseturgency(false);
            } else if e.data.get_long(1) == XEMBED_FOCUS_OUT {
                self.win.mode &= !MODE_FOCUSED;
            }
        } else if e.data.get_long(0) == self.xw.wmdeletewin as c_long {
            self.ttyhangup();
            std::process::exit(0);
        } else if e.message_type == self.xw.xdnd_enter {
            self.xw.xdnd_source_win = e.data.get_long(0) as Window;
            self.xw.xdnd_source_version = e.data.get_long(1) >> 24;
            self.xw.xdnd_source_format = 0;
            if self.xw.xdnd_source_version <= 5 {
                self.xdndenter(ev);
            }
        } else if e.message_type == self.xw.xdnd_position && self.xw.xdnd_source_version <= 5 {
            self.xdndpos(ev);
        } else if e.message_type == self.xw.xdnd_drop && self.xw.xdnd_source_version <= 5 {
            self.xdnddrop(ev);
        }
    }

    /// An XDND reply event addressed to the drag source.  `data` fills the
    /// four payload slots after the destination window.
    fn xdndreply(&self, message_type: Atom, data: [c_long; 4]) -> XEvent {
        let mut ev: XEvent = unsafe { std::mem::zeroed() };
        unsafe {
            let mut cm = ev.client_message;
            cm.type_ = ClientMessage;
            cm.window = self.xw.xdnd_source_win;
            cm.message_type = message_type;
            cm.format = 32;
            cm.data.set_long(0, self.xw.win as c_long);
            for (i, v) in data.iter().enumerate() {
                cm.data.set_long(i + 1, *v);
            }
            ev.client_message = cm;
        }
        ev
    }

    fn xdndsend(&self, mut ev: XEvent) {
        unsafe {
            XSendEvent(
                self.xw.dpy,
                self.xw.xdnd_source_win,
                False,
                NoEventMask,
                &mut ev,
            );
            XFlush(self.xw.dpy);
        }
    }

    /// XdndEnter: pick text/uri-list or text/plain if the source offers one.
    fn xdndenter(&mut self, ev: &XEvent) {
        let e = unsafe { ev.client_message };
        let mut formats: Vec<Atom> = Vec::new();

        if e.data.get_long(1) & 1 != 0 {
            // The formats are in the source's XdndTypeList property.
            let mut actual_type: Atom = 0;
            let mut actual_format: c_int = 0;
            let mut count: c_ulong = 0;
            let mut bytes_after: c_ulong = 0;
            let mut data: *mut libc::c_uchar = ptr::null_mut();

            let rc = unsafe {
                XGetWindowProperty(
                    self.xw.dpy,
                    self.xw.xdnd_source_win,
                    self.xw.xdnd_type_list,
                    0,
                    c_long::MAX,
                    False,
                    XA_ATOM,
                    &mut actual_type,
                    &mut actual_format,
                    &mut count,
                    &mut bytes_after,
                    &mut data,
                )
            };
            if rc == 0 && !data.is_null() {
                let atoms =
                    unsafe { std::slice::from_raw_parts(data as *const Atom, count as usize) };
                formats.extend_from_slice(atoms);
                unsafe { XFree(data as *mut c_void) };
            }
        } else {
            for i in 2..=4 {
                let a = e.data.get_long(i) as Atom;
                if a != 0 {
                    formats.push(a);
                }
            }
        }

        for f in formats {
            if f == self.xw.xtext_uri_list || f == self.xw.xtext_plain {
                self.xw.xdnd_source_format = f;
                break;
            }
        }
    }

    /// XdndPosition: accept the drop when a supported type was offered.
    fn xdndpos(&mut self, _ev: &XEvent) {
        let reply = if self.xw.xdnd_source_format != 0 {
            // flags: accept; no rectangle; action in l[4] since version 2.
            let action = if self.xw.xdnd_source_version >= 2 {
                self.xw.xdnd_action_copy as c_long
            } else {
                0
            };
            self.xdndreply(self.xw.xdnd_status, [1, 0, 0, action])
        } else {
            self.xdndreply(self.xw.xdnd_status, [0, 0, 0, 0])
        };
        self.xdndsend(reply);
    }

    /// XdndDrop: ask the source for the dropped data.
    fn xdnddrop(&mut self, ev: &XEvent) {
        let e = unsafe { ev.client_message };

        if self.xw.xdnd_source_format != 0 {
            let mut time = CurrentTime;
            if self.xw.xdnd_source_version >= 1 {
                time = e.data.get_long(2) as Time;
            }
            unsafe {
                XConvertSelection(
                    self.xw.dpy,
                    self.xw.xdnd_selection,
                    self.xw.xdnd_source_format,
                    self.xw.xdnd_selection,
                    self.xw.win,
                    time,
                );
            }
        } else if self.xw.xdnd_source_version >= 2 {
            let reply = self.xdndreply(self.xw.xdnd_finished, [0, 0, 0, 0]);
            self.xdndsend(reply);
        }
    }

    /// SelectionNotify for the converted XdndSelection: paste the payload and
    /// tell the source the drop is finished.
    fn xdndsel(&mut self, ev: &XEvent) {
        let e = unsafe { ev.selection };
        let mut actual_type: Atom = 0;
        let mut actual_format: c_int = 0;
        let mut nitems: c_ulong = 0;
        let mut bytes_after: c_ulong = 0;
        let mut data: *mut libc::c_uchar = ptr::null_mut();

        let rc = unsafe {
            XGetWindowProperty(
                self.xw.dpy,
                e.requestor,
                e.property,
                0,
                c_long::MAX,
                False,
                e.target,
                &mut actual_type,
                &mut actual_format,
                &mut nitems,
                &mut bytes_after,
                &mut data,
            )
        };

        if rc == 0 && nitems > 0 && !data.is_null() {
            let bytes = unsafe { std::slice::from_raw_parts(data, nitems as usize) };
            self.xdndpastedata(bytes);
        }
        if !data.is_null() {
            unsafe { XFree(data as *mut c_void) };
        }

        if rc != 0 || nitems == 0 {
            return;
        }

        if self.xw.xdnd_source_version >= 2 {
            // data.l[1] bit 0 is "accept", data.l[2] the performed action.
            let reply = self.xdndreply(
                self.xw.xdnd_finished,
                [1, self.xw.xdnd_action_copy as c_long, 0, 0],
            );
            self.xdndsend(reply);
        }
    }

    fn xdndpastedata(&mut self, data: &[u8]) {
        let text = xdnd_decode(data);
        if text.is_empty() {
            return;
        }
        self.xsetsel(text);
        self.selpaste(&Arg::i(0));
    }

    pub fn resize(&mut self, ev: &XEvent) {
        let e = unsafe { ev.configure };
        if e.width == self.win.w && e.height == self.win.h {
            return;
        }
        self.cresize(e.width, e.height);
    }

    fn handle_event(&mut self, ev: &XEvent) {
        match etype(ev) {
            KeyPress => self.kpress(ev),
            KeyRelease => self.krelease(ev),
            ClientMessage => self.cmessage(ev),
            ConfigureNotify => self.resize(ev),
            VisibilityNotify => self.visibility(ev),
            UnmapNotify => self.unmap(ev),
            Expose => self.expose(ev),
            FocusIn | FocusOut => self.focus(ev),
            MotionNotify => self.bmotion(ev),
            ButtonPress => self.bpress(ev),
            ButtonRelease => self.brelease(ev),
            SelectionNotify => self.selnotify(ev),
            PropertyNotify => self.propnotify(ev),
            SelectionRequest => self.selrequest(ev),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Drag-and-drop decoding
// ---------------------------------------------------------------------------

fn hex_to_int(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Append `src` to `out` with %xx escapes decoded and everything in
/// `XDNDESCCHAR` backslash-escaped, then a trailing space.
fn xdndurldecode(src: &[u8], out: &mut Vec<u8>) {
    let escchars = config::XDNDESCCHAR.as_bytes();
    let mut i = 0;
    while i < src.len() {
        let mut c = src[i];
        if c == b'%' && i + 2 < src.len() {
            if let (Some(hi), Some(lo)) = (hex_to_int(src[i + 1]), hex_to_int(src[i + 2])) {
                c = (hi << 4) | lo;
                i += 3;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
        if escchars.contains(&c) {
            out.push(b'\\');
        }
        out.push(c);
    }
    out.push(b' ');
}

/// Turn a dropped `text/uri-list` / `text/plain` payload into text for the
/// terminal: the `file://` prefix is removed from each URI, `\r\n`-separated
/// items become space-separated shell-quoted paths, and empty lines are
/// skipped.
fn xdnd_decode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 2);
    for line in data.split(|&b| b == b'\n' || b == b'\r') {
        if line.is_empty() {
            continue;
        }
        let line = line.strip_prefix(&b"file://"[..]).unwrap_or(line);
        xdndurldecode(line, &mut out);
    }
    out
}

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------

impl App {
    pub fn run(&mut self) -> ! {
        let mut ev: XEvent = unsafe { std::mem::zeroed() };
        let mut w = self.win.w;
        let mut h = self.win.h;

        // Wait for the window to be mapped.
        loop {
            unsafe { XNextEvent(self.xw.dpy, &mut ev) };
            if unsafe { XFilterEvent(&mut ev, 0) } != 0 {
                continue;
            }
            if etype(&ev) == ConfigureNotify {
                let e = unsafe { ev.configure };
                w = e.width;
                h = e.height;
            }
            if etype(&ev) == MapNotify {
                break;
            }
        }

        let ttyfd = {
            let line = self.opts.line.clone();
            let io = self.opts.io.clone();
            let cmd = config::SHELL.to_string();
            let args_owned: Option<Vec<CString>> = if self.opts.cmd.is_empty() {
                None
            } else {
                Some(self.opts.cmd.clone())
            };
            self.ttynew(line.as_ref(), &cmd, io.as_ref(), args_owned.as_deref())
        };
        self.cresize(w, h);

        let xfd = unsafe { XConnectionNumber(self.xw.dpy) };
        let mut timeout: f64 = -1.0;
        let mut drawing = false;
        let mut lastblink: Option<Instant> = None;
        let mut trigger = Instant::now();

        loop {
            let mut rfd: libc::fd_set = unsafe { std::mem::zeroed() };
            unsafe {
                libc::FD_ZERO(&mut rfd);
                libc::FD_SET(ttyfd, &mut rfd);
                libc::FD_SET(xfd, &mut rfd);
            }

            if unsafe { XPending(self.xw.dpy) } != 0 {
                timeout = 0.0; // existing events might not set xfd
            }

            let seltv = libc::timespec {
                tv_sec: (timeout / 1e3) as i64,
                tv_nsec: (1e6 * (timeout - 1e3 * (timeout / 1e3).floor())) as i64,
            };
            let tv = if timeout >= 0.0 {
                &seltv as *const libc::timespec
            } else {
                ptr::null()
            };

            let rc = unsafe {
                libc::pselect(
                    max(xfd, ttyfd) + 1,
                    &mut rfd,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    tv,
                    ptr::null(),
                )
            };
            if rc < 0 {
                let e = unsafe { *libc::__errno_location() };
                if e == libc::EINTR {
                    continue;
                }
                die(&format!("select failed: {}\n", errno_message(e)));
            }

            let now = Instant::now();

            if unsafe { libc::FD_ISSET(ttyfd, &rfd) } {
                self.ttyread();
            }

            let mut xev = false;
            while unsafe { XPending(self.xw.dpy) } != 0 {
                xev = true;
                unsafe { XNextEvent(self.xw.dpy, &mut ev) };
                if unsafe { XFilterEvent(&mut ev, 0) } != 0 {
                    continue;
                }
                self.handle_event(&ev);
            }

            // Wait a little for more content before drawing, to reduce tearing.
            if unsafe { libc::FD_ISSET(ttyfd, &rfd) } || xev {
                if !drawing {
                    trigger = now;
                    // Make a blinking cursor visible again until the screen
                    // settles, and restart its blink cycle.
                    if self.win_is_set(MODE_BLINK) {
                        self.win.mode ^= MODE_BLINK;
                    }
                    lastblink = Some(now);
                    drawing = true;
                }
                timeout = (MAXLATENCY - now.saturating_duration_since(trigger).as_millis() as f64)
                    / MAXLATENCY
                    * MINLATENCY;
                if timeout > 0.0 {
                    continue; // we have time, try to find idle
                }
            }

            // idle detected or maxlatency exhausted -> draw
            timeout = -1.0;
            if config::BLINKTIMEOUT != 0 && (self.cursorblinks || self.tattrset(ATTR_BLINK)) {
                let elapsed = lastblink
                    .map(|t| now.saturating_duration_since(t).as_millis() as i64)
                    .unwrap_or(i64::MAX / 2);
                let mut to = config::BLINKTIMEOUT as i64 - elapsed;
                if to <= 0 {
                    if -to > config::BLINKTIMEOUT as i64 {
                        // start visible
                        self.win.mode |= MODE_BLINK;
                    }
                    self.win.mode ^= MODE_BLINK;
                    self.tsetdirtattr(ATTR_BLINK);
                    lastblink = Some(now);
                    to = config::BLINKTIMEOUT as i64;
                }
                timeout = to as f64;
            }

            self.draw();
            unsafe { XFlush(self.xw.dpy) };
            drawing = false;
        }
    }
}

pub fn usage(argv0: &str) -> ! {
    die(&format!(
        "usage: {} [-aiv] [-c class] [-f font] [-g geometry] [-n name] [-o file]\n\
         \x20         [-T title] [-t title] [-w windowid] [[-e] command [args ...]]\n\
         \x20      {} [-aiv] [-c class] [-f font] [-g geometry] [-n name] [-o file]\n\
         \x20         [-T title] [-t title] [-w windowid] -l line [stty_args ...]\n",
        argv0, argv0
    ))
}

// ---------------------------------------------------------------------------
// Command line
// ---------------------------------------------------------------------------

pub struct ParsedArgs {
    pub opts: Options,
    pub cols: u32,
    pub rows: u32,
}

/// Port of the ARGBEGIN/ARGEND argument handling in `main()`.
pub fn parse_args(argv: &[std::ffi::OsString], app: &mut App, argv0: &str) -> ParsedArgs {
    let mut opts = Options::default();
    let mut cols = config::COLS;
    let mut rows = config::ROWS;
    let mut i = 1usize;
    let mut command_start: Option<usize> = None;

    'outer: while i < argv.len() {
        let arg = argv[i].to_string_lossy().into_owned();
        if !arg.starts_with('-') || arg.len() == 1 {
            break;
        }
        if arg == "--" {
            i += 1;
            command_start = Some(i);
            break;
        }

        let chars: Vec<char> = arg.chars().skip(1).collect();
        let mut ci = 0usize;
        while ci < chars.len() {
            let c = chars[ci];
            match c {
                'a' => app.allowaltscreen = false,
                'i' => app.xw.isfixed = true,
                'v' => die(&format!("{} {}\n", argv0, env!("CARGO_PKG_VERSION"))),
                'e' => {
                    // Remaining args are the command; the rest of this
                    // argument is ignored, like the C version.
                    command_start = Some(i + 1);
                    break 'outer;
                }
                'c' | 'f' | 'g' | 'o' | 'l' | 'n' | 't' | 'T' | 'w' | 'A' => {
                    let rest: String = chars[ci + 1..].iter().collect();
                    let value = if !rest.is_empty() {
                        rest
                    } else {
                        i += 1;
                        if i >= argv.len() {
                            usage(argv0);
                        }
                        argv[i].to_string_lossy().into_owned()
                    };

                    match c {
                        'c' => opts.class = Some(CString::new(value).unwrap()),
                        'f' => opts.font = Some(CString::new(value).unwrap()),
                        'o' => opts.io = Some(CString::new(value).unwrap()),
                        'l' => opts.line = Some(CString::new(value).unwrap()),
                        'n' => opts.name = Some(CString::new(value).unwrap()),
                        't' | 'T' => opts.title = Some(CString::new(value).unwrap()),
                        'w' => opts.embed = Some(CString::new(value).unwrap()),
                        'A' => {
                            app.alpha = value
                                .parse::<f32>()
                                .unwrap_or(0.0)
                                .clamp(0.0, 1.0);
                        }
                        'g' => {
                            let g = CString::new(value).unwrap();
                            let mut x = app.xw.l;
                            let mut y = app.xw.t;
                            let mut w = cols;
                            let mut h = rows;
                            app.xw.gm = unsafe {
                                XParseGeometry(g.as_ptr(), &mut x, &mut y, &mut w, &mut h)
                            };
                            app.xw.l = x;
                            app.xw.t = y;
                            cols = w;
                            rows = h;
                        }
                        _ => unreachable!(),
                    }
                    break; // the rest of this argument was the value
                }
                _ => usage(argv0),
            }
            ci += 1;
        }
        i += 1;
    }

    if command_start.is_none() && i < argv.len() {
        command_start = Some(i);
    }
    if let Some(start) = command_start {
        opts.cmd = argv[start..].iter().map(|s| os_to_cstring(s)).collect();
    }

    ParsedArgs { opts, cols, rows }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_to_int_accepts_both_cases() {
        assert_eq!(hex_to_int(b'0'), Some(0));
        assert_eq!(hex_to_int(b'9'), Some(9));
        assert_eq!(hex_to_int(b'a'), Some(10));
        assert_eq!(hex_to_int(b'F'), Some(15));
        assert_eq!(hex_to_int(b'g'), None);
        assert_eq!(hex_to_int(b'/'), None);
    }

    #[test]
    fn configured_hotkeys_are_matched() {
        let app = App::new(Options::default());
        let ret = x11::keysym::XK_Return as KeySym;
        let kp_enter = x11::keysym::XK_KP_Enter as KeySym;
        let end = x11::keysym::XK_End as KeySym;

        assert_eq!(app.hotkey(ret, ShiftMask), Some(&b"\x1b\r"[..]));
        assert_eq!(app.hotkey(kp_enter, ShiftMask), Some(&b"\x1b\r"[..]));

        // Ctrl+Home/End are application keys encoded as rxvt-style sequences.
        let home = x11::keysym::XK_Home as KeySym;
        assert_eq!(app.hotkey(home, ControlMask), Some(&b"\x1b[7^"[..]));
        assert_eq!(app.hotkey(end, ControlMask), Some(&b"\x1b[8^"[..]));

        // The modifiers must match exactly, apart from the ignored ones.
        assert_eq!(app.hotkey(ret, 0), None);
        assert_eq!(app.hotkey(end, 0), None);
        assert_eq!(app.hotkey(end, ControlMask | ShiftMask), None);
        assert_eq!(
            app.hotkey(ret, ShiftMask | LockMask),
            Some(&b"\x1b\r"[..])
        );
    }

    #[test]
    fn xdnd_decodes_file_uris() {
        assert_eq!(
            xdnd_decode(b"file:///tmp/foo.txt\r\n"),
            b"/tmp/foo.txt ".to_vec()
        );
        assert_eq!(
            xdnd_decode(b"file:///tmp/a%20b.txt\r\nfile:///tmp/c%23d.txt\r\n"),
            b"/tmp/a\\ b.txt /tmp/c\\#d.txt ".to_vec()
        );
    }

    #[test]
    fn xdnd_escapes_shell_metacharacters() {
        assert_eq!(
            xdnd_decode(b"file:///tmp/a&b;c(d)$e.txt"),
            b"/tmp/a\\&b\\;c\\(d\\)\\$e.txt ".to_vec()
        );
        assert_eq!(xdnd_decode(b"100%"), b"100% ".to_vec());
        assert_eq!(xdnd_decode(b"a'b\"c"), b"a\\'b\\\"c ".to_vec());
    }

    #[test]
    fn xdnd_keeps_bad_escapes_and_plain_text() {
        // Not a valid %xx escape: copied literally ('%' is not escaped).
        assert_eq!(xdnd_decode(b"100%zz"), b"100%zz ".to_vec());
        assert_eq!(xdnd_decode(b"end%2"), b"end%2 ".to_vec());
        // Lowercase hex, and text/plain payloads.
        assert_eq!(xdnd_decode(b"%2f%2F"), b"// ".to_vec());
        assert_eq!(xdnd_decode(b"hello world"), b"hello\\ world ".to_vec());
        // Empty payloads and blank lines produce nothing.
        assert!(xdnd_decode(b"").is_empty());
        assert!(xdnd_decode(b"\r\n\r\n").is_empty());
    }
}

#[cfg(test)]
mod alpha_tests {
    use super::*;

    #[test]
    fn alpha_components_scale() {
        assert_eq!(alpha_components(0.0), (0, 0));
        assert_eq!(alpha_components(1.0), (0xffff, (0xff as c_ulong) << 24));
        assert_eq!(alpha_components(0.8), (52428, (0xcc as c_ulong) << 24));
    }

    #[test]
    fn alpha_option_is_clamped() {
        for (arg, want) in [
            ("0.5", 0.5f32),
            ("0", 0.0),
            ("1", 1.0),
            ("-1", 0.0),
            ("2", 1.0),
            ("nonsense", 0.0),
        ] {
            let mut app = App::new(Options::default());
            let argv: Vec<std::ffi::OsString> =
                ["st", "-A", arg].iter().map(std::ffi::OsString::from).collect();
            parse_args(&argv, &mut app, "st");
            assert_eq!(app.alpha, want, "-A {}", arg);
        }
    }

    #[test]
    fn gradient_alpha_ramp() {
        // The top row is GRAD_ALPHA + STAT_ALPHA (fully opaque).
        assert_eq!(gradient_alpha(0, 14, 144), 0xffff);
        let top = gradient_alpha(0, 14, 144) as u32;
        let mid = gradient_alpha(4, 14, 144) as u32;
        let bottom = gradient_alpha(9, 14, 144) as u32;
        assert!(top > mid && mid > bottom, "{} {} {}", top, mid, bottom);
        // Even the last row keeps at least STAT_ALPHA.
        assert!(bottom >= (config::STAT_ALPHA * 0xffff as f32) as u32);
        // A zero-height window cannot divide by zero.
        assert_eq!(gradient_alpha(0, 14, 0), 0xffff);
    }
}
