//! Terminal emulator core: UTF-8, escape sequences, screen buffers, selection
//! and the pty.  This is a direct port of the original `st.c`.

use std::collections::VecDeque;
use std::ffi::{CStr, CString};
use std::ptr;
use std::sync::atomic::Ordering;

extern "C" {
    fn wcwidth(c: libc::wchar_t) -> libc::c_int;
}

use crate::app::*;
use crate::config;
use crate::config::{DEFAULTBG, DEFAULTFG, TABSPACES};

// ---------------------------------------------------------------------------
// Small I/O helpers
// ---------------------------------------------------------------------------

/// `xwrite`: write the whole buffer, returning the number of bytes queued.
pub unsafe fn xwrite(fd: i32, s: &[u8]) -> isize {
    let aux = s.len() as isize;
    let mut off = 0usize;
    while off < s.len() {
        let r = libc::write(fd, s[off..].as_ptr() as *const libc::c_void, s.len() - off);
        if r < 0 {
            return r;
        }
        off += r as usize;
    }
    aux
}

fn read_errno() -> i32 {
    unsafe { *libc::__errno_location() }
}

fn errno_message(e: i32) -> String {
    unsafe {
        CStr::from_ptr(libc::strerror(e))
            .to_string_lossy()
            .into_owned()
    }
}

fn last_errno_message() -> String {
    errno_message(read_errno())
}

fn cstr_to_string(p: *const libc::c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() }
    }
}

// ---------------------------------------------------------------------------
// UTF-8
// ---------------------------------------------------------------------------

const UTFBYTE: [u8; UTF_SIZ + 1] = [0x80, 0, 0xC0, 0xE0, 0xF0];
const UTFMASK: [u8; UTF_SIZ + 1] = [0xC0, 0x80, 0xE0, 0xF0, 0xF8];
const UTFMIN: [Rune; UTF_SIZ + 1] = [0, 0, 0x80, 0x800, 0x10000];
const UTFMAX: [Rune; UTF_SIZ + 1] = [0x10FFFF, 0x7F, 0x7FF, 0xFFFF, 0x10FFFF];

fn utf8decodebyte(c: u8, len: &mut usize) -> Rune {
    for i in 0..UTFMASK.len() {
        if c & UTFMASK[i] == UTFBYTE[i] {
            *len = i;
            return (c & !UTFMASK[i]) as Rune;
        }
    }
    *len = UTFMASK.len(); // C's for-loop leaves i == LEN on failure
    0
}

pub fn utf8validate(u: &mut Rune, mut i: usize) -> usize {
    if !between(*u as i32, UTFMIN[i] as i32, UTFMAX[i] as i32) || between(*u as i32, 0xD800, 0xDFFF)
    {
        *u = UTF_INVALID;
    }
    i = 1;
    while *u > UTFMAX[i] {
        i += 1;
    }
    i
}

fn utf8encodebyte(u: Rune, i: usize) -> u8 {
    UTFBYTE[i] | (u & !(UTFMASK[i] as Rune)) as u8
}

/// Decode one UTF-8 character.  Returns bytes consumed (0 if incomplete).
pub fn utf8decode(c: &[u8], u: &mut Rune) -> usize {
    *u = UTF_INVALID;
    if c.is_empty() {
        return 0;
    }
    let mut len = 0usize;
    let mut udecoded = utf8decodebyte(c[0], &mut len);
    if !between(len as i32, 1, UTF_SIZ as i32) {
        return 1;
    }
    let (mut i, mut j) = (1usize, 1usize);
    while i < c.len() && j < len {
        let mut typ = 0usize;
        let byte = utf8decodebyte(c[i], &mut typ);
        udecoded = (udecoded << 6) | byte;
        if typ != 0 {
            return j;
        }
        i += 1;
        j += 1;
    }
    if j < len {
        return 0;
    }
    *u = udecoded;
    utf8validate(u, len);
    len
}

pub fn utf8encode(u: Rune, c: &mut [u8; UTF_SIZ]) -> usize {
    let mut u = u;
    let len = utf8validate(&mut u, 0);
    if len > UTF_SIZ {
        return 0;
    }
    for i in (1..len).rev() {
        c[i] = utf8encodebyte(u, 0);
        u >>= 6;
    }
    c[0] = utf8encodebyte(u, len);
    len
}

fn base64dec_digit(c: u8) -> i32 {
    match c {
        b'+' => 62,
        b'/' => 63,
        b'0'..=b'9' => 52 + (c - b'0') as i32,
        b'A'..=b'Z' => (c - b'A') as i32,
        b'a'..=b'z' => 26 + (c - b'a') as i32,
        b'=' => -1,
        _ => 0,
    }
}

fn base64dec_getc(src: &[u8], pos: &mut usize) -> u8 {
    while *pos < src.len() && unsafe { libc::isprint(src[*pos] as i32) } == 0 {
        *pos += 1;
    }
    if *pos < src.len() {
        *pos += 1;
        src[*pos - 1]
    } else {
        b'=' // emulate padding if string ends
    }
}

pub fn base64dec(src: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < src.len() {
        let a = base64dec_digit(base64dec_getc(src, &mut pos));
        let b = base64dec_digit(base64dec_getc(src, &mut pos));
        let c = base64dec_digit(base64dec_getc(src, &mut pos));
        let d = base64dec_digit(base64dec_getc(src, &mut pos));

        // invalid input: 'a' can be -1, e.g. if src is "\n" (c-str)
        if a == -1 || b == -1 {
            break;
        }

        out.push(((a << 2) | ((b & 0x30) >> 4)) as u8);
        if c == -1 {
            break;
        }
        out.push((((b & 0x0f) << 4) | ((c & 0x3c) >> 2)) as u8);
        if d == -1 {
            break;
        }
        out.push((((c & 0x03) << 6) | d) as u8);
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

impl App {
    pub fn selinit(&mut self) {
        self.sel.mode = SEL_IDLE;
        self.sel.snap = 0;
        self.sel.ob.x = -1;
    }

    /// The line displayed at screen row `y`, honoring the scrollback view.
    pub fn tline(&self, y: i32) -> &Line {
        let scr = self.term.scr as isize;
        let y = y as isize;
        if y < scr {
            let idx = self.term.hist.len() as isize - scr + y;
            &self.term.hist[idx as usize]
        } else {
            &self.term.line[(y - scr) as usize]
        }
    }

    fn tline_mut(&mut self, y: i32) -> &mut Line {
        let scr = self.term.scr as isize;
        let y = y as isize;
        if y < scr {
            let idx = self.term.hist.len() as isize - scr + y;
            &mut self.term.hist[idx as usize]
        } else {
            &mut self.term.line[(y - scr) as usize]
        }
    }

    pub fn tlinelen(&self, y: i32) -> i32 {
        let line = self.tline(y);
        let mut i = self.term.col;
        if line_is_wrapped(line) {
            return i;
        }
        while i > 0 && line[(i - 1) as usize].u == b' ' as Rune {
            i -= 1;
        }
        i
    }

    pub fn selstart(&mut self, col: i32, row: i32, snap: i32) {
        self.selclear();
        self.sel.mode = SEL_EMPTY;
        self.sel.type_ = SEL_REGULAR;
        self.sel.alt = self.is_set(MODE_ALTSCREEN) as i32;
        self.sel.snap = snap;
        self.sel.oe.x = col;
        self.sel.ob.x = col;
        self.sel.oe.y = row;
        self.sel.ob.y = row;
        self.selnormalize();

        if self.sel.snap != 0 {
            self.sel.mode = SEL_READY;
        }
        self.tsetdirt(self.sel.nb.y, self.sel.ne.y);
    }

    pub fn selextend(&mut self, col: i32, row: i32, type_: i32, done: bool) {
        if self.sel.mode == SEL_IDLE {
            return;
        }
        if done && self.sel.mode == SEL_EMPTY {
            self.selclear();
            return;
        }

        let oldey = self.sel.oe.y;
        let oldex = self.sel.oe.x;
        let oldsby = self.sel.nb.y;
        let oldsey = self.sel.ne.y;
        let oldtype = self.sel.type_;

        self.sel.oe.x = col;
        self.sel.oe.y = row;
        self.selnormalize();
        self.sel.type_ = type_;

        if oldey != self.sel.oe.y
            || oldex != self.sel.oe.x
            || oldtype != self.sel.type_
            || self.sel.mode == SEL_EMPTY
        {
            self.tsetdirt(min(self.sel.nb.y, oldsby), max(self.sel.ne.y, oldsey));
        }

        self.sel.mode = if done { SEL_IDLE } else { SEL_READY };
    }

    pub fn selnormalize(&mut self) {
        if self.sel.type_ == SEL_REGULAR && self.sel.ob.y != self.sel.oe.y {
            self.sel.nb.x = if self.sel.ob.y < self.sel.oe.y {
                self.sel.ob.x
            } else {
                self.sel.oe.x
            };
            self.sel.ne.x = if self.sel.ob.y < self.sel.oe.y {
                self.sel.oe.x
            } else {
                self.sel.ob.x
            };
        } else {
            self.sel.nb.x = min(self.sel.ob.x, self.sel.oe.x);
            self.sel.ne.x = max(self.sel.ob.x, self.sel.oe.x);
        }
        self.sel.nb.y = min(self.sel.ob.y, self.sel.oe.y);
        self.sel.ne.y = max(self.sel.ob.y, self.sel.oe.y);

        let mut nbx = self.sel.nb.x;
        let mut nby = self.sel.nb.y;
        self.selsnap(&mut nbx, &mut nby, -1);
        self.sel.nb.x = nbx;
        self.sel.nb.y = nby;

        let mut nex = self.sel.ne.x;
        let mut ney = self.sel.ne.y;
        self.selsnap(&mut nex, &mut ney, 1);
        self.sel.ne.x = nex;
        self.sel.ne.y = ney;

        // expand selection over line breaks
        if self.sel.type_ == SEL_RECTANGULAR {
            return;
        }
        let i = self.tlinelen(self.sel.nb.y);
        if i < self.sel.nb.x {
            self.sel.nb.x = i;
        }
        if self.tlinelen(self.sel.ne.y) <= self.sel.ne.x {
            self.sel.ne.x = self.term.col - 1;
        }
    }

    pub fn selected(&self, x: i32, y: i32) -> bool {
        if self.sel.mode == SEL_EMPTY
            || self.sel.ob.x == -1
            || self.sel.alt != self.is_set(MODE_ALTSCREEN) as i32
        {
            return false;
        }

        if self.sel.type_ == SEL_RECTANGULAR {
            return between(y, self.sel.nb.y, self.sel.ne.y)
                && between(x, self.sel.nb.x, self.sel.ne.x);
        }

        between(y, self.sel.nb.y, self.sel.ne.y)
            && (y != self.sel.nb.y || x >= self.sel.nb.x)
            && (y != self.sel.ne.y || x <= self.sel.ne.x)
    }

    fn selsnap(&mut self, x: &mut i32, y: &mut i32, direction: i32) {
        match self.sel.snap {
            SNAP_WORD => {
                let mut prevgp = self.tline(*y)[*x as usize];
                let mut prevdelim = is_delim(prevgp.u);
                loop {
                    let mut newx = *x + direction;
                    let mut newy = *y;
                    if !between(newx, 0, self.term.col - 1) {
                        newy += direction;
                        newx = (newx + self.term.col) % self.term.col;
                        if !between(newy, 0, self.term.row - 1) {
                            break;
                        }

                        let (yt, xt) = if direction > 0 {
                            (*y, *x)
                        } else {
                            (newy, newx)
                        };
                        if self.tline(yt)[xt as usize].mode & ATTR_WRAP == 0 {
                            break;
                        }
                    }

                    if newx >= self.tlinelen(newy) {
                        break;
                    }

                    let gp = self.tline(newy)[newx as usize];
                    let delim = is_delim(gp.u);
                    if gp.mode & ATTR_WDUMMY == 0
                        && (delim != prevdelim || (delim && gp.u != prevgp.u))
                    {
                        break;
                    }

                    *x = newx;
                    *y = newy;
                    prevgp = gp;
                    prevdelim = delim;
                }
            }
            SNAP_LINE => {
                *x = if direction < 0 { 0 } else { self.term.col - 1 };
                if direction < 0 {
                    while *y > 0 {
                        if self.tline(*y - 1)[(self.term.col - 1) as usize].mode & ATTR_WRAP == 0 {
                            break;
                        }
                        *y += direction;
                    }
                } else if direction > 0 {
                    while *y < self.term.row - 1 {
                        if self.tline(*y)[(self.term.col - 1) as usize].mode & ATTR_WRAP == 0 {
                            break;
                        }
                        *y += direction;
                    }
                }
            }
            _ => {}
        }
    }

    pub fn getsel(&self) -> Option<Vec<u8>> {
        if self.sel.ob.x == -1 {
            return None;
        }
        if self.term.row < 1 || self.term.col < 1 {
            return None;
        }

        // A selection may outlive the screen it was made on (for example when
        // the window is shrunk).  Clamp it to the visible screen instead of
        // indexing the retired rows; the C original reads out of bounds here.
        let nb_x = limit(self.sel.nb.x, 0, self.term.col - 1);
        let ne_x = limit(self.sel.ne.x, 0, self.term.col - 1);
        let nb_y = limit(self.sel.nb.y, 0, self.term.row - 1);
        let ne_y = limit(self.sel.ne.y, 0, self.term.row - 1);
        if self.sel.ne.y < 0 || self.sel.nb.y > self.term.row - 1 {
            return None;
        }

        let mut out: Vec<u8> = Vec::new();
        let mut buf = [0u8; UTF_SIZ];

        for y in nb_y..=ne_y {
            let linelen = self.tlinelen(y);
            if linelen == 0 {
                out.push(b'\n');
                continue;
            }

            let (gp, lastx) = if self.sel.type_ == SEL_RECTANGULAR {
                (nb_x, ne_x)
            } else {
                (
                    if nb_y == y { nb_x } else { 0 },
                    if ne_y == y { ne_x } else { self.term.col - 1 },
                )
            };
            let mut last = min(lastx, linelen - 1);
            let line = self.tline(y);
            while last >= gp && line[last as usize].u == b' ' as Rune {
                last -= 1;
            }

            let mut i = gp;
            while i <= last {
                let glyph = line[i as usize];
                if glyph.mode & ATTR_WDUMMY == 0 {
                    let n = utf8encode(glyph.u, &mut buf);
                    out.extend_from_slice(&buf[..n]);
                }
                i += 1;
            }

            if (y < ne_y || lastx >= linelen)
                && (line[last.max(0) as usize].mode & ATTR_WRAP == 0
                    || self.sel.type_ == SEL_RECTANGULAR)
            {
                out.push(b'\n');
            }
        }
        Some(out)
    }

    pub fn selclear(&mut self) {
        if self.sel.ob.x == -1 {
            return;
        }
        self.sel.mode = SEL_IDLE;
        self.sel.ob.x = -1;
        self.tsetdirt(self.sel.nb.y, self.sel.ne.y);
    }
}

// ---------------------------------------------------------------------------
// URLs: highlight with Ctrl held, open the one clicked
// ---------------------------------------------------------------------------

/// First position at or after `from` where one of the configured URL prefixes
/// starts.  Unlike the C patch this looks at every prefix and returns the
/// leftmost match, so `https://a ... http://b` finds `a` first.
fn findurlprefix(buf: &[u8], from: usize) -> Option<usize> {
    let mut best: Option<usize> = None;
    for prefix in config::URLPREFIXES {
        if prefix.is_empty() || from >= buf.len() {
            continue;
        }
        if let Some(off) = buf[from..].windows(prefix.len()).position(|w| w == *prefix) {
            let at = from + off;
            best = Some(match best {
                Some(b) => b.min(at),
                None => at,
            });
        }
    }
    best
}

/// Most hyperlinks kept at once; when the table fills up, links no longer on
/// screen or in the history are dropped.
pub const LINKMAX: usize = 4096;

/// The path of an OSC 7 `file://host/path` report, `%xx`-decoded.
pub fn osc7_path(uri: &[u8]) -> Option<Vec<u8>> {
    let rest = uri.strip_prefix(b"file://")?;
    let slash = rest.iter().position(|&b| b == b'/')?;
    let mut out = Vec::new();
    let enc = &rest[slash..];
    let mut i = 0;
    while i < enc.len() {
        if enc[i] == b'%' && i + 2 < enc.len() {
            if let Some(v) = hexdecode(&enc[i + 1..i + 3]) {
                out.push(v[0]);
                i += 3;
                continue;
            }
        }
        out.push(enc[i]);
        i += 1;
    }
    if out.contains(&0) {
        return None;
    }
    Some(out)
}

impl App {
    /// Index in `links` for the OSC 8 link (`params`, `uri`); 0 ends the
    /// current link.  Links with the same id and URI share an entry.
    pub fn linkid(&mut self, params: &[u8], uri: &[u8]) -> u32 {
        if uri.is_empty() {
            return 0;
        }
        if self.links.is_empty() {
            self.links.push(Vec::new()); // index 0: no link
        }
        let id = params
            .split(|&b| b == b':')
            .find_map(|p| p.strip_prefix(b"id="))
            .unwrap_or(&[])
            .to_vec();
        let key = (id, uri.to_vec());
        if let Some(&i) = self.linkids.get(&key) {
            return i;
        }
        if self.links.len() >= LINKMAX {
            self.compactlinks();
            if self.links.len() >= LINKMAX {
                return 0;
            }
        }
        let i = self.links.len() as u32;
        self.links.push(uri.to_vec());
        self.linkids.insert(key, i);
        i
    }

    /// Drop the links no cell refers to any more and renumber the rest.
    pub fn compactlinks(&mut self) {
        let n = self.links.len();
        let mut used = vec![false; n];
        used[0] = true;
        let mark = |used: &mut Vec<bool>, g: &Glyph| {
            if (g.link as usize) < used.len() {
                used[g.link as usize] = true;
            }
        };
        for line in self.term.line.iter().chain(self.term.alt.iter()).chain(self.term.hist.iter()) {
            for g in line {
                mark(&mut used, g);
            }
        }
        mark(&mut used, &self.term.c.attr);
        for c in &self.saved_cursors {
            mark(&mut used, &c.attr);
        }

        let mut remap = vec![0u32; n];
        let mut links = Vec::new();
        for (i, l) in std::mem::take(&mut self.links).into_iter().enumerate() {
            if used[i] {
                remap[i] = links.len() as u32;
                links.push(l);
            }
        }
        let fix = |g: &mut Glyph| {
            g.link = remap.get(g.link as usize).copied().unwrap_or(0);
        };
        for line in self
            .term
            .line
            .iter_mut()
            .chain(self.term.alt.iter_mut())
            .chain(self.term.hist.iter_mut())
        {
            line.iter_mut().for_each(fix);
        }
        fix(&mut self.term.c.attr);
        for c in self.saved_cursors.iter_mut() {
            fix(&mut c.attr);
        }
        self.linkids = std::mem::take(&mut self.linkids)
            .into_iter()
            .filter_map(|(k, v)| {
                let nv = remap[v as usize];
                (nv != 0).then_some((k, nv))
            })
            .collect();
        self.links = links;
    }
}

impl App {
    /// One byte per screen column for `row`: ASCII cells are copied verbatim,
    /// anything else (wide-character halves, CJK, emoji, empty cells) becomes
    /// NUL, which also ends a URL.  One byte per column keeps buffer offsets
    /// equal to column numbers.
    fn rowbytes(&self, row: i32) -> Vec<u8> {
        let col = self.term.col as usize;
        let mut buf = vec![0u8; col + 1];
        for (j, g) in self.tline(row).iter().take(col).enumerate() {
            if g.u > 0 && g.u < 127 {
                buf[j] = g.u as u8;
            }
        }
        buf
    }

    /// Find the URLs in a row buffer as `[start, end)` column ranges.
    fn urlspans_in(&self, buf: &[u8]) -> Vec<(i32, i32)> {
        let col = self.term.col as usize;
        let mut spans = Vec::new();
        let mut from = 0usize;
        while from < col {
            let start = match findurlprefix(buf, from) {
                Some(s) => s,
                None => break,
            };
            let mut end = start;
            while end < col && config::URLCHARS.contains(&buf[end]) {
                end += 1;
            }
            spans.push((start as i32, end as i32));
            from = end.max(start + 1);
        }
        spans
    }

    /// Column ranges (`[start, end)`) of every URL on `row`, left to right.
    pub fn urlspans(&self, row: i32) -> Vec<(i32, i32)> {
        if row < 0 || row >= self.term.row {
            return Vec::new();
        }
        self.urlspans_in(&self.rowbytes(row))
    }

    /// The OSC 8 link index of the cell at (row, col); the right half of a
    /// wide character belongs to its left half.
    fn link_at(&self, row: i32, col: i32) -> u32 {
        let line = self.tline(row);
        let mut c = col as usize;
        if line[c].mode & ATTR_WDUMMY != 0 && c > 0 {
            c -= 1;
        }
        line[c].link
    }

    /// The URL covering column `col` on `row`, if any: an OSC 8 hyperlink
    /// first, then a URL recognised in the text.
    pub fn url_at(&self, row: i32, col: i32) -> Option<Vec<u8>> {
        if row < 0 || row >= self.term.row || col < 0 || col >= self.term.col {
            return None;
        }
        let link = self.link_at(row, col);
        if link != 0 {
            return self.links.get(link as usize).cloned();
        }
        let buf = self.rowbytes(row);
        for (start, end) in self.urlspans_in(&buf) {
            if start <= col && col < end {
                return Some(buf[start as usize..end as usize].to_vec());
            }
        }
        None
    }

    /// Underline every URL on screen.  Called while Ctrl is held; cells that
    /// are already highlighted are left alone, so key auto-repeat does not
    /// keep dirtying the screen.
    pub fn highlighturls(&mut self) {
        for row in 0..self.term.row {
            let mut spans = self.urlspans(row);
            // OSC 8 hyperlinks: every cell that carries a link.
            for c in 0..self.term.col {
                if self.link_at(row, c) != 0 {
                    spans.push((c, c + 1));
                }
            }
            if spans.is_empty() {
                continue;
            }
            let mut changed = false;
            {
                let line = self.tline_mut(row);
                for (start, end) in spans {
                    for c in start..end {
                        let g = &mut line[c as usize];
                        if g.mode & ATTR_URL == 0 {
                            g.mode |= ATTR_URL;
                            changed = true;
                        }
                    }
                }
            }
            if changed {
                self.tsetdirt(row, row);
            }
        }
    }

    /// Remove the underlines added by [`App::highlighturls`].
    pub fn unhighlighturls(&mut self) {
        let mut changed = false;
        // Clear both the live screen and the history: the view may have moved
        // while Ctrl was held, leaving underlines on rows that are no longer
        // visible.
        for line in self.term.line.iter_mut().chain(self.term.hist.iter_mut()) {
            for g in line.iter_mut() {
                if g.mode & ATTR_URL != 0 {
                    g.mode &= !ATTR_URL;
                    changed = true;
                }
            }
        }
        if changed {
            self.tfulldirt();
        }
    }

    /// Open the URL under (row, col) with `config::URLHANDLER`.
    pub fn followurl(&mut self, row: i32, col: i32) {
        let url = match self.url_at(row, col) {
            // Never let a link pose as an option to the handler.
            Some(u) if !u.is_empty() && u[0] != b'-' => u,
            _ => return,
        };

        // Double fork, as in the C patch: the intermediate child is reaped
        // here while the handler is re-parented to init, so no zombie.
        unsafe {
            let handler = match CString::new(config::URLHANDLER) {
                Ok(h) => h,
                Err(_) => return,
            };
            let arg = match CString::new(url) {
                Ok(a) => a,
                Err(_) => return,
            };
            let pid = libc::fork();
            if pid == 0 {
                if libc::fork() == 0 {
                    libc::execlp(
                        handler.as_ptr(),
                        handler.as_ptr(),
                        arg.as_ptr(),
                        ptr::null::<libc::c_char>(),
                    );
                }
                libc::_exit(1);
            } else if pid > 0 {
                let mut status: libc::c_int = 0;
                libc::waitpid(pid, &mut status, 0);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shell / pty
// ---------------------------------------------------------------------------

impl App {
    /// Replace the process with the configured shell/command.  Never returns.
    fn execsh(&mut self, cmd: &str, args: Option<&[CString]>) -> ! {
        unsafe {
            *libc::__errno_location() = 0;
            let pw = libc::getpwuid(libc::getuid());
            if pw.is_null() {
                if read_errno() != 0 {
                    die(&format!("getpwuid: {}\n", last_errno_message()));
                } else {
                    die("who are you?\n");
                }
            }
            let pw_shell = cstr_to_string((*pw).pw_shell);
            let pw_name = cstr_to_string((*pw).pw_name);
            let pw_dir = cstr_to_string((*pw).pw_dir);

            let sh = match std::env::var_os("SHELL") {
                Some(s) => s.to_string_lossy().into_owned(),
                None => {
                    if !pw_shell.is_empty() {
                        pw_shell
                    } else {
                        cmd.to_string()
                    }
                }
            };

            let argv: Vec<CString> = if let Some(a) = args {
                if a.is_empty() {
                    vec![cstring(sh.as_bytes())]
                } else {
                    a.to_vec()
                }
            } else if let Some(scroll) = config::SCROLL {
                let arg = config::UTMP.unwrap_or(&sh);
                vec![cstring(scroll.as_bytes()), cstring(arg.as_bytes())]
            } else if let Some(utmp) = config::UTMP {
                vec![cstring(utmp.as_bytes())]
            } else {
                vec![cstring(sh.as_bytes())]
            };

            for v in ["COLUMNS", "LINES", "TERMCAP"] {
                libc::unsetenv(cstring(v.as_bytes()).as_ptr());
            }
            set_env("LOGNAME", &pw_name);
            set_env("USER", &pw_name);
            set_env("SHELL", &sh);
            set_env("HOME", &pw_dir);
            set_env("TERM", config::TERMNAME);

            for sig in [
                libc::SIGCHLD,
                libc::SIGHUP,
                libc::SIGINT,
                libc::SIGQUIT,
                libc::SIGTERM,
                libc::SIGALRM,
            ] {
                libc::signal(sig, libc::SIG_DFL);
            }

            let mut ptrs: Vec<*const libc::c_char> = argv.iter().map(|a| a.as_ptr()).collect();
            ptrs.push(ptr::null());
            libc::execvp(argv[0].as_ptr(), ptrs.as_ptr());
            libc::_exit(1);
        }
    }

    fn stty(&mut self, args: &[CString]) {
        let limit = 4096usize; // _POSIX_ARG_MAX
        let mut cmd: Vec<u8> = config::STTY_ARGS.as_bytes().to_vec();
        if cmd.len() > limit - 1 {
            die("incorrect stty parameters\n");
        }
        let mut siz = limit - cmd.len();
        for a in args {
            let s = a.as_bytes();
            if s.len() > siz - 1 {
                die("stty parameter length too long\n");
            }
            cmd.push(b' ');
            cmd.extend_from_slice(s);
            siz -= s.len() + 1;
        }
        let c = cstring(&cmd);
        let rc = unsafe { libc::system(c.as_ptr()) };
        if rc != 0 {
            let e = read_errno();
            eprintln!("Couldn't call stty: {}", errno_message(e));
        }
    }

    /// Create the pty (or attach to a line) and spawn the shell.
    pub fn ttynew(
        &mut self,
        line: Option<&CString>,
        cmd: &str,
        out: Option<&CString>,
        args: Option<&[CString]>,
    ) -> i32 {
        if let Some(out) = out {
            self.term.mode |= MODE_PRINT;
            self.iofd = if out.as_bytes() == b"-" {
                1
            } else {
                unsafe { libc::open(out.as_ptr(), libc::O_WRONLY | libc::O_CREAT, 0o666) }
            };
            if self.iofd < 0 {
                let e = read_errno();
                eprintln!(
                    "Error opening {}:{}",
                    out.to_string_lossy(),
                    errno_message(e)
                );
            }
        }

        if let Some(line) = line {
            let fd = unsafe { libc::open(line.as_ptr(), libc::O_RDWR) };
            if fd < 0 {
                die(&format!(
                    "open line '{}' failed: {}\n",
                    line.to_string_lossy(),
                    last_errno_message()
                ));
            }
            unsafe { libc::dup2(fd, 0) };
            self.stty(args.unwrap_or(&[]));
            CMDFD.store(fd, Ordering::Relaxed);
            return fd;
        }

        // openpty() equivalent using the portable pty API
        let m = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
        if m < 0 {
            die(&format!("openpty failed: {}\n", last_errno_message()));
        }
        if unsafe { libc::grantpt(m) } < 0 || unsafe { libc::unlockpt(m) } < 0 {
            die(&format!("openpty failed: {}\n", last_errno_message()));
        }
        let mut name = [0i8; 256];
        if unsafe { libc::ptsname_r(m, name.as_mut_ptr(), name.len()) } != 0 {
            die(&format!("ptsname failed: {}\n", last_errno_message()));
        }
        let s = unsafe { libc::open(name.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
        if s < 0 {
            die(&format!("openpty failed: {}\n", last_errno_message()));
        }

        let pid = unsafe { libc::fork() };
        if pid < 0 {
            die(&format!("fork failed: {}\n", last_errno_message()));
        }

        if pid == 0 {
            // child
            unsafe {
                libc::close(self.iofd);
                libc::close(m);
                libc::setsid();
                libc::dup2(s, 0);
                libc::dup2(s, 1);
                libc::dup2(s, 2);
                if libc::ioctl(s, libc::TIOCSCTTY as libc::c_ulong, 0) < 0 {
                    die(&format!(
                        "ioctl TIOCSCTTY failed: {}\n",
                        last_errno_message()
                    ));
                }
                if s > 2 {
                    libc::close(s);
                }
            }
            self.execsh(cmd, args);
        }

        // parent
        PID.store(pid, Ordering::Relaxed);
        unsafe {
            libc::close(s);
            let handler: extern "C" fn(libc::c_int) = sigchld_handler;
            libc::signal(libc::SIGCHLD, handler as libc::sighandler_t);
        }
        CMDFD.store(m, Ordering::Relaxed);
        m
    }

    pub fn ttyread(&mut self) -> usize {
        let fd = CMDFD.load(Ordering::Relaxed);
        let cap = self.ttybuf.len();
        let ret = unsafe {
            libc::read(
                fd,
                self.ttybuf.as_mut_ptr().add(self.ttybuflen) as *mut libc::c_void,
                cap - self.ttybuflen,
            )
        };

        match ret {
            0 => std::process::exit(0),
            r if r < 0 => die(&format!(
                "couldn't read from shell: {}\n",
                last_errno_message()
            )),
            r => {
                self.ttybuflen += r as usize;
                let data = self.ttybuf[..self.ttybuflen].to_vec();
                let written = self.twrite(&data, self.ttybuflen as i32, false) as usize;
                self.ttybuflen -= written;
                if self.ttybuflen > 0 {
                    self.ttybuf
                        .copy_within(written..written + self.ttybuflen, 0);
                }
                r as usize
            }
        }
    }

    pub fn ttywrite(&mut self, s: &[u8], may_echo: bool) {
        // Sending input returns to the live screen, like other terminals.
        if self.term.scr != 0 {
            self.kscrolldown(&Arg::i(i32::MAX));
        }

        if may_echo && self.is_set(MODE_ECHO) {
            self.twrite(s, s.len() as i32, true);
        }

        if !self.is_set(MODE_CRLF) {
            self.ttywriteraw(s);
            return;
        }

        // similar to how the kernel handles ONLCR for ttys
        let mut s = s;
        while !s.is_empty() {
            if s[0] == b'\r' {
                self.ttywriteraw(b"\r\n");
                s = &s[1..];
            } else {
                let next = s.iter().position(|&b| b == b'\r').unwrap_or(s.len());
                self.ttywriteraw(&s[..next]);
                s = &s[next..];
            }
        }
    }

    fn ttywriteraw(&mut self, s: &[u8]) {
        #[cfg(test)]
        if CMDFD.load(Ordering::Relaxed) < 0 {
            self.test_out.extend_from_slice(s);
            return;
        }
        let mut s = s;
        let mut lim: usize = 256;

        while !s.is_empty() {
            let fd = CMDFD.load(Ordering::Relaxed);
            let mut wfd: libc::fd_set = unsafe { std::mem::zeroed() };
            let mut rfd: libc::fd_set = unsafe { std::mem::zeroed() };
            unsafe {
                libc::FD_ZERO(&mut wfd);
                libc::FD_ZERO(&mut rfd);
                libc::FD_SET(fd, &mut wfd);
                libc::FD_SET(fd, &mut rfd);

                let rc = libc::pselect(
                    fd + 1,
                    &mut rfd,
                    &mut wfd,
                    ptr::null_mut(),
                    ptr::null(),
                    ptr::null(),
                );
                if rc < 0 {
                    if read_errno() == libc::EINTR {
                        continue;
                    }
                    die(&format!("select failed: {}\n", last_errno_message()));
                }

                if libc::FD_ISSET(fd, &wfd) {
                    let towrite = if s.len() < lim { s.len() } else { lim };
                    let r = libc::write(fd, s.as_ptr() as *const libc::c_void, towrite);
                    if r < 0 {
                        die(&format!("write error on tty: {}\n", last_errno_message()));
                    }
                    let r = r as usize;
                    if r < s.len() {
                        if s.len() < lim {
                            lim = self.ttyread();
                        }
                        s = &s[r..];
                    } else {
                        break;
                    }
                }
                if libc::FD_ISSET(fd, &rfd) {
                    lim = self.ttyread();
                }
            }
        }
    }

    pub fn ttyresize(&mut self, tw: i32, th: i32) {
        let w = libc::winsize {
            ws_row: self.term.row as u16,
            ws_col: self.term.col as u16,
            ws_xpixel: tw as u16,
            ws_ypixel: th as u16,
        };
        let rc = unsafe {
            libc::ioctl(
                CMDFD.load(Ordering::Relaxed),
                libc::TIOCSWINSZ as libc::c_ulong,
                &w,
            )
        };
        if rc < 0 {
            eprintln!("Couldn't set window size: {}", last_errno_message());
        }
    }

    pub fn ttyhangup(&mut self) {
        unsafe {
            libc::kill(PID.load(Ordering::Relaxed), libc::SIGHUP);
        }
    }
}

fn set_env(key: &str, val: &str) {
    let k = cstring(key.as_bytes());
    let v = cstring(val.as_bytes());
    unsafe {
        libc::setenv(k.as_ptr(), v.as_ptr(), 1);
    }
}

extern "C" fn sigchld_handler(_sig: libc::c_int) {
    unsafe {
        let mut stat: libc::c_int = 0;
        let pid = PID.load(Ordering::Relaxed);
        let p = libc::waitpid(pid, &mut stat, libc::WNOHANG);
        if p < 0 {
            libc::write(2, b"waiting for child failed\n".as_ptr() as *const _, 27);
            libc::_exit(1);
        }
        if pid != p {
            return;
        }
        if (libc::WIFEXITED(stat) && libc::WEXITSTATUS(stat) != 0) || libc::WIFSIGNALED(stat) {
            libc::_exit(1);
        }
        libc::_exit(0);
    }
}

// ---------------------------------------------------------------------------
// Soft-wrapped lines
// ---------------------------------------------------------------------------

/// Whether a screen row continues on the next one.  The mark normally sits
/// on the last cell, but `tputc` puts it on the double-width cell when a wide
/// character ends the row, so both places are checked.
pub fn line_is_wrapped(line: &Line) -> bool {
    let n = line.len();
    if n == 0 {
        return false;
    }
    if line[n - 1].mode & ATTR_WRAP != 0 {
        return true;
    }
    n >= 2 && line[n - 1].mode & ATTR_WDUMMY != 0 && line[n - 2].mode & ATTR_WRAP != 0
}

/// Number of cells of `line` that carry text.  Trailing blanks are dropped so
/// that re-wrapping a line does not drag padding along with it; a row that
/// continues on the next one is counted in full.
fn line_text_len(line: &Line) -> usize {
    if line_is_wrapped(line) {
        return line.len();
    }
    let mut i = line.len();
    while i > 0 {
        let g = &line[i - 1];
        if g.mode & (ATTR_WIDE | ATTR_WDUMMY) != 0 || (g.u != b' ' as Rune && g.u != 0) {
            break;
        }
        i -= 1;
    }
    i
}

/// Append a full row that continues on the next one, marking the soft-wrap.
fn push_continued(rows: &mut Vec<Line>, mut line: Line) {
    let n = line.len();
    if n > 0 {
        let i = if n >= 2 && line[n - 1].mode & ATTR_WDUMMY != 0 {
            n - 2
        } else {
            n - 1
        };
        line[i].mode |= ATTR_WRAP;
    }
    rows.push(line);
}

// ---------------------------------------------------------------------------
// Screen / dirty tracking
// ---------------------------------------------------------------------------

impl App {
    pub fn tattrset(&self, attr: u16) -> bool {
        for i in 0..self.term.row - 1 {
            for j in 0..self.term.col - 1 {
                if self.term.line[i as usize][j as usize].mode & attr != 0 {
                    return true;
                }
            }
        }
        false
    }

    pub fn tsetdirt(&mut self, top: i32, bot: i32) {
        let top = limit(top, 0, self.term.row - 1);
        let bot = limit(bot, 0, self.term.row - 1);
        for i in top..=bot {
            self.term.dirty[i as usize] = true;
        }
    }

    pub fn tsetdirtattr(&mut self, attr: u16) {
        for i in 0..self.term.row - 1 {
            for j in 0..self.term.col - 1 {
                if self.term.line[i as usize][j as usize].mode & attr != 0 {
                    self.tsetdirt(i, i);
                    break;
                }
            }
        }
    }

    pub fn tfulldirt(&mut self) {
        self.tsetdirt(0, self.term.row - 1);
    }

    pub fn tcursor(&mut self, mode: CursorMovement) {
        let alt = self.is_set(MODE_ALTSCREEN) as usize;
        match mode {
            CursorMovement::Save => {
                self.saved_cursors[alt] = self.term.c;
            }
            CursorMovement::Load => {
                self.term.c = self.saved_cursors[alt];
                self.tmoveto(self.saved_cursors[alt].x, self.saved_cursors[alt].y);
            }
        }
    }

    pub fn treset(&mut self) {
        self.term.c = TCursor::new(DEFAULTFG, DEFAULTBG);
        self.term.c.attr.mode = ATTR_NULL;
        self.term.c.state = CURSOR_DEFAULT;

        for t in self.term.tabs.iter_mut() {
            *t = false;
        }
        let mut i = TABSPACES as i32;
        while i < self.term.col {
            self.term.tabs[i as usize] = true;
            i += TABSPACES as i32;
        }
        self.term.top = 0;
        self.term.bot = self.term.row - 1;
        self.term.mode = MODE_WRAP | MODE_UTF8;
        self.term.trantbl = [CS_USA; 4];
        self.term.charset = 0;

        for _ in 0..2 {
            self.tmoveto(0, 0);
            self.tcursor(CursorMovement::Save);
            self.tclearregion(0, 0, self.term.col - 1, self.term.row - 1);
            self.tswapscreen();
        }
    }

    pub fn tnew(&mut self, col: i32, row: i32) {
        self.term = Term::new(DEFAULTFG, DEFAULTBG);
        self.tresize(col, row);
        self.treset();
    }

    pub fn tswapscreen(&mut self) {
        std::mem::swap(&mut self.term.line, &mut self.term.alt);
        self.term.mode ^= MODE_ALTSCREEN;
        self.term.scr = 0;
        self.tfulldirt();
    }

    /// Append a line that just left the top of the main screen.
    fn histpush(&mut self, line: Line) {
        if config::SCROLLBACK == 0 {
            return;
        }
        self.term.hist.push_back(line);
        while self.term.hist.len() > config::SCROLLBACK {
            self.term.hist.pop_front();
            self.term.histdrop += 1;
        }
        // Keep the displayed history stable while new output arrives.
        if self.term.scr > 0 {
            self.term.scr = min(self.term.scr + 1, self.term.hist.len() as i32);
        }
    }

    /// Scroll the viewport up (towards older lines).  `Arg::i(-1)` scrolls a
    /// page; a large positive value goes to the oldest line.
    ///
    /// The main screen's history is shown above the alternate screen too, as
    /// in the upstream scrollback patch, so the key is never a silent no-op
    /// inside a full-screen program.
    pub fn kscrollup(&mut self, arg: &Arg) {
        let mut n = match arg.0 {
            ArgVal::I(n) => n,
            _ => 1,
        };
        if n < 0 {
            n += self.term.row;
        }
        let room = self.term.hist.len() as i32 - self.term.scr;
        let n = limit(n, 0, room.max(0));
        if n > 0 {
            self.term.scr += n;
            self.selscroll(0, n);
            self.tfulldirt();
        }
    }

    /// Scroll the viewport down (towards the live screen).
    pub fn kscrolldown(&mut self, arg: &Arg) {
        let mut n = match arg.0 {
            ArgVal::I(n) => n,
            _ => 1,
        };
        if n < 0 {
            n += self.term.row;
        }
        let n = limit(n, 0, self.term.scr);
        if n > 0 {
            self.term.scr -= n;
            self.selscroll(0, -n);
            self.tfulldirt();
        }
    }

    pub fn tscrolldown(&mut self, orig: i32, n: i32) {
        let n = limit(n, 0, self.term.bot - orig + 1);
        if n == 0 {
            return;
        }
        self.tsetdirt(orig, self.term.bot - n);
        self.tclearregion(0, self.term.bot - n + 1, self.term.col - 1, self.term.bot);

        for i in (orig + n..=self.term.bot).rev() {
            self.term.line.swap(i as usize, (i - n) as usize);
        }

        if self.term.scr == 0 {
            self.selscroll(orig, n);
        }
    }

    pub fn tscrollup(&mut self, orig: i32, n: i32, record: bool) {
        let n = limit(n, 0, self.term.bot - orig + 1);
        if n == 0 {
            return;
        }

        if record
            && !self.is_set(MODE_ALTSCREEN)
            && orig == 0
            && self.term.top == 0
            && self.term.bot == self.term.row - 1
        {
            for i in 0..n {
                let line = self.term.line[i as usize].clone();
                self.histpush(line);
            }
        }

        self.tclearregion(0, orig, self.term.col - 1, orig + n - 1);
        self.tsetdirt(orig + n, self.term.bot);

        for i in orig..=self.term.bot - n {
            self.term.line.swap(i as usize, (i + n) as usize);
        }

        if self.term.scr == 0 {
            self.selscroll(orig, -n);
        }
    }

    pub fn selscroll(&mut self, orig: i32, n: i32) {
        if self.sel.ob.x == -1 || self.sel.alt != self.is_set(MODE_ALTSCREEN) as i32 {
            return;
        }

        if between(self.sel.nb.y, orig, self.term.bot)
            != between(self.sel.ne.y, orig, self.term.bot)
        {
            self.selclear();
        } else if between(self.sel.nb.y, orig, self.term.bot) {
            self.sel.ob.y += n;
            self.sel.oe.y += n;
            if self.sel.ob.y < self.term.top
                || self.sel.ob.y > self.term.bot
                || self.sel.oe.y < self.term.top
                || self.sel.oe.y > self.term.bot
            {
                self.selclear();
            } else {
                self.selnormalize();
            }
        }
    }

    pub fn tnewline(&mut self, first_col: bool) {
        let mut y = self.term.c.y;
        if y == self.term.bot {
            self.tscrollup(self.term.top, 1, true);
        } else {
            y += 1;
        }
        let x = if first_col { 0 } else { self.term.c.x };
        self.tmoveto(x, y);
    }

    pub fn tmoveato(&mut self, x: i32, y: i32) {
        let y = y + if self.term.c.state & CURSOR_ORIGIN != 0 {
            self.term.top
        } else {
            0
        };
        self.tmoveto(x, y);
    }

    pub fn tmoveto(&mut self, x: i32, y: i32) {
        let (miny, maxy) = if self.term.c.state & CURSOR_ORIGIN != 0 {
            (self.term.top, self.term.bot)
        } else {
            (0, self.term.row - 1)
        };
        self.term.c.state &= !CURSOR_WRAPNEXT;
        self.term.c.x = limit(x, 0, self.term.col - 1);
        self.term.c.y = limit(y, miny, maxy);
    }

    pub fn tsetchar(&mut self, u: Rune, attr: &Glyph, x: i32, y: i32) {
        // The table is proudly stolen from rxvt.
        let mut u = u;
        if self.term.trantbl[self.term.charset as usize] == CS_GRAPHIC0
            && between(u as i32, 0x41, 0x7e)
        {
            if let Some(s) = VT100_0[(u - 0x41) as usize] {
                utf8decode(s.as_bytes(), &mut u);
            }
        }

        if self.term.line[y as usize][x as usize].mode & ATTR_WIDE != 0 {
            if x + 1 < self.term.col {
                self.term.line[y as usize][(x + 1) as usize].u = b' ' as Rune;
                self.term.line[y as usize][(x + 1) as usize].mode &= !ATTR_WDUMMY;
            }
        } else if self.term.line[y as usize][x as usize].mode & ATTR_WDUMMY != 0 && x > 0 {
            self.term.line[y as usize][(x - 1) as usize].u = b' ' as Rune;
            self.term.line[y as usize][(x - 1) as usize].mode &= !ATTR_WIDE;
        }

        self.term.dirty[y as usize] = true;
        self.term.line[y as usize][x as usize] = *attr;
        self.term.line[y as usize][x as usize].u = u;
    }

    pub fn tclearregion(&mut self, x1: i32, y1: i32, x2: i32, y2: i32) {
        let (mut x1, mut x2) = (x1, x2);
        let (mut y1, mut y2) = (y1, y2);
        if x1 > x2 {
            std::mem::swap(&mut x1, &mut x2);
        }
        if y1 > y2 {
            std::mem::swap(&mut y1, &mut y2);
        }

        x1 = limit(x1, 0, self.term.col - 1);
        x2 = limit(x2, 0, self.term.col - 1);
        y1 = limit(y1, 0, self.term.row - 1);
        y2 = limit(y2, 0, self.term.row - 1);

        for y in y1..=y2 {
            self.term.dirty[y as usize] = true;
            for x in x1..=x2 {
                if self.term.scr == 0 && self.selected(x, y) {
                    self.selclear();
                }
                let gp = &mut self.term.line[y as usize][x as usize];
                gp.fg = self.term.c.attr.fg;
                gp.bg = self.term.c.attr.bg;
                gp.mode = 0;
                gp.ustyle = UL_NONE;
                gp.ucolor = 0;
                gp.link = 0;
                gp.u = b' ' as Rune;
            }
        }
    }

    pub fn tdeletechar(&mut self, n: i32) {
        let n = limit(n, 0, self.term.col - self.term.c.x);
        let dst = self.term.c.x;
        let src = self.term.c.x + n;
        let size = (self.term.col - src) as usize;
        let y = self.term.c.y as usize;
        self.term.line[y].copy_within(src as usize..src as usize + size, dst as usize);
        self.tclearregion(
            self.term.col - n,
            self.term.c.y,
            self.term.col - 1,
            self.term.c.y,
        );
    }

    pub fn tinsertblank(&mut self, n: i32) {
        let n = limit(n, 0, self.term.col - self.term.c.x);
        let dst = self.term.c.x + n;
        let src = self.term.c.x;
        let size = (self.term.col - dst) as usize;
        let y = self.term.c.y as usize;
        self.term.line[y].copy_within(src as usize..src as usize + size, dst as usize);
        self.tclearregion(src, self.term.c.y, dst - 1, self.term.c.y);
    }

    pub fn tinsertblankline(&mut self, n: i32) {
        if between(self.term.c.y, self.term.top, self.term.bot) {
            self.tscrolldown(self.term.c.y, n);
        }
    }

    pub fn tdeleteline(&mut self, n: i32) {
        if between(self.term.c.y, self.term.top, self.term.bot) {
            self.tscrollup(self.term.c.y, n, false);
        }
    }
}

// vt100 line-drawing table (0x41 - 0x7e), stolen from rxvt.
static VT100_0: [Option<&str>; 62] = [
    Some("↑"),
    Some("↓"),
    Some("→"),
    Some("←"),
    Some("█"),
    Some("▚"),
    Some("☃"), // A-G
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None, // H-O
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None, // P-W
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some(" "), // X-_
    Some("◆"),
    Some("▒"),
    Some("␉"),
    Some("␌"),
    Some("␍"),
    Some("␊"),
    Some("°"),
    Some("±"), // `-g
    Some("␤"),
    Some("␋"),
    Some("┘"),
    Some("┐"),
    Some("┌"),
    Some("└"),
    Some("┼"),
    Some("⎺"), // h-o
    Some("⎻"),
    Some("─"),
    Some("⎼"),
    Some("⎽"),
    Some("├"),
    Some("┤"),
    Some("┴"),
    Some("┬"), // p-w
    Some("│"),
    Some("≤"),
    Some("≥"),
    Some("π"),
    Some("≠"),
    Some("£"),
    Some("·"), // x-~
];

// ---------------------------------------------------------------------------
// Escape parsing
// ---------------------------------------------------------------------------

/// C's `strtol(s, &end, 10)` over a byte slice.
fn strtol10(s: &[u8], start: usize) -> (i64, usize) {
    let mut i = start;
    while i < s.len() && (s[i] == b' ' || (0x09..=0x0d).contains(&s[i])) {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let mut any = false;
    let mut v: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        any = true;
        v = v.saturating_mul(10).saturating_add((s[i] - b'0') as i64);
        i += 1;
    }
    if !any {
        return (0, start);
    }
    (if neg { -v } else { v }, i)
}

/// Capabilities reported through XTGETTCAP: `Some(Some(v))` for a string or
/// number, `Some(None)` for a boolean that is present, `None` if unknown.
fn tcap_value(name: &[u8]) -> Option<Option<String>> {
    let v = match name {
        b"TN" | b"name" => Some(config::TERMNAME.to_string()),
        b"Co" | b"colors" => Some("256".to_string()),
        b"RGB" | b"Tc" | b"Su" => None,
        b"Smulx" => Some("\x1b[4:%p1%dm".to_string()),
        b"Setulc" => Some(
            "\x1b[58:2::%p1%{65536}%/%d:%p1%{256}%/%{255}%&%d:%p1%{255}%&%dm".to_string(),
        ),
        b"Sync" => Some("\x1b[?2026%?%p1%{1}%-%tl%eh%;".to_string()),
        b"Ss" => Some("\x1b[%p1%d q".to_string()),
        b"Se" => Some("\x1b[2 q".to_string()),
        _ => return None,
    };
    Some(v)
}

pub fn hexencode(b: &[u8]) -> String {
    b.iter().map(|c| format!("{:02X}", c)).collect()
}

pub fn hexdecode(s: &[u8]) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    s.chunks(2)
        .map(|p| {
            let h = (p[0] as char).to_digit(16)?;
            let l = (p[1] as char).to_digit(16)?;
            Some((h * 16 + l) as u8)
        })
        .collect()
}

impl App {
    pub fn csiparse(&mut self) {
        let buf = self.csiescseq.buf;
        let len = self.csiescseq.len;
        let mut p = 0usize;
        let mut narg = 0usize;

        // Parameter prefix: `?` (DEC private), `>`, `=` or `<`.
        let mut prefix = 0u8;
        if p < len && matches!(buf[p], b'?' | b'>' | b'=' | b'<') {
            prefix = buf[p];
            p += 1;
        }
        self.csiescseq.prefix = prefix;
        self.csiescseq.priv_ = (prefix == b'?') as u8;
        self.csiescseq.sub = [false; ESC_ARG_SIZ];

        let mut parse: Vec<u8> = buf[..len].to_vec();
        parse.push(0);

        // Arguments are separated by `;`; `:` introduces a sub-parameter
        // (ITU T.416 colours, underline styles).  Each separator is looked
        // at on its own, so `38:2::1:2:3;4:3` keeps its structure.
        let mut next_sub = false;
        while p < len {
            let (v, np) = strtol10(&parse, p);
            let v = if v == i64::MAX || v == i64::MIN || v > i32::MAX as i64 || v < i32::MIN as i64 {
                -1
            } else {
                v
            };
            self.csiescseq.arg[narg] = v as i32;
            self.csiescseq.sub[narg] = next_sub;
            narg += 1;
            p = np;
            if p >= len || narg == ESC_ARG_SIZ {
                break;
            }
            match parse[p] {
                b';' => next_sub = false,
                b':' => next_sub = true,
                _ => break,
            }
            p += 1;
        }
        self.csiescseq.narg = narg;

        self.csiescseq.mode[0] = if p < parse.len() { parse[p] } else { 0 };
        p += 1;
        self.csiescseq.mode[1] = if p < len { buf[p] } else { 0 };
    }

    pub fn csireset(&mut self) {
        self.csiescseq = CSIEscape::default();
    }

    pub fn csidump(&self) {
        let mut out = String::from("ESC[");
        for &b in &self.csiescseq.buf[..self.csiescseq.len] {
            dump_byte(&mut out, b as u32);
        }
        out.push('\n');
        eprint!("{}", out);
    }

    fn csi_unknown(&mut self) {
        eprint!("erresc: unknown csi ");
        self.csidump();
    }

    /// Parse an extended colour (`38`, `48`, `58`) starting at `attr[*npar]`
    /// and leave `*npar` on its last argument.  Both the `;` form
    /// (`38;2;r;g;b`, `38;5;n`) and the `:` form (`38:2::r:g:b`,
    /// `38:2:id:r:g:b`, `38:5:n`) are accepted.
    pub fn tdefcolor(&mut self, attr: &[i32], sub: &[bool], npar: &mut usize, l: usize) -> i32 {
        let mut idx: i32 = -1;

        let colon = sub.get(*npar + 1).copied().unwrap_or(false);
        // Number of `:` sub-parameters following the attribute.
        let nsub = sub[*npar + 1..l.min(sub.len())]
            .iter()
            .take_while(|&&b| b)
            .count();

        let kind = attr.get(*npar + 1).copied().unwrap_or(0);
        match kind {
            2 => {
                // direct color in RGB space; the colon form may carry a
                // colour-space id before the components.
                let skip = if colon && nsub >= 5 { 1 } else { 0 };
                if *npar + 4 + skip >= l {
                    eprintln!("erresc(38): Incorrect number of parameters ({})", *npar);
                    return idx;
                }
                let r = attr[*npar + 2 + skip] as u32;
                let g = attr[*npar + 3 + skip] as u32;
                let b = attr[*npar + 4 + skip] as u32;
                *npar += 4 + skip;
                if !(0..=255).contains(&r) || !(0..=255).contains(&g) || !(0..=255).contains(&b) {
                    eprintln!("erresc: bad rgb color ({},{},{})", r, g, b);
                } else {
                    idx = truecolor(r, g, b) as i32;
                }
            }
            5 => {
                // indexed color
                if *npar + 2 >= l {
                    eprintln!("erresc(38): Incorrect number of parameters ({})", *npar);
                    return idx;
                }
                *npar += 2;
                let c = attr.get(*npar).copied().unwrap_or(0);
                if !between(c, 0, 255) {
                    eprintln!("erresc: bad fgcolor {}", c);
                } else {
                    idx = c;
                }
            }
            _ => {
                eprintln!(
                    "erresc(38): gfx attr {} unknown",
                    attr.get(*npar).copied().unwrap_or(0)
                );
            }
        }

        idx
    }

    pub fn tsetattr(&mut self, attr: &[i32], sub: &[bool]) {
        let l = attr.len();
        let issub = |j: usize| sub.get(j).copied().unwrap_or(false);
        let mut i = 0usize;
        while i < l {
            match attr[i] {
                0 => {
                    self.term.c.attr.mode &= !(ATTR_BOLD
                        | ATTR_FAINT
                        | ATTR_ITALIC
                        | ATTR_UNDERLINE
                        | ATTR_BLINK
                        | ATTR_REVERSE
                        | ATTR_INVISIBLE
                        | ATTR_STRUCK);
                    self.term.c.attr.fg = DEFAULTFG;
                    self.term.c.attr.bg = DEFAULTBG;
                    self.term.c.attr.ustyle = UL_NONE;
                    self.term.c.attr.ucolor = 0;
                }
                1 => self.term.c.attr.mode |= ATTR_BOLD,
                2 => self.term.c.attr.mode |= ATTR_FAINT,
                3 => self.term.c.attr.mode |= ATTR_ITALIC,
                4 => {
                    // `4:n` selects an underline style, plain `4` a single one.
                    let style = if issub(i + 1) {
                        match attr[i + 1] {
                            0 => UL_NONE,
                            2 => UL_DOUBLE,
                            3 => UL_CURLY,
                            4 => UL_DOTTED,
                            5 => UL_DASHED,
                            _ => UL_SINGLE,
                        }
                    } else {
                        UL_SINGLE
                    };
                    self.setunderline(style);
                }
                21 => self.setunderline(UL_DOUBLE),
                5 | 6 => self.term.c.attr.mode |= ATTR_BLINK,
                7 => self.term.c.attr.mode |= ATTR_REVERSE,
                8 => self.term.c.attr.mode |= ATTR_INVISIBLE,
                9 => self.term.c.attr.mode |= ATTR_STRUCK,
                22 => self.term.c.attr.mode &= !(ATTR_BOLD | ATTR_FAINT),
                23 => self.term.c.attr.mode &= !ATTR_ITALIC,
                24 => self.setunderline(UL_NONE),
                25 => self.term.c.attr.mode &= !ATTR_BLINK,
                27 => self.term.c.attr.mode &= !ATTR_REVERSE,
                28 => self.term.c.attr.mode &= !ATTR_INVISIBLE,
                29 => self.term.c.attr.mode &= !ATTR_STRUCK,
                38 => {
                    let idx = self.tdefcolor(attr, sub, &mut i, l);
                    if idx >= 0 {
                        self.term.c.attr.fg = idx as u32;
                    }
                }
                39 => self.term.c.attr.fg = DEFAULTFG,
                48 => {
                    let idx = self.tdefcolor(attr, sub, &mut i, l);
                    if idx >= 0 {
                        self.term.c.attr.bg = idx as u32;
                    }
                }
                49 => self.term.c.attr.bg = DEFAULTBG,
                58 => {
                    let idx = self.tdefcolor(attr, sub, &mut i, l);
                    if idx >= 0 {
                        self.term.c.attr.ucolor = idx as u32 | UCOLOR_SET;
                    }
                }
                59 => self.term.c.attr.ucolor = 0,
                a => {
                    if between(a, 30, 37) {
                        self.term.c.attr.fg = (a - 30) as u32;
                    } else if between(a, 40, 47) {
                        self.term.c.attr.bg = (a - 40) as u32;
                    } else if between(a, 90, 97) {
                        self.term.c.attr.fg = (a - 90 + 8) as u32;
                    } else if between(a, 100, 107) {
                        self.term.c.attr.bg = (a - 100 + 8) as u32;
                    } else {
                        eprintln!("erresc(default): gfx attr {} unknown", a);
                        self.csidump();
                    }
                }
            }
            i += 1;
            // Sub-parameters nobody consumed belong to the attribute above.
            while i < l && issub(i) {
                i += 1;
            }
        }
    }

    fn setunderline(&mut self, style: u8) {
        self.term.c.attr.ustyle = style;
        self.term.c.attr.mode = if style == UL_NONE {
            self.term.c.attr.mode & !ATTR_UNDERLINE
        } else {
            self.term.c.attr.mode | ATTR_UNDERLINE
        };
    }

    pub fn tsetscroll(&mut self, t: i32, b: i32) {
        let mut t = limit(t, 0, self.term.row - 1);
        let mut b = limit(b, 0, self.term.row - 1);
        if t > b {
            std::mem::swap(&mut t, &mut b);
        }
        self.term.top = t;
        self.term.bot = b;
    }

    pub fn tsetmode(&mut self, priv_: bool, set: bool, args: &[i32]) {
        let set = set as i32;
        for &arg in args {
            if priv_ {
                match arg {
                    1 => self.xsetmode(set != 0, MODE_APPCURSOR),
                    5 => self.xsetmode(set != 0, MODE_REVERSE),
                    6 => {
                        self.term.c.state = modbit(self.term.c.state, set != 0, CURSOR_ORIGIN);
                        self.tmoveato(0, 0);
                    }
                    7 => self.term.mode = modbit(self.term.mode, set != 0, MODE_WRAP),
                    0 | 2 | 3 | 4 | 8 | 18 | 19 | 42 | 12 => {}
                    25 => self.xsetmode(set == 0, MODE_HIDE),
                    9 => {
                        self.xsetpointermotion(false);
                        self.xsetmode(false, MODE_MOUSE);
                        self.xsetmode(set != 0, MODE_MOUSEX10);
                    }
                    1000 => {
                        self.xsetpointermotion(false);
                        self.xsetmode(false, MODE_MOUSE);
                        self.xsetmode(set != 0, MODE_MOUSEBTN);
                    }
                    1002 => {
                        self.xsetpointermotion(false);
                        self.xsetmode(false, MODE_MOUSE);
                        self.xsetmode(set != 0, MODE_MOUSEMOTION);
                    }
                    1003 => {
                        self.xsetpointermotion(set != 0);
                        self.xsetmode(false, MODE_MOUSE);
                        self.xsetmode(set != 0, MODE_MOUSEMANY);
                    }
                    1004 => self.xsetmode(set != 0, MODE_FOCUS),
                    1006 => self.xsetmode(set != 0, MODE_MOUSESGR),
                    1034 => self.xsetmode(set != 0, MODE_8BIT),
                    1049 => {
                        if self.allowaltscreen {
                            self.tcursor(if set != 0 {
                                CursorMovement::Save
                            } else {
                                CursorMovement::Load
                            });
                            let alt = self.is_set(MODE_ALTSCREEN);
                            if alt {
                                self.tclearregion(0, 0, self.term.col - 1, self.term.row - 1);
                            }
                            if set ^ (alt as i32) != 0 {
                                self.tswapscreen();
                            }
                            self.tcursor(if set != 0 {
                                CursorMovement::Save
                            } else {
                                CursorMovement::Load
                            });
                        }
                    }
                    47 | 1047 => {
                        if self.allowaltscreen {
                            let alt = self.is_set(MODE_ALTSCREEN);
                            if alt {
                                self.tclearregion(0, 0, self.term.col - 1, self.term.row - 1);
                            }
                            if set ^ (alt as i32) != 0 {
                                self.tswapscreen();
                            }
                        }
                    }
                    1048 => self.tcursor(if set != 0 {
                        CursorMovement::Save
                    } else {
                        CursorMovement::Load
                    }),
                    2004 => self.xsetmode(set != 0, MODE_BRCKTPASTE),
                    2026 => self.tsetsync(set != 0),
                    1001 | 1005 | 1015 => {}
                    _ => eprintln!("erresc: unknown private set/reset mode {}", arg),
                }
            } else {
                match arg {
                    0 => {}
                    2 => self.xsetmode(set != 0, MODE_KBDLOCK),
                    4 => self.term.mode = modbit(self.term.mode, set != 0, MODE_INSERT),
                    12 => self.term.mode = modbit(self.term.mode, set == 0, MODE_ECHO),
                    20 => self.term.mode = modbit(self.term.mode, set != 0, MODE_CRLF),
                    _ => eprintln!("erresc: unknown set/reset mode {}", arg),
                }
            }
        }
    }

    /// Synchronized output (mode 2026): hold drawing until it is reset or
    /// `SYNCTIMEOUT` passes.
    pub fn tsetsync(&mut self, set: bool) {
        if set {
            if !self.is_set(MODE_SYNC) {
                self.sync_since = Some(std::time::Instant::now());
            }
            self.term.mode |= MODE_SYNC;
        } else {
            self.term.mode &= !MODE_SYNC;
            self.sync_since = None;
        }
    }

    /// Whether drawing is currently held back by synchronized output, and
    /// for how many more milliseconds.  Expired holds are dropped.
    pub fn sync_pending(&mut self, now: std::time::Instant) -> Option<f64> {
        let since = self.sync_since?;
        let elapsed = now.saturating_duration_since(since).as_millis() as f64;
        if self.is_set(MODE_SYNC) && elapsed < config::SYNCTIMEOUT {
            Some(config::SYNCTIMEOUT - elapsed)
        } else {
            self.tsetsync(false);
            None
        }
    }

    /// DECRQM answer for a mode: 1 set, 2 reset, 4 permanently reset,
    /// 0 not recognised.
    pub fn mode_state(&self, priv_: bool, n: i32) -> u8 {
        let b = |on: bool| if on { 1 } else { 2 };
        if priv_ {
            match n {
                1 => b(self.win_is_set(MODE_APPCURSOR)),
                5 => b(self.win_is_set(MODE_REVERSE)),
                6 => b(self.term.c.state & CURSOR_ORIGIN != 0),
                7 => b(self.is_set(MODE_WRAP)),
                9 => b(self.win_is_set(MODE_MOUSEX10)),
                25 => b(!self.win_is_set(MODE_HIDE)),
                47 | 1047 | 1049 => b(self.is_set(MODE_ALTSCREEN)),
                1000 => b(self.win_is_set(MODE_MOUSEBTN)),
                1002 => b(self.win_is_set(MODE_MOUSEMOTION)),
                1003 => b(self.win_is_set(MODE_MOUSEMANY)),
                1004 => b(self.win_is_set(MODE_FOCUS)),
                1006 => b(self.win_is_set(MODE_MOUSESGR)),
                1034 => b(self.win_is_set(MODE_8BIT)),
                2004 => b(self.win_is_set(MODE_BRCKTPASTE)),
                2026 => b(self.is_set(MODE_SYNC)),
                1001 | 1005 | 1015 => 4,
                _ => 0,
            }
        } else {
            match n {
                2 => b(self.win_is_set(MODE_KBDLOCK)),
                4 => b(self.is_set(MODE_INSERT)),
                12 => b(!self.is_set(MODE_ECHO)),
                20 => b(self.is_set(MODE_CRLF)),
                _ => 0,
            }
        }
    }

    /// CSI sequences with a `>`, `=` or `<` parameter prefix.  Only the
    /// queries are answered; settings such as xterm's modifyOtherKeys
    /// (`CSI > 4 ; 1 m`) or the kitty keyboard protocol (`CSI > 1 u`) are
    /// ignored so they cannot be mistaken for SGR or cursor restore.
    fn csihandle_prefixed(&mut self) {
        let prefix = self.csiescseq.prefix;
        let arg0 = self.csiescseq.arg[0];
        match (prefix, self.csiescseq.mode[0]) {
            (b'>', b'c') if arg0 == 0 => {
                // DA2 -- Secondary Device Attributes
                self.ttywrite(config::VTIDEN2, false);
            }
            (b'>', b'q') if arg0 == 0 => {
                // XTVERSION
                let buf = format!("\x1bP>|st {}-rs\x1b\\", env!("CARGO_PKG_VERSION"));
                self.ttywrite(buf.as_bytes(), false);
            }
            (b'>', b'm') | (b'>', b'n') | (b'>', b'u') | (b'<', b'u') | (b'=', b'u') => {}
            _ => self.csi_unknown(),
        }
    }

    pub fn csihandle(&mut self) {
        let mode0 = self.csiescseq.mode[0];
        let arg0 = self.csiescseq.arg[0];
        let arg1 = self.csiescseq.arg[1];
        let narg = self.csiescseq.narg;
        let priv_ = self.csiescseq.priv_ != 0;

        if matches!(self.csiescseq.prefix, b'>' | b'=' | b'<') {
            self.csihandle_prefixed();
            return;
        }

        match mode0 {
            b'@' => {
                // ICH -- Insert <n> blank char
                let n = if arg0 == 0 { 1 } else { arg0 };
                self.tinsertblank(n);
            }
            b'A' => {
                // CUU -- Cursor <n> Up
                let n = if arg0 == 0 { 1 } else { arg0 };
                self.tmoveto(self.term.c.x, self.term.c.y - n);
            }
            b'B' | b'e' => {
                // CUD / VPR
                let n = if arg0 == 0 { 1 } else { arg0 };
                self.tmoveto(self.term.c.x, self.term.c.y + n);
            }
            b'i' => {
                // MC -- Media Copy
                match arg0 {
                    0 => self.tdump(),
                    1 => self.tdumpline(self.term.c.y),
                    2 => self.tdumpsel(),
                    4 => self.term.mode &= !MODE_PRINT,
                    5 => self.term.mode |= MODE_PRINT,
                    _ => {}
                }
            }
            b'c' => {
                // DA -- Device Attributes
                if arg0 == 0 {
                    self.ttywrite(config::VTIDEN, false);
                }
            }
            b'b' => {
                // REP -- repeat last printed char
                let n = limit(arg0, 1, 65535);
                if self.term.lastc != 0 {
                    for _ in 0..n {
                        self.tputc(self.term.lastc);
                    }
                }
            }
            b'C' | b'a' => {
                // CUF / HPR
                let n = if arg0 == 0 { 1 } else { arg0 };
                self.tmoveto(self.term.c.x + n, self.term.c.y);
            }
            b'D' => {
                // CUB
                let n = if arg0 == 0 { 1 } else { arg0 };
                self.tmoveto(self.term.c.x - n, self.term.c.y);
            }
            b'E' => {
                // CNL
                let n = if arg0 == 0 { 1 } else { arg0 };
                self.tmoveto(0, self.term.c.y + n);
            }
            b'F' => {
                // CPL
                let n = if arg0 == 0 { 1 } else { arg0 };
                self.tmoveto(0, self.term.c.y - n);
            }
            b'g' => {
                // TBC
                match arg0 {
                    0 => self.term.tabs[self.term.c.x as usize] = false,
                    3 => {
                        for t in self.term.tabs.iter_mut() {
                            *t = false;
                        }
                    }
                    _ => self.csi_unknown(),
                }
            }
            b'G' | b'`' => {
                // CHA / HPA
                let n = if arg0 == 0 { 1 } else { arg0 };
                self.tmoveto(n - 1, self.term.c.y);
            }
            b'H' | b'f' => {
                // CUP / HVP
                let n0 = if arg0 == 0 { 1 } else { arg0 };
                let n1 = if arg1 == 0 { 1 } else { arg1 };
                self.tmoveato(n1 - 1, n0 - 1);
            }
            b'I' => {
                // CHT
                let n = if arg0 == 0 { 1 } else { arg0 };
                self.tputtab(n);
            }
            b'J' => {
                // ED
                match arg0 {
                    0 => {
                        self.tclearregion(
                            self.term.c.x,
                            self.term.c.y,
                            self.term.col - 1,
                            self.term.c.y,
                        );
                        if self.term.c.y < self.term.row - 1 {
                            self.tclearregion(
                                0,
                                self.term.c.y + 1,
                                self.term.col - 1,
                                self.term.row - 1,
                            );
                        }
                    }
                    1 => {
                        if self.term.c.y > 0 {
                            self.tclearregion(0, 0, self.term.col - 1, self.term.c.y - 1);
                        }
                        self.tclearregion(0, self.term.c.y, self.term.c.x, self.term.c.y);
                    }
                    2 => self.tclearregion(0, 0, self.term.col - 1, self.term.row - 1),
                    _ => self.csi_unknown(),
                }
            }
            b'K' => {
                // EL
                match arg0 {
                    0 => self.tclearregion(
                        self.term.c.x,
                        self.term.c.y,
                        self.term.col - 1,
                        self.term.c.y,
                    ),
                    1 => self.tclearregion(0, self.term.c.y, self.term.c.x, self.term.c.y),
                    2 => self.tclearregion(0, self.term.c.y, self.term.col - 1, self.term.c.y),
                    _ => {}
                }
            }
            b'S' => {
                // SU
                if !priv_ {
                    let n = if arg0 == 0 { 1 } else { arg0 };
                    self.tscrollup(self.term.top, n, false);
                }
            }
            b'T' => {
                // SD
                let n = if arg0 == 0 { 1 } else { arg0 };
                self.tscrolldown(self.term.top, n);
            }
            b'L' => {
                // IL
                let n = if arg0 == 0 { 1 } else { arg0 };
                self.tinsertblankline(n);
            }
            b'l' => {
                // RM
                let a = self.csiescseq.arg[..narg].to_vec();
                self.tsetmode(priv_, false, &a);
            }
            b'M' => {
                // DL
                let n = if arg0 == 0 { 1 } else { arg0 };
                self.tdeleteline(n);
            }
            b'X' => {
                // ECH
                let n = if arg0 == 0 { 1 } else { arg0 };
                self.tclearregion(
                    self.term.c.x,
                    self.term.c.y,
                    self.term.c.x + n - 1,
                    self.term.c.y,
                );
            }
            b'P' => {
                // DCH
                let n = if arg0 == 0 { 1 } else { arg0 };
                self.tdeletechar(n);
            }
            b'Z' => {
                // CBT
                let n = if arg0 == 0 { 1 } else { arg0 };
                self.tputtab(-n);
            }
            b'd' => {
                // VPA
                let n = if arg0 == 0 { 1 } else { arg0 };
                self.tmoveato(self.term.c.x, n - 1);
            }
            b'h' => {
                // SM
                let a = self.csiescseq.arg[..narg].to_vec();
                self.tsetmode(priv_, true, &a);
            }
            b'm' => {
                // SGR
                let a = self.csiescseq.arg[..narg].to_vec();
                let sub = self.csiescseq.sub[..narg].to_vec();
                self.tsetattr(&a, &sub);
            }
            b'n' => {
                // DSR
                match arg0 {
                    5 => self.ttywrite(b"\x1b[0n", false),
                    6 => {
                        let buf = format!("\x1b[{};{}R", self.term.c.y + 1, self.term.c.x + 1);
                        self.ttywrite(buf.as_bytes(), false);
                    }
                    _ => self.csi_unknown(),
                }
            }
            b'r' => {
                // DECSTBM
                if priv_ {
                    self.csi_unknown();
                } else {
                    let n0 = if arg0 == 0 { 1 } else { arg0 };
                    let n1 = if arg1 == 0 { self.term.row } else { arg1 };
                    self.tsetscroll(n0 - 1, n1 - 1);
                    self.tmoveato(0, 0);
                }
            }
            b's' => self.tcursor(CursorMovement::Save),
            b'u' => {
                if priv_ {
                    self.csi_unknown();
                } else {
                    self.tcursor(CursorMovement::Load);
                }
            }
            b'$' => match self.csiescseq.mode[1] {
                b'p' => {
                    // DECRQM -- request mode
                    let st = self.mode_state(priv_, arg0);
                    let buf = format!(
                        "\x1b[{}{};{}$y",
                        if priv_ { "?" } else { "" },
                        arg0,
                        st
                    );
                    self.ttywrite(buf.as_bytes(), false);
                }
                _ => self.csi_unknown(),
            },
            b' ' => match self.csiescseq.mode[1] {
                b'q' => {
                    // DECSCUSR
                    if self.xsetcursor(arg0) {
                        self.csi_unknown();
                    }
                }
                _ => self.csi_unknown(),
            },
            _ => self.csi_unknown(),
        }
    }

    pub fn osc_color_response(&mut self, num: i32, index: i32, is_osc4: bool) {
        let (r, g, b) = match self.xgetcolor(if is_osc4 { num } else { index }) {
            Some(v) => v,
            None => {
                eprintln!(
                    "erresc: failed to fetch {} color {}",
                    if is_osc4 { "osc4" } else { "osc" },
                    if is_osc4 { num } else { index }
                );
                return;
            }
        };

        let buf = format!(
            "\x1b]{}{};rgb:{:02x}{:02x}/{:02x}{:02x}/{:02x}{:02x}\x07",
            if is_osc4 { "4;" } else { "" },
            num,
            r,
            r,
            g,
            g,
            b,
            b
        );
        if buf.len() >= 32 {
            eprintln!("error: truncation occurred while printing osc response");
        } else {
            self.ttywrite(buf.as_bytes(), true);
        }
    }

    pub fn strreset(&mut self) {
        self.strescseq.reset();
    }

    pub fn strparse(&mut self) {
        self.strescseq.narg = 0;
        if self.strescseq.len == 0 {
            return;
        }
        let data = self.strescseq.buf[..self.strescseq.len].to_vec();
        let mut narg = 0usize;
        let mut start = 0usize;

        while narg < STR_ARG_SIZ {
            let mut end = start;
            while end < data.len() && data[end] != b';' {
                end += 1;
            }
            // An embedded NUL terminates the string (and the parsing) in C.
            if let Some(pos) = data[start..end].iter().position(|&b| b == 0) {
                self.strescseq.args[narg] = data[start..start + pos].to_vec();
                narg += 1;
                break;
            }
            self.strescseq.args[narg] = data[start..end].to_vec();
            narg += 1;
            if end >= data.len() {
                break;
            }
            start = end + 1;
        }
        self.strescseq.narg = narg;
    }

    pub fn strdump(&self) {
        let mut out = format!("ESC{}", self.strescseq.type_ as char);
        for &b in &self.strescseq.buf[..self.strescseq.len] {
            if b == 0 {
                out.push('\n');
                eprint!("{}", out);
                return;
            }
            dump_byte(&mut out, b as u32);
        }
        out.push_str("ESC\\\n");
        eprint!("{}", out);
    }

    pub fn strhandle(&mut self) {
        let osc_table = [
            (DEFAULTFG, "foreground"),
            (DEFAULTBG, "background"),
            (config::DEFAULTCS, "cursor"),
        ];

        self.term.esc &= !(ESC_STR_END | ESC_STR);
        self.strparse();

        let narg = self.strescseq.narg;
        let typ = self.strescseq.type_;
        let args: Vec<Vec<u8>> = self.strescseq.args[..narg].to_vec();
        let par = if narg > 0 { c_atoi(&args[0]) } else { 0 };

        match typ {
            b']' => {
                // OSC -- Operating System Command
                match par {
                    0 => {
                        if narg > 1 {
                            self.xsettitle(Some(&args[1]));
                            self.xseticontitle(Some(&args[1]));
                        }
                        return;
                    }
                    1 => {
                        if narg > 1 {
                            self.xseticontitle(Some(&args[1]));
                        }
                        return;
                    }
                    2 => {
                        if narg > 1 {
                            self.xsettitle(Some(&args[1]));
                        }
                        return;
                    }
                    7 => {
                        // current working directory: OSC 7 ; file://host/path
                        let data = self.strescseq.buf[..self.strescseq.len].to_vec();
                        if let Some(path) = data.get(2..).and_then(osc7_path) {
                            self.cwd = Some(path);
                        }
                        return;
                    }
                    8 => {
                        // hyperlink: OSC 8 ; params ; URI  (an empty URI ends it).
                        // Split the raw buffer: the URI may contain `;`.
                        let data = self.strescseq.buf[..self.strescseq.len].to_vec();
                        let rest = data.get(2..).unwrap_or(&[]);
                        let (params, uri) = match rest.iter().position(|&b| b == b';') {
                            Some(i) => (&rest[..i], &rest[i + 1..]),
                            None => (rest, &[][..]),
                        };
                        self.term.c.attr.link = self.linkid(params, uri);
                        return;
                    }
                    52 => {
                        // manipulate selection data
                        if narg > 2 && self.allowwindowops {
                            if let Some(dec) = base64dec(&args[2]) {
                                self.xsetsel(dec);
                                self.xclipcopy();
                            } else {
                                eprintln!("erresc: invalid base64");
                            }
                        }
                        return;
                    }
                    10..=12 => {
                        // set dynamic VT100 text colours
                        if narg >= 2 {
                            let j = par - 10;
                            if (0..3).contains(&j) {
                                let p = &args[1];
                                if p == b"?" {
                                    self.osc_color_response(
                                        par,
                                        osc_table[j as usize].0 as i32,
                                        false,
                                    );
                                } else if self
                                    .xsetcolorname(osc_table[j as usize].0 as i32, Some(p))
                                    != 0
                                {
                                    eprintln!(
                                        "erresc: invalid {} color: {}",
                                        osc_table[j as usize].1,
                                        String::from_utf8_lossy(p)
                                    );
                                } else {
                                    self.tfulldirt();
                                }
                                return;
                            }
                        }
                    }
                    4 | 104 => {
                        // color set / reset
                        if par != 4 || narg >= 3 {
                            let p: Option<&[u8]> = if par == 4 { Some(&args[2]) } else { None };
                            let j = if narg > 1 { c_atoi(&args[1]) } else { -1 };

                            if p == Some(&b"?"[..]) {
                                self.osc_color_response(j, 0, true);
                            } else if self.xsetcolorname(j, p) != 0 {
                                if par == 104 && narg <= 1 {
                                    // color reset without parameter
                                    self.xloadcols();
                                    return;
                                }
                                eprintln!(
                                    "erresc: invalid color j={}, p={}",
                                    j,
                                    p.map(|v| String::from_utf8_lossy(v).into_owned())
                                        .unwrap_or_else(|| "(null)".into())
                                );
                            } else {
                                // TODO: if defaultbg changes, borders are dirty
                                self.tfulldirt();
                            }
                            return;
                        }
                    }
                    110..=112 => {
                        // reset dynamic VT100 colours
                        if narg == 1 {
                            let j = par - 110;
                            if (0..3).contains(&j) {
                                if self.xsetcolorname(osc_table[j as usize].0 as i32, None) != 0 {
                                    eprintln!(
                                        "erresc: {} color not found",
                                        osc_table[j as usize].1
                                    );
                                } else {
                                    self.tfulldirt();
                                }
                                return;
                            }
                        }
                    }
                    _ => {}
                }
            }
            b'k' => {
                // old title set compatibility
                let a = if narg > 0 {
                    Some(args[0].as_slice())
                } else {
                    None
                };
                self.xsettitle(a);
                return;
            }
            b'P' => {
                // DCS: only XTGETTCAP (`DCS + q Pt ST`) is answered.
                let data = self.strescseq.buf[..self.strescseq.len].to_vec();
                if let Some(names) = data.strip_prefix(b"+q") {
                    self.xtgettcap(names);
                }
                return;
            }
            b'_' | b'^' => {
                // APC / PM
                return;
            }
            _ => {}
        }

        eprint!("erresc: unknown str ");
        self.strdump();
    }

    /// XTGETTCAP: answer each hex-encoded capability name in `names`
    /// (separated by `;`) with its terminfo value.
    fn xtgettcap(&mut self, names: &[u8]) {
        for hexname in names.split(|&b| b == b';') {
            let reply = match hexdecode(hexname).and_then(|n| tcap_value(&n)) {
                Some(Some(v)) => format!(
                    "\x1bP1+r{}={}\x1b\\",
                    String::from_utf8_lossy(hexname),
                    hexencode(v.as_bytes())
                ),
                Some(None) => format!("\x1bP1+r{}\x1b\\", String::from_utf8_lossy(hexname)),
                None => format!("\x1bP0+r{}\x1b\\", String::from_utf8_lossy(hexname)),
            };
            self.ttywrite(reply.as_bytes(), false);
        }
    }

    pub fn tstrsequence(&mut self, c: u8) {
        let c = match c {
            0x90 => b'P', // DCS
            0x9f => b'_', // APC
            0x9e => b'^', // PM
            0x9d => b']', // OSC
            _ => c,
        };
        self.strreset();
        self.strescseq.type_ = c;
        self.term.esc |= ESC_STR;
    }

    pub fn tcontrolcode(&mut self, ascii: u32) {
        match ascii {
            0x09 => {
                // HT
                self.tputtab(1);
                return;
            }
            0x08 => {
                // BS
                self.tmoveto(self.term.c.x - 1, self.term.c.y);
                return;
            }
            0x0d => {
                // CR
                self.tmoveto(0, self.term.c.y);
                return;
            }
            0x0a..=0x0c => {
                // FF VT LF
                self.tnewline(self.is_set(MODE_CRLF));
                return;
            }
            0x07 => {
                // BEL
                if self.term.esc & ESC_STR_END != 0 {
                    // backwards compatibility to xterm
                    self.strhandle();
                } else {
                    self.xbell();
                }
            }
            0x1b => {
                // ESC
                self.csireset();
                self.term.esc &= !(ESC_CSI | ESC_ALTCHARSET | ESC_TEST);
                self.term.esc |= ESC_START;
                return;
            }
            0x0e | 0x0f => {
                // SO / SI
                self.term.charset = 1 - (ascii as i32 - 0x0e);
                return;
            }
            0x1a => {
                // SUB
                let attr = self.term.c.attr;
                self.tsetchar(b'?' as Rune, &attr, self.term.c.x, self.term.c.y);
                self.csireset();
            }
            0x18 => {
                // CAN
                self.csireset();
            }
            0x05 | 0x00 | 0x11 | 0x13 | 0x7f => return,
            0x80..=0x84 => {}
            0x85 => {
                // NEL
                self.tnewline(true);
            }
            0x86 | 0x87 => {}
            0x88 => {
                // HTS
                self.term.tabs[self.term.c.x as usize] = true;
            }
            0x89..=0x8f | 0x91..=0x99 => {}
            0x9a => {
                // DECID
                self.ttywrite(config::VTIDEN, false);
            }
            0x9b | 0x9c => {}
            0x90 | 0x9d | 0x9e | 0x9f => {
                self.tstrsequence(ascii as u8);
                return;
            }
            _ => {}
        }
        // only CAN, SUB, \a and C1 chars interrupt a sequence
        self.term.esc &= !(ESC_STR_END | ESC_STR);
    }

    /// Returns 1 when the sequence is finished and it hasn't to read more
    /// characters for this sequence, otherwise 0.
    pub fn eschandle(&mut self, ascii: u32) -> bool {
        match ascii {
            0x5b => {
                self.term.esc |= ESC_CSI;
                return false;
            }
            0x23 => {
                self.term.esc |= ESC_TEST;
                return false;
            }
            0x25 => {
                self.term.esc |= ESC_UTF8;
                return false;
            }
            0x50 | 0x5f | 0x5e | 0x5d | 0x6b => {
                self.tstrsequence(ascii as u8);
                return false;
            }
            0x6e | 0x6f => {
                self.term.charset = 2 + (ascii as i32 - 0x6e);
            }
            0x28..=0x2b => {
                self.term.icharset = ascii as i32 - 0x28;
                self.term.esc |= ESC_ALTCHARSET;
                return false;
            }
            0x44 => {
                // IND
                if self.term.c.y == self.term.bot {
                    self.tscrollup(self.term.top, 1, true);
                } else {
                    self.tmoveto(self.term.c.x, self.term.c.y + 1);
                }
            }
            0x45 => {
                // NEL
                self.tnewline(true);
            }
            0x48 => {
                // HTS
                self.term.tabs[self.term.c.x as usize] = true;
            }
            0x4d => {
                // RI
                if self.term.c.y == self.term.top {
                    self.tscrolldown(self.term.top, 1);
                } else {
                    self.tmoveto(self.term.c.x, self.term.c.y - 1);
                }
            }
            0x5a => {
                // DECID
                self.ttywrite(config::VTIDEN, false);
            }
            0x63 => {
                // RIS
                self.treset();
                self.resettitle();
                self.xloadcols();
                self.xsetmode(false, MODE_HIDE);
            }
            0x3d => self.xsetmode(true, MODE_APPKEYPAD),
            0x3e => self.xsetmode(false, MODE_APPKEYPAD),
            0x37 => self.tcursor(CursorMovement::Save),
            0x38 => self.tcursor(CursorMovement::Load),
            0x5c => {
                // ST
                if self.term.esc & ESC_STR_END != 0 {
                    self.strhandle();
                }
            }
            _ => {
                eprintln!(
                    "erresc: unknown sequence ESC 0x{:02X} '{}'",
                    ascii as u8,
                    if libc_isprint(ascii) {
                        ascii as u8 as char
                    } else {
                        '.'
                    }
                );
            }
        }
        true
    }

    pub fn tputc(&mut self, u: Rune) {
        let mut c = [0u8; UTF_SIZ];
        let control = is_control(u);
        let mut width: i32 = 1;
        let len: usize;

        if u < 127 || !self.is_set(MODE_UTF8) {
            c[0] = u as u8;
            width = 1;
            len = 1;
        } else {
            len = utf8encode(u, &mut c);
            if !control {
                let w = unsafe { wcwidth(u as libc::wchar_t) };
                width = if w == -1 { 1 } else { w };
            }
        }

        if self.is_set(MODE_PRINT) {
            self.tprinter(&c[..len]);
        }

        // STR sequence consumes everything until ESC, SUB, ST or C1
        if self.term.esc & ESC_STR != 0 {
            if u == 0x07 || u == 0o30 || u == 0o32 || u == 0o33 || is_control_c1(u) {
                self.term.esc &= !(ESC_START | ESC_STR);
                self.term.esc |= ESC_STR_END;
            } else {
                self.strescseq.buf.extend_from_slice(&c[..len]);
                self.strescseq.len += len;
                return;
            }
        }

        // Actions of control codes must be performed as soon as they arrive.
        if control {
            if self.is_set(MODE_UTF8) && is_control_c1(u) {
                return;
            }
            self.tcontrolcode(u);
            // control codes are not shown ever
            if self.term.esc == 0 {
                self.term.lastc = 0;
            }
            return;
        } else if self.term.esc & ESC_START != 0 {
            if self.term.esc & ESC_CSI != 0 {
                self.csiescseq.buf[self.csiescseq.len] = u as u8;
                self.csiescseq.len += 1;
                if between(u as i32, 0x40, 0x7E)
                    || self.csiescseq.len >= self.csiescseq.buf.len() - 1
                {
                    self.term.esc = 0;
                    self.csiparse();
                    self.csihandle();
                }
                return;
            } else if self.term.esc & ESC_UTF8 != 0 {
                self.tdefutf8(u as u8);
            } else if self.term.esc & ESC_ALTCHARSET != 0 {
                self.tdeftran(u as u8);
            } else if self.term.esc & ESC_TEST != 0 {
                self.tdectest(u as u8);
            } else if !self.eschandle(u) {
                return;
            }
            self.term.esc = 0;
            return;
        }

        if self.term.scr == 0 && self.selected(self.term.c.x, self.term.c.y) {
            self.selclear();
        }

        let mut x = self.term.c.x;
        let mut y = self.term.c.y;

        if self.is_set(MODE_WRAP) && self.term.c.state & CURSOR_WRAPNEXT != 0 {
            self.term.line[y as usize][x as usize].mode |= ATTR_WRAP;
            self.tnewline(true);
            x = self.term.c.x;
            y = self.term.c.y;
        }

        if self.is_set(MODE_INSERT) && x + width < self.term.col {
            let row = &mut self.term.line[y as usize];
            let size = (self.term.col - x - width) as usize;
            row.copy_within(x as usize..x as usize + size, (x + width) as usize);
            row[x as usize].mode &= !ATTR_WIDE;
        }

        if x + width > self.term.col {
            if self.is_set(MODE_WRAP) {
                self.tnewline(true);
            } else {
                self.tmoveto(self.term.col - width, self.term.c.y);
            }
            x = self.term.c.x;
            y = self.term.c.y;
        }

        let attr = self.term.c.attr;
        self.tsetchar(u, &attr, x, y);
        self.term.lastc = u;

        if width == 2 {
            self.term.line[y as usize][x as usize].mode |= ATTR_WIDE;
            if x + 1 < self.term.col {
                if self.term.line[y as usize][(x + 1) as usize].mode == ATTR_WIDE
                    && x + 2 < self.term.col
                {
                    self.term.line[y as usize][(x + 2) as usize].u = b' ' as Rune;
                    self.term.line[y as usize][(x + 2) as usize].mode &= !ATTR_WDUMMY;
                }
                self.term.line[y as usize][(x + 1) as usize].u = 0;
                self.term.line[y as usize][(x + 1) as usize].mode = ATTR_WDUMMY;
            }
        }
        if x + width < self.term.col {
            self.tmoveto(x + width, self.term.c.y);
        } else {
            self.term.c.state |= CURSOR_WRAPNEXT;
        }
    }

    pub fn twrite(&mut self, buf: &[u8], buflen: i32, show_ctrl: bool) -> i32 {
        self.term.gen = self.term.gen.wrapping_add(1);
        let buflen = buflen as usize;
        let mut n = 0usize;
        while n < buflen {
            let charsize;
            let mut u: Rune = 0;
            if self.is_set(MODE_UTF8) {
                charsize = utf8decode(&buf[n..buflen], &mut u);
                if charsize == 0 {
                    break;
                }
            } else {
                u = buf[n] as u32;
                charsize = 1;
            }
            if show_ctrl && is_control(u) {
                if u & 0x80 != 0 {
                    u &= 0x7f;
                    self.tputc(b'^' as Rune);
                    self.tputc(b'[' as Rune);
                } else if u != 0x0a && u != 0x0d && u != 0x09 {
                    u ^= 0x40;
                    self.tputc(b'^' as Rune);
                }
            }
            self.tputc(u);
            n += charsize;
        }
        n as i32
    }

    pub fn tresize(&mut self, col: i32, row: i32) {
        self.term.gen = self.term.gen.wrapping_add(1);
        if col < 1 || row < 1 {
            eprintln!("tresize: error resizing to {}x{}", col, row);
            return;
        }

        // A width change re-wraps the main screen instead of clipping it.
        // The alternate screen keeps the original truncating behaviour: what
        // does not fit has no scrollback to go to there.
        if col != self.term.col && !self.is_set(MODE_ALTSCREEN) && !self.term.line.is_empty() {
            self.treflow(col, row);
            return;
        }

        let minrow = min(row, self.term.row);
        let mincol = min(col, self.term.col);

        // Slide screen to keep cursor where we expect it.
        let shift = if self.term.c.y >= row {
            (self.term.c.y - row + 1) as usize
        } else {
            0
        };
        if shift > 0 && shift <= self.term.line.len() {
            // Rows lost above the cursor on the main screen become history.
            if !self.is_set(MODE_ALTSCREEN) {
                for i in 0..shift {
                    let line = self.term.line[i].clone();
                    self.histpush(line);
                }
            }
            self.term.line.drain(0..shift);
            self.term.alt.drain(0..shift);
        }

        self.term.line.truncate(row as usize);
        self.term.alt.truncate(row as usize);
        self.term.line.resize_with(row as usize, Vec::new);
        self.term.alt.resize_with(row as usize, Vec::new);
        self.term.dirty.resize(row as usize, false);
        self.retab(self.term.col as usize, col as usize);

        for i in 0..minrow as usize {
            self.term.line[i].resize(col as usize, Glyph::default());
            self.term.alt[i].resize(col as usize, Glyph::default());
        }
        for i in minrow as usize..row as usize {
            self.term.line[i] = vec![Glyph::default(); col as usize];
            self.term.alt[i] = vec![Glyph::default(); col as usize];
        }

        // History lines follow the new width too, so scrolling back after a
        // resize never indexes a short line.
        let blank = Glyph {
            u: b' ' as Rune,
            fg: self.term.c.attr.fg,
            bg: self.term.c.attr.bg,
            mode: ATTR_NULL,
            ..Glyph::default()
        };
        for line in self.term.hist.iter_mut() {
            line.resize(col as usize, blank);
        }

        self.term.col = col;
        self.term.row = row;
        self.term.scr = limit(self.term.scr, 0, self.term.hist.len() as i32);

        // A selection whose bounds no longer fit must not survive: later
        // selection code (and the C original) would index retired rows.
        if self.sel.ob.x != -1
            && (self.sel.nb.x >= col
                || self.sel.ne.x >= col
                || self.sel.nb.y >= row
                || self.sel.ne.y >= row)
        {
            self.selclear();
        }

        self.tsetscroll(0, row - 1);
        self.tmoveto(self.term.c.x, self.term.c.y);

        let c = self.term.c;
        for _ in 0..2 {
            if mincol < col && 0 < minrow {
                self.tclearregion(mincol, 0, col - 1, minrow - 1);
            }
            if 0 < col && minrow < row {
                self.tclearregion(0, minrow, col - 1, row - 1);
            }
            self.tswapscreen();
            self.tcursor(CursorMovement::Load);
        }
        self.term.c = c;
    }

    /// Keep the tab stop pattern after a width change: new columns continue
    /// the multiples of `TABSPACES` reached from the last stop before the old
    /// width, exactly like the C original.
    fn retab(&mut self, oldcol: usize, col: usize) {
        self.term.tabs.resize(col, false);
        if col <= oldcol {
            return;
        }
        let mut bp = oldcol;
        loop {
            if bp == 0 {
                break;
            }
            bp -= 1;
            if self.term.tabs[bp] {
                break;
            }
        }
        bp += TABSPACES as usize;
        while bp < col {
            self.term.tabs[bp] = true;
            bp += TABSPACES as usize;
        }
    }

    /// Re-wrap the main screen and its scrollback to `col` cells per row.
    ///
    /// Rows joined by a soft-wrap mark are concatenated back into logical
    /// lines and split again at the new width, so shrinking the window wraps
    /// long lines instead of cutting them and growing it restores them.  The
    /// cursor follows its cell, rows that no longer fit move into history and
    /// the screen is refilled from history when the re-wrap needs fewer rows.
    fn treflow(&mut self, col: i32, row: i32) {
        let new_col = col as usize;
        let new_row = row as usize;
        let cur = self.term.c;
        let cur_row = self.term.hist.len() + cur.y as usize;
        // With a pending wrap the cursor sits past the last written cell.
        let wrapnext = cur.state & CURSOR_WRAPNEXT != 0;

        // Join the physical rows (history first) into logical lines.
        let src: Vec<&Line> = self.term.hist.iter().chain(self.term.line.iter()).collect();
        let mut logical: Vec<Vec<Glyph>> = Vec::new();
        let mut pending: Vec<Glyph> = Vec::new();
        let mut cur_logical = 0usize;
        let mut cur_offset = 0usize;
        for (i, line) in src.iter().enumerate() {
            let wrapped = line_is_wrapped(line);
            let mut n = line_text_len(line);
            if n > 0
                && wrapped
                && line[n - 1].u == b' ' as Rune
                && src
                    .get(i + 1)
                    .and_then(|l| l.first())
                    .is_some_and(|g| g.mode & ATTR_WIDE != 0)
            {
                // A double-width glyph that had to be pushed whole onto the
                // next row left a padding blank at the end of this one.
                n -= 1;
            }
            if i == cur_row {
                cur_logical = logical.len();
                let mut off = min(cur.x, n as i32).max(0) as usize;
                if wrapnext && n > 0 {
                    off = off.min(n - 1) + 1;
                }
                cur_offset = pending.len() + off;
            }
            pending.extend_from_slice(&line[..n]);
            if !wrapped {
                logical.push(std::mem::take(&mut pending));
            }
        }
        if !pending.is_empty() {
            logical.push(pending);
        }

        // Split the logical lines at the new width, remembering where the
        // cursor cell lands.
        let blank = Glyph {
            u: b' ' as Rune,
            fg: DEFAULTFG,
            bg: DEFAULTBG,
            mode: ATTR_NULL,
            ..Glyph::default()
        };
        let mut rows: Vec<Line> = Vec::new();
        let mut cur_abs_row = 0usize;
        let mut cur_col = 0usize;
        let mut cur_wrapnext = false;
        for (li, cells) in logical.iter().enumerate() {
            let here = li == cur_logical;
            let mut line = vec![blank; new_col];
            let mut x = 0usize;

            for (ci, cell) in cells.iter().enumerate() {
                let mut g = *cell;
                g.mode &= !ATTR_WRAP;
                // A double-width glyph must not straddle two rows.
                if x + 1 == new_col && g.mode & ATTR_WIDE != 0 {
                    push_continued(&mut rows, line);
                    line = vec![blank; new_col];
                    x = 0;
                }
                if x == new_col {
                    push_continued(&mut rows, line);
                    line = vec![blank; new_col];
                    x = 0;
                }
                if here && ci == cur_offset {
                    cur_abs_row = rows.len();
                    cur_col = x;
                }
                line[x] = g;
                x += 1;
            }

            if here && cur_offset >= cells.len() {
                cur_abs_row = rows.len();
                cur_col = x;
                if cur_col == new_col {
                    // The cursor sat just past a row-filling line: leave it
                    // pending a wrap, as tputc does at the last column.
                    cur_col = new_col - 1;
                    cur_wrapnext = true;
                }
            }
            rows.push(line);
        }

        // Keep the newest rows on screen and move the rest into history.  If
        // the rows below the cursor grew past the screen on their own, the
        // cursor still wins and the surplus at the bottom is dropped.
        let mut start = rows.len().saturating_sub(new_row);
        if start > cur_abs_row {
            start = cur_abs_row;
        }
        let mut screen: Vec<Line> = rows.split_off(start);
        screen.truncate(new_row);
        screen.resize_with(new_row, || vec![blank; new_col]);
        let mut hist: VecDeque<Line> = rows.into();
        while hist.len() > config::SCROLLBACK {
            hist.pop_front();
        }

        let oldcol = self.term.col as usize;
        self.term.line = screen;
        self.term.hist = hist;
        self.term.col = col;
        self.term.row = row;
        self.term.dirty = vec![true; new_row];
        self.retab(oldcol, new_col);

        // The hidden alternate buffer is clipped as before: it is not part
        // of the scrollback and switching back to it resets it anyway.
        self.term.alt.truncate(new_row);
        self.term.alt.resize_with(new_row, Vec::new);
        for line in self.term.alt.iter_mut() {
            line.resize(new_col, Glyph::default());
        }

        self.tsetscroll(0, row - 1);
        self.term.c.x = limit(cur_col as i32, 0, col - 1);
        self.term.c.y = (cur_abs_row - start) as i32;
        self.term.c.state &= !CURSOR_WRAPNEXT;
        if cur_wrapnext {
            self.term.c.state |= CURSOR_WRAPNEXT;
        }
        self.term.scr = limit(self.term.scr, 0, self.term.hist.len() as i32);
        self.selclear();
    }

    pub fn resettitle(&mut self) {
        self.xsettitle(None);
    }

    // -----------------------------------------------------------------------
    // Printing / dumping
    // -----------------------------------------------------------------------

    pub fn tprinter(&mut self, s: &[u8]) {
        if self.iofd != -1 && unsafe { xwrite(self.iofd, s) } < 0 {
            eprintln!("Error writing to output file: {}", last_errno_message());
            unsafe { libc::close(self.iofd) };
            self.iofd = -1;
        }
    }

    pub fn toggleprinter(&mut self, _arg: &Arg) {
        self.term.mode ^= MODE_PRINT;
    }

    pub fn printscreen(&mut self, _arg: &Arg) {
        self.tdump();
    }

    pub fn printsel(&mut self, _arg: &Arg) {
        self.tdumpsel();
    }

    pub fn sendbreak(&mut self, _arg: &Arg) {
        let rc = unsafe { libc::tcsendbreak(CMDFD.load(Ordering::Relaxed), 0) };
        if rc != 0 {
            eprintln!("Error sending break: {}", last_errno_message());
        }
    }

    pub fn tdumpsel(&mut self) {
        if let Some(ptr) = self.getsel() {
            self.tprinter(&ptr);
        }
    }

    pub fn tdumpline(&mut self, n: i32) {
        let len = min(self.tlinelen(n), self.term.col);
        if len > 0 {
            let last = (len - 1) as usize;
            let line = self.tline(n);
            let first = line[0];
            if !(last == 0 && first.u == b' ' as Rune) {
                let mut out = Vec::new();
                let mut buf = [0u8; UTF_SIZ];
                for glyph in line.iter().take(last + 1) {
                    let sz = utf8encode(glyph.u, &mut buf);
                    out.extend_from_slice(&buf[..sz]);
                }
                self.tprinter(&out);
            }
        }
        self.tprinter(b"\n");
    }

    pub fn tdump(&mut self) {
        for i in 0..self.term.row {
            self.tdumpline(i);
        }
    }

    pub fn tputtab(&mut self, n: i32) {
        let mut x = self.term.c.x as u32;
        let col = self.term.col as u32;

        if n > 0 {
            let mut n = n;
            while x < col && n > 0 {
                n -= 1;
                x += 1;
                while x < col && !self.term.tabs[x as usize] {
                    x += 1;
                }
            }
        } else if n < 0 {
            let mut n = n;
            while x > 0 && n < 0 {
                n += 1;
                x -= 1;
                while x > 0 && !self.term.tabs[x as usize] {
                    x -= 1;
                }
            }
        }
        self.term.c.x = limit(x as i32, 0, self.term.col - 1);
    }

    pub fn tdefutf8(&mut self, ascii: u8) {
        if ascii == b'G' {
            self.term.mode |= MODE_UTF8;
        } else if ascii == b'@' {
            self.term.mode &= !MODE_UTF8;
        }
    }

    pub fn tdeftran(&mut self, ascii: u8) {
        let cs = b"0B";
        let vcs = [CS_GRAPHIC0, CS_USA];
        match cs.iter().position(|&c| c == ascii) {
            Some(p) => self.term.trantbl[self.term.icharset as usize] = vcs[p],
            None => eprintln!("esc unhandled charset: ESC ( {}", ascii as char),
        }
    }

    pub fn tdectest(&mut self, c: u8) {
        if c == b'8' {
            // DEC screen alignment test
            for x in 0..self.term.col {
                for y in 0..self.term.row {
                    let attr = self.term.c.attr;
                    self.tsetchar(b'E' as Rune, &attr, x, y);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Drawing entry points (implemented in x.rs)
    // -----------------------------------------------------------------------

    pub fn drawregion(&mut self, x1: i32, y1: i32, x2: i32, y2: i32) {
        for y in y1..y2 {
            if !self.term.dirty[y as usize] {
                continue;
            }
            self.term.dirty[y as usize] = false;
            let mut line = self.tline(y).clone();
            if self.search.is_some() {
                self.search_highlight(y, &mut line);
            }
            self.xdrawline(&line, x1, y, x2);
        }
    }

    pub fn draw(&mut self) {
        let mut cx = self.term.c.x;
        let ocx = self.term.ocx;
        let ocy = self.term.ocy;

        if !self.xstartdraw() {
            return;
        }
        self.search_predraw();

        // While viewing history, all visible rows depend on the offset; a
        // normal dirty-row update would mark the wrong screen rows.
        if self.term.scr != 0 {
            self.tfulldirt();
        }

        self.term.ocx = limit(ocx, 0, self.term.col - 1);
        self.term.ocy = limit(ocy, 0, self.term.row - 1);
        if self.term.line[self.term.ocy as usize][self.term.ocx as usize].mode & ATTR_WDUMMY != 0 {
            self.term.ocx = max(self.term.ocx - 1, 0);
        }
        if self.term.line[self.term.c.y as usize][cx as usize].mode & ATTR_WDUMMY != 0 {
            cx = max(cx - 1, 0);
        }

        self.drawregion(0, 0, self.term.col, self.term.row);
        if let Some(bar) = self.searchbar() {
            // Drawn over the last row on every frame; closing the bar marks
            // the screen dirty.
            self.xdrawline(&bar, 0, self.term.row - 1, self.term.col);
        } else if self.term.scr == 0 {
            let g = self.term.line[self.term.c.y as usize][cx as usize];
            let og = self.term.line[self.term.ocy as usize][self.term.ocx as usize];
            self.xdrawcursor(cx, self.term.c.y, g, self.term.ocx, self.term.ocy, og);
        }
        self.term.ocx = cx;
        self.term.ocy = self.term.c.y;
        self.xfinishdraw();
        if ocx != self.term.ocx || ocy != self.term.ocy {
            self.xximspot(self.term.ocx, self.term.ocy);
        }
    }

    pub fn redraw(&mut self) {
        self.tfulldirt();
        self.draw();
    }
}

pub fn libc_isprint(c: u32) -> bool {
    unsafe { libc::isprint(c as i32) != 0 }
}

fn dump_byte(out: &mut String, c: u32) {
    if libc_isprint(c) {
        out.push(c as u8 as char);
    } else if c == b'\n' as u32 {
        out.push_str("(\\n)");
    } else if c == b'\r' as u32 {
        out.push_str("(\\r)");
    } else if c == 0x1b {
        out.push_str("(\\e)");
    } else {
        out.push_str(&format!("({:02x})", c));
    }
}

#[inline]
pub fn modbit(v: i32, set: bool, bit: i32) -> i32 {
    if set {
        v | bit
    } else {
        v & !bit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_app(cols: i32, rows: i32) -> App {
        // wcwidth() depends on LC_CTYPE, as in the real main().
        unsafe {
            libc::setlocale(libc::LC_CTYPE, c"".as_ptr());
        }
        let mut app = App::new(crate::x::Options::default());
        app.tnew(cols, rows);
        app
    }

    fn feed(app: &mut App, s: &[u8]) {
        app.twrite(s, s.len() as i32, false);
    }

    fn cell(app: &App, x: i32, y: i32) -> Glyph {
        app.term.line[y as usize][x as usize]
    }

    /// Text of the whole buffer (history + screen) with soft-wrapped rows
    /// joined back together; trailing blank rows are dropped.
    fn logical_text(app: &App) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut cur = String::new();
        for line in app.term.hist.iter().chain(app.term.line.iter()) {
            let n = line_text_len(line);
            for g in &line[..n] {
                cur.push(g.u as u8 as char);
            }
            if !line_is_wrapped(line) {
                out.push(std::mem::take(&mut cur));
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
        while out.last().is_some_and(|l| l.is_empty()) {
            out.pop();
        }
        out
    }

    fn row_text(app: &App, y: i32) -> String {
        let len = app.tlinelen(y) as usize;
        app.tline(y)[..len].iter().map(|g| g.u as u8 as char).collect()
    }

    #[test]
    fn utf8_roundtrip() {
        let sample = "h\u{e9}llo \u{4e16}\u{754c} \u{2603} \u{1F600}";
        for ch in sample.chars() {
            let mut buf = [0u8; UTF_SIZ];
            let n = utf8encode(ch as Rune, &mut buf);
            let mut out: Rune = 0;
            let d = utf8decode(&buf[..n], &mut out);
            assert_eq!(d, n, "decode length for {:?}", ch);
            assert_eq!(out, ch as Rune);
        }
    }

    #[test]
    fn utf8_invalid_sequences() {
        let mut u = 0;
        // lone continuation byte
        assert_eq!(utf8decode(&[0x80], &mut u), 1);
        assert_eq!(u, UTF_INVALID);
        // truncated 3-byte sequence: incomplete -> 0 (wait for more)
        assert_eq!(utf8decode(&[0xE4, 0xB8], &mut u), 0);
        // surrogate -> invalid
        assert_eq!(utf8decode(&[0xED, 0xA0, 0x80], &mut u), 3);
        assert_eq!(u, UTF_INVALID);
        // out of range -> invalid
        assert_eq!(utf8decode(&[0xF4, 0x90, 0x80, 0x80], &mut u), 4);
        assert_eq!(u, UTF_INVALID);
    }

    #[test]
    fn base64_decode() {
        assert_eq!(base64dec(b"aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64dec(b"aGVsbG8").unwrap(), b"hello");
        assert_eq!(base64dec(b"\n").unwrap(), b"");
    }

    #[test]
    fn plain_text_and_cursor() {
        let mut app = new_app(80, 24);
        feed(&mut app, b"Hello");
        assert_eq!(app.term.c.x, 5);
        assert_eq!(app.term.c.y, 0);
        for (i, b) in b"Hello".iter().enumerate() {
            assert_eq!(cell(&app, i as i32, 0).u, *b as Rune);
        }
    }

    #[test]
    fn cursor_position_and_erase() {
        let mut app = new_app(80, 24);
        feed(&mut app, b"abcdef");
        feed(&mut app, b"\x1b[1;4H"); // row 1, col 4
        assert_eq!((app.term.c.x, app.term.c.y), (3, 0));
        feed(&mut app, b"\x1b[K"); // erase to end of line
        assert_eq!(cell(&app, 2, 0).u, 'c' as Rune);
        assert_eq!(cell(&app, 3, 0).u, ' ' as Rune);
        assert_eq!(cell(&app, 79, 0).u, ' ' as Rune);

        feed(&mut app, b"\x1b[2J"); // clear screen
        for x in 0..80 {
            assert_eq!(cell(&app, x, 0).u, ' ' as Rune);
        }
    }

    #[test]
    fn sgr_colors() {
        let mut app = new_app(80, 24);
        feed(&mut app, b"\x1b[1;31mX");
        let g = cell(&app, 0, 0);
        assert_eq!(g.fg, 1);
        assert_ne!(g.mode & ATTR_BOLD, 0);

        feed(&mut app, b"\x1b[0m");
        feed(&mut app, b"\x1b[38;5;123mY");
        assert_eq!(cell(&app, 1, 0).fg, 123);

        feed(&mut app, b"\x1b[38;2;10;20;30mZ");
        assert_eq!(cell(&app, 2, 0).fg, truecolor(10, 20, 30));
    }

    #[test]
    fn wide_characters() {
        let mut app = new_app(80, 24);
        feed(&mut app, "\u{4e16}".as_bytes());
        assert_eq!(cell(&app, 0, 0).u, 0x4e16);
        assert_ne!(cell(&app, 0, 0).mode & ATTR_WIDE, 0);
        assert_eq!(cell(&app, 1, 0).mode, ATTR_WDUMMY);
        assert_eq!(app.term.c.x, 2);

        // overwriting the wide char must clear the dummy cell
        feed(&mut app, b"\x1b[1;1Hx");
        assert_eq!(cell(&app, 1, 0).mode & ATTR_WDUMMY, 0);
    }

    #[test]
    fn wrapping() {
        let mut app = new_app(5, 2);
        feed(&mut app, b"abcdef");
        assert_eq!(cell(&app, 4, 0).u, 'e' as Rune);
        assert_ne!(cell(&app, 4, 0).mode & ATTR_WRAP, 0);
        assert_eq!(cell(&app, 0, 1).u, 'f' as Rune);
        assert_eq!((app.term.c.x, app.term.c.y), (1, 1));
    }

    #[test]
    fn tabs() {
        let mut app = new_app(80, 24);
        feed(&mut app, b"\tX");
        assert_eq!(cell(&app, 8, 0).u, 'X' as Rune);
        assert_eq!(app.term.c.x, 9);
    }

    #[test]
    fn scrolling() {
        let mut app = new_app(10, 3);
        feed(&mut app, b"a\r\nb\r\nc\r\nd");
        // 'a' has scrolled off, rows are b, c, d
        assert_eq!(cell(&app, 0, 0).u, 'b' as Rune);
        assert_eq!(cell(&app, 0, 1).u, 'c' as Rune);
        assert_eq!(cell(&app, 0, 2).u, 'd' as Rune);
    }

    #[test]
    fn scrollback_history_and_view() {
        let mut app = new_app(10, 3);
        feed(&mut app, b"one\r\ntwo\r\nthree\r\nfour\r\n");

        assert_eq!(app.term.hist.len(), 2);
        assert_eq!(row_text(&app, 0), "three");
        assert_eq!(row_text(&app, 1), "four");

        app.kscrollup(&Arg::i(1));
        assert_eq!(app.term.scr, 1);
        assert_eq!(row_text(&app, 0), "two");
        assert_eq!(row_text(&app, 1), "three");
        assert_eq!(row_text(&app, 2), "four");

        app.kscrollup(&Arg::i(1));
        assert_eq!(row_text(&app, 0), "one");
        app.kscrollup(&Arg::i(1)); // clamped at the oldest line
        assert_eq!(app.term.scr, 2);

        app.kscrolldown(&Arg::i(-1)); // one page down
        assert_eq!(app.term.scr, 0);
        assert_eq!(row_text(&app, 0), "three");

        // New output must not move the view while it is scrolled back.
        app.kscrollup(&Arg::i(2));
        feed(&mut app, b"five\r\n");
        assert_eq!(app.term.scr, 3);
        assert_eq!(row_text(&app, 0), "one");
        assert_eq!(row_text(&app, 1), "two");
        assert_eq!(row_text(&app, 2), "three");

        // History lines follow a width change.
        app.tresize(5, 3);
        assert_eq!(app.term.hist.len(), 3);
        for line in &app.term.hist {
            assert_eq!(line.len(), 5);
        }
    }

    #[test]
    fn scrollback_alt_screen_shows_main_history() {
        let mut app = new_app(10, 3);
        feed(&mut app, b"a\r\nb\r\nc\r\nd\r\n");
        let histlen = app.term.hist.len();
        assert_eq!(histlen, 2);

        feed(&mut app, b"\x1b[?1049h");
        assert_ne!(app.term.mode & MODE_ALTSCREEN, 0);
        feed(&mut app, b"x\r\ny\r\nz\r\nw");
        assert_eq!(app.term.hist.len(), histlen);

        // The history belongs to the main screen and is still reachable from
        // the alternate screen; the alternate lines themselves are not saved.
        app.kscrollup(&Arg::i(1));
        assert_eq!(app.term.scr, 1);
        assert_eq!(row_text(&app, 0), "b");
        assert_eq!(row_text(&app, 1), "y");
        assert_eq!(row_text(&app, 2), "z");
        app.kscrolldown(&Arg::i(i32::MAX));
        assert_eq!(app.term.scr, 0);

        feed(&mut app, b"\x1b[?1049l");
        assert_eq!(app.term.hist.len(), histlen);
        assert_eq!(row_text(&app, 0), "c");
        assert_eq!(row_text(&app, 1), "d");
    }

    #[test]
    fn scrollback_selection() {
        let mut app = new_app(10, 3);
        feed(&mut app, b"one\r\ntwo\r\nthree\r\n");
        app.kscrollup(&Arg::i(2));

        app.selinit();
        app.selstart(0, 0, 0);
        app.selextend(2, 0, SEL_REGULAR, false);
        app.selextend(2, 0, SEL_REGULAR, true);
        assert_eq!(app.getsel().unwrap(), b"one");
    }

    #[test]
    fn alt_screen() {
        let mut app = new_app(20, 5);
        feed(&mut app, b"main");
        feed(&mut app, b"\x1b[?1049h");
        assert_ne!(app.term.mode & MODE_ALTSCREEN, 0);
        // 1049 keeps the cursor position on the alternate screen
        feed(&mut app, b"alt");
        assert_eq!(cell(&app, 4, 0).u, 'a' as Rune);
        feed(&mut app, b"\x1b[?1049l");
        assert_eq!(app.term.mode & MODE_ALTSCREEN, 0);
        assert_eq!(cell(&app, 0, 0).u, 'm' as Rune);
    }

    #[test]
    fn selection() {
        let mut app = new_app(80, 24);
        feed(&mut app, b"hello world");
        app.selinit();
        app.selstart(0, 0, 0);
        app.selextend(4, 0, SEL_REGULAR, false);
        app.selextend(4, 0, SEL_REGULAR, true);
        assert_eq!(app.getsel().unwrap(), b"hello");

        app.selstart(0, 0, 0);
        app.selextend(10, 0, SEL_REGULAR, false);
        app.selextend(10, 0, SEL_REGULAR, true);
        assert_eq!(app.getsel().unwrap(), b"hello world");
        app.selclear();
        assert!(app.getsel().is_none());
    }

    #[test]
    fn selection_snap_word_and_line() {
        let mut app = new_app(80, 24);
        feed(&mut app, b"hello world foo");
        app.selinit();

        // double click on "hello" snaps to the whole word
        app.selstart(2, 0, SNAP_WORD);
        assert_eq!(app.sel.mode, SEL_READY);
        assert_eq!(app.getsel().unwrap(), b"hello");

        // ... and "foo" in the middle
        app.selstart(13, 0, SNAP_WORD);
        assert_eq!(app.getsel().unwrap(), b"foo");

        // triple click on the line selects it including the newline
        app.selstart(5, 0, SNAP_LINE);
        assert_eq!(app.getsel().unwrap(), b"hello world foo\n");
    }

    #[test]
    fn selection_rectangular() {
        let mut app = new_app(20, 4);
        feed(&mut app, b"hello\r\nworld");
        app.selinit();
        app.selstart(0, 0, 0);
        app.selextend(4, 1, SEL_RECTANGULAR, false);
        app.selextend(4, 1, SEL_RECTANGULAR, true);
        assert_eq!(app.getsel().unwrap(), b"hello\nworld");
    }

    #[test]
    fn selection_joins_wrapped_lines() {
        let mut app = new_app(5, 3);
        feed(&mut app, b"abcdefgh");
        app.selinit();
        app.selstart(0, 0, 0);
        app.selextend(2, 1, SEL_REGULAR, false);
        app.selextend(2, 1, SEL_REGULAR, true);
        assert_eq!(app.getsel().unwrap(), b"abcdefgh");
    }

    #[test]
    fn erase_in_display_modes() {
        let mut app = new_app(10, 3);
        feed(&mut app, b"aaa\r\nbbb\r\nccc");
        feed(&mut app, b"\x1b[2;2H"); // row 2, col 2
        feed(&mut app, b"\x1b[1J"); // erase above
        assert_eq!(cell(&app, 0, 0).u, ' ' as Rune);
        assert_eq!(cell(&app, 1, 1).u, ' ' as Rune);
        assert_eq!(cell(&app, 2, 1).u, 'b' as Rune);
    }

    #[test]
    fn charsets_and_insert_mode() {
        let mut app = new_app(20, 3);
        // DEC special graphics: A -> up arrow
        feed(&mut app, b"\x1b(0A\x1b(B");
        assert_eq!(cell(&app, 0, 0).u, 0x2191);

        // insert mode: shift existing text right
        feed(&mut app, b"\x1b[1;1H\x1b[4hXY");
        assert_eq!(cell(&app, 0, 0).u, 'X' as Rune);
        assert_eq!(cell(&app, 1, 0).u, 'Y' as Rune);
    }

    #[test]
    fn resize_keeps_content_and_cursor_visible() {
        let mut app = new_app(10, 5);
        feed(&mut app, b"a\r\nb\r\nc\r\nd\r\ne");
        assert_eq!((app.term.c.x, app.term.c.y), (1, 4));

        // shrink height with the cursor on the last row: rows slide up
        app.tresize(10, 3);
        assert_eq!(app.term.row, 3);
        assert_eq!(cell(&app, 0, 0).u, 'c' as Rune);
        assert_eq!(cell(&app, 0, 1).u, 'd' as Rune);
        assert_eq!(cell(&app, 0, 2).u, 'e' as Rune);
        assert_eq!((app.term.c.x, app.term.c.y), (1, 2));

        // grow again: the screen keeps the newest rows and refills the top
        // ones from history
        app.tresize(16, 6);
        assert_eq!(app.term.row, 6);
        assert_eq!(row_text(&app, 0), "a");
        assert_eq!(row_text(&app, 4), "e");
        assert_eq!(cell(&app, 0, 5).u, ' ' as Rune);
        assert_eq!(cell(&app, 15, 5).u, ' ' as Rune);
        assert_eq!((app.term.c.x, app.term.c.y), (1, 4));

        // shrink width: retained columns are kept, cursor clamped
        app.tresize(5, 6);
        assert_eq!(app.term.col, 5);
        assert_eq!(app.term.c.x, 1);
        assert_eq!(app.term.line[5].len(), 5);
    }

    #[test]
    fn reflow_wraps_and_unwraps_lines() {
        let mut app = new_app(10, 4);
        feed(&mut app, b"abcdefghijklmnopqrst"); // 20 cells, two rows
        assert_ne!(cell(&app, 9, 0).mode & ATTR_WRAP, 0);
        assert_eq!(row_text(&app, 0), "abcdefghij");
        assert_eq!(row_text(&app, 1), "klmnopqrst");

        // narrowing splits the logical line at the new width; the newest
        // rows stay on screen and the rest moves into history
        app.tresize(5, 4);
        assert_eq!(app.term.hist.len(), 2);
        assert_eq!(row_text(&app, 0), "klmno");
        assert_eq!(row_text(&app, 1), "pqrst");
        assert_eq!((app.term.c.x, app.term.c.y), (4, 1));
        assert_ne!(app.term.c.state & CURSOR_WRAPNEXT, 0);

        // widening joins the rows marked as wrapped again
        app.tresize(10, 4);
        assert_eq!(app.term.hist.len(), 0);
        assert_eq!(row_text(&app, 0), "abcdefghij");
        assert_eq!(row_text(&app, 1), "klmnopqrst");
        assert_eq!((app.term.c.x, app.term.c.y), (9, 1));
        assert_ne!(app.term.c.state & CURSOR_WRAPNEXT, 0);

        // the cursor keeps its cell when the whole line fits one row
        app.tresize(20, 4);
        assert_eq!(row_text(&app, 0), "abcdefghijklmnopqrst");
        assert_eq!(app.term.hist.len(), 0);
        assert_eq!((app.term.c.x, app.term.c.y), (19, 0));
        assert_ne!(app.term.c.state & CURSOR_WRAPNEXT, 0);
    }

    #[test]
    fn reflow_pulls_rows_from_history() {
        let mut app = new_app(10, 3);
        feed(&mut app, b"0123456789abc\r\nsecond\r\nthird\r\n");
        // "0123456789abc" soft-wrapped; two rows are in history
        assert_eq!(app.term.hist.len(), 2);
        assert_eq!(row_text(&app, 0), "second");
        assert_eq!(row_text(&app, 1), "third");

        app.tresize(5, 3);
        // the wrapped line is re-flowed and keeps all of its text
        let hist: Vec<String> = app
            .term
            .hist
            .iter()
            .map(|l| l[..line_text_len(l)].iter().map(|g| g.u as u8 as char).collect())
            .collect();
        let screen = format!("{}{}", row_text(&app, 0), row_text(&app, 1));
        assert_eq!(format!("{}{}", hist.join(""), screen), "0123456789abcsecondthird");
        // the cursor was on the blank row below "third" and stays there
        assert_eq!((app.term.c.x, app.term.c.y), (0, 2));

        // and growing back restores the original rows
        app.tresize(10, 3);
        assert_eq!(app.term.hist.len(), 2);
        assert_eq!(row_text(&app, 0), "second");
        assert_eq!(row_text(&app, 1), "third");
    }

    #[test]
    fn reflow_round_trip_preserves_text() {
        let mut app = new_app(40, 8);
        for i in 0..120 {
            let line = format!("line {} {}\r\n", i, "x".repeat(i % 37));
            feed(&mut app, line.as_bytes());
        }
        let want = logical_text(&app);
        assert!(app.term.hist.len() > 8);

        for (col, row) in [(80, 8), (12, 5), (40, 8), (7, 3), (40, 8), (3, 2), (40, 8)] {
            app.tresize(col, row);
            assert_eq!(logical_text(&app), want, "reflow at {}x{}", col, row);
            for line in app.term.line.iter().chain(app.term.hist.iter()) {
                assert_eq!(line.len(), col as usize);
            }
        }
    }

    #[test]
    fn reflow_wide_char_invariants() {
        let mut app = new_app(20, 5);
        for i in 0..60 {
            let line = format!("{}\u{4e16}{}\r\n", "a".repeat(i % 9), "b".repeat(i % 7));
            feed(&mut app, line.as_bytes());
        }
        let want = logical_text(&app);

        for (col, row) in [(9, 4), (20, 5), (6, 3), (20, 5), (3, 4), (21, 6)] {
            app.tresize(col, row);
            for line in app.term.line.iter().chain(app.term.hist.iter()) {
                let n = line.len();
                for i in 0..n {
                    if line[i].mode & ATTR_WIDE != 0 {
                        assert!(i + 1 < n, "wide glyph at the end of {}x{}", col, row);
                        assert_eq!(line[i + 1].mode, ATTR_WDUMMY);
                    }
                    if line[i].mode & ATTR_WDUMMY != 0 {
                        assert!(i > 0, "orphan wide dummy at {}x{}", col, row);
                        assert_ne!(line[i - 1].mode & ATTR_WIDE, 0);
                    }
                }
            }
            // Coming back to the original width restores the exact text; the
            // narrower widths may only add a padding blank before a wide
            // glyph that no longer fits.
            if col == 20 {
                assert_eq!(logical_text(&app), want);
            }
        }
    }

    #[test]
    fn reflow_while_scrolled_back() {
        let mut app = new_app(10, 3);
        feed(&mut app, b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n");
        app.kscrollup(&Arg::i(2));
        assert!(app.term.scr > 0);

        app.tresize(5, 3);
        assert!(app.term.scr <= app.term.hist.len() as i32);
        // every row of the view must still be indexed safely
        for y in 0..app.term.row {
            assert_eq!(app.tline(y).len(), 5);
            let _ = app.tlinelen(y);
        }
    }

    #[test]
    fn reflow_keeps_wide_chars_whole() {
        let mut app = new_app(6, 2);
        feed(&mut app, "abc\u{4e16}".as_bytes());
        // narrowing places the wide char on the next row instead of splitting
        app.tresize(4, 2);
        assert_eq!(app.term.hist.len(), 1);
        assert_eq!(
            app.term.hist[0][..3].iter().map(|g| g.u).collect::<Vec<_>>(),
            [b'a' as Rune, b'b' as Rune, b'c' as Rune]
        );
        assert_eq!(app.term.line[0][0].u, 0x4e16);
        assert_ne!(app.term.line[0][0].mode & ATTR_WIDE, 0);
        assert_eq!(app.term.line[0][1].mode, ATTR_WDUMMY);

        // widening joins it back without accumulating padding blanks
        app.tresize(6, 2);
        assert!(app.term.hist.is_empty());
        for (i, u) in [b'a' as Rune, b'b' as Rune, b'c' as Rune, 0x4e16]
            .iter()
            .enumerate()
        {
            assert_eq!(app.term.line[0][i].u, *u);
        }
        assert_eq!(app.term.line[0][4].mode, ATTR_WDUMMY);
        assert_eq!(app.term.line[0][5].u, b' ' as Rune);
        assert_eq!(row_text(&app, 1), "");
    }

    #[test]
    fn resize_alt_screen() {
        let mut app = new_app(20, 5);
        feed(&mut app, b"main");
        feed(&mut app, b"\x1b[?1049h");
        feed(&mut app, b"alt");
        app.tresize(10, 4);
        assert_eq!(app.term.line.len(), 4);
        assert_eq!(app.term.alt.len(), 4);
        assert_eq!(cell(&app, 4, 0).u, 'a' as Rune);

        feed(&mut app, b"\x1b[?1049l");
        assert_eq!(cell(&app, 0, 0).u, 'm' as Rune);
    }

    #[test]
    fn selection_after_shrink_is_safe() {
        let mut app = new_app(20, 5);
        feed(&mut app, b"aaa\r\nbbb\r\nccc\r\nddd\r\neee");
        app.selinit();
        app.selstart(0, 0, 0);
        app.selextend(2, 4, SEL_REGULAR, false);
        app.selextend(2, 4, SEL_REGULAR, true);
        assert_eq!(app.getsel().unwrap(), b"aaa\nbbb\nccc\nddd\neee");

        // The selection now extends beyond the shrunken screen; getsel must
        // stay within the buffer instead of panicking.
        app.tresize(20, 3);
        // The out-of-range selection is dropped, so a later drag cannot index
        // retired rows either.
        assert!(app.getsel().is_none());
        app.selextend(1, 1, SEL_REGULAR, true);

        // Same for horizontal shrinking.
        let mut app = new_app(20, 3);
        feed(&mut app, b"abcdefghij");
        app.selinit();
        app.selstart(0, 0, 0);
        app.selextend(9, 0, SEL_RECTANGULAR, false);
        app.selextend(9, 0, SEL_RECTANGULAR, true);
        app.tresize(4, 3);
        assert!(app.getsel().is_none());
    }

    #[test]
    fn resize_fuzz() {
        let mut app = new_app(20, 5);
        let mut seed: u32 = 0x1234_5678;
        for _ in 0..500 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let col = 1 + (seed % 80) as i32;
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let row = 1 + (seed % 40) as i32;
            app.tresize(col, row);
            feed(&mut app, b"xyz\r\n");
            assert!(app.term.c.x >= 0 && app.term.c.x < app.term.col);
            assert!(app.term.c.y >= 0 && app.term.c.y < app.term.row);
            assert_eq!(app.term.line.len(), row as usize);
            for line in app.term.line.iter().chain(app.term.hist.iter()) {
                assert_eq!(line.len(), col as usize);
            }
            assert!(app.term.scr >= 0 && app.term.scr <= app.term.hist.len() as i32);
        }
    }

    #[test]
    fn resize_invalid_is_ignored() {
        let mut app = new_app(10, 3);
        app.tresize(0, 3);
        app.tresize(10, 0);
        app.tresize(-1, -1);
        assert_eq!((app.term.col, app.term.row), (10, 3));
    }

    #[test]
    fn tab_stops_after_resize() {
        let mut app = new_app(8, 2);
        app.tresize(20, 2);
        feed(&mut app, b"\x1b[1;1H\tX");
        assert_eq!(cell(&app, 8, 0).u, 'X' as Rune);
        feed(&mut app, b"\x1b[1;1H\t\tY");
        assert_eq!(cell(&app, 16, 0).u, 'Y' as Rune);
    }

    #[test]
    fn last_char_repeat() {
        let mut app = new_app(20, 3);
        feed(&mut app, b"A\x1b[3b");
        for x in 0..4 {
            assert_eq!(cell(&app, x, 0).u, 'A' as Rune);
        }
    }

    #[test]
    fn url_detection() {
        let mut app = new_app(80, 3);
        let line = b"see http://a.example/x, then https://b.example/y";
        feed(&mut app, line);
        let first = b"http://a.example/x,"; // the comma is a valid URL character
        let second = b"https://b.example/y";
        let p0 = line.windows(first.len()).position(|w| w == first).unwrap();
        let p1 = line.windows(second.len()).position(|w| w == second).unwrap();

        assert_eq!(
            app.urlspans(0),
            vec![
                (p0 as i32, (p0 + first.len()) as i32),
                (p1 as i32, (p1 + second.len()) as i32),
            ]
        );
        assert_eq!(app.url_at(0, p0 as i32).unwrap(), first.to_vec());
        assert_eq!(app.url_at(0, (p0 + 3) as i32).unwrap(), first.to_vec());
        assert_eq!(app.url_at(0, p1 as i32).unwrap(), second.to_vec());

        // The comma belongs to the URL; the trailing space does not.
        assert!(app.url_at(0, (p0 + first.len()) as i32).is_none());
        assert!(app.url_at(0, (p1 - 1) as i32).is_none());
        assert!(app.url_at(0, -1).is_none());
        assert!(app.url_at(0, app.term.col).is_none());
        assert!(app.url_at(app.term.row, 0).is_none());
        assert!(app.urlspans(2).is_empty());
    }

    #[test]
    fn url_prefixes_are_matched_left_to_right() {
        let mut app = new_app(80, 2);
        feed(&mut app, b"https://a http://b");
        assert_eq!(app.urlspans(0), vec![(0, 9), (10, 18)]);
    }

    #[test]
    fn url_stops_at_non_ascii() {
        let mut app = new_app(80, 2);
        feed(&mut app, "\u{4e16}http://a".as_bytes());
        assert_eq!(app.urlspans(0), vec![(2, 10)]);
    }

    #[test]
    fn url_highlight_and_unhighlight() {
        let mut app = new_app(80, 2);
        // The URL is already underlined: highlighting must not disturb SGR state.
        feed(&mut app, b"\x1b[4mhttp://a b");

        app.highlighturls();
        for x in 0..8 {
            assert_ne!(cell(&app, x, 0).mode & ATTR_URL, 0);
            assert_ne!(cell(&app, x, 0).mode & ATTR_UNDERLINE, 0);
        }
        assert_eq!(cell(&app, 8, 0).mode & ATTR_URL, 0);

        app.unhighlighturls();
        for x in 0..80 {
            assert_eq!(cell(&app, x, 0).mode & ATTR_URL, 0);
        }
        assert_ne!(cell(&app, 0, 0).mode & ATTR_UNDERLINE, 0);
    }

    #[test]
    fn cursor_styles() {
        let mut app = new_app(20, 3);
        for (style, blinks) in [
            (0, true),
            (1, true),
            (2, false),
            (3, true),
            (4, false),
            (5, true),
            (6, false),
            (7, true),
            (8, false),
        ] {
            assert!(!app.xsetcursor(style), "style {} rejected", style);
            assert_eq!(app.win.cursor, style as u32);
            assert_eq!(app.cursorblinks, blinks, "style {} blink flag", style);
        }
        // Out-of-range styles are rejected and leave the state alone.
        assert!(app.xsetcursor(-1));
        assert!(app.xsetcursor(9));
        assert_eq!(app.win.cursor, 8);
        assert!(!app.cursorblinks);
    }

    #[test]
    fn control_string_reset_rewinds_the_buffer() {
        let mut esc = STREscape::default();
        esc.buf.extend_from_slice(b"4;0;#383a42");
        esc.len = 11;
        esc.reset();
        esc.buf.extend_from_slice(b"4;1;#e45649");
        esc.len = 11;
        assert_eq!(&esc.buf[..esc.len], b"4;1;#e45649");
    }

    #[test]
    fn decscusr_sets_cursor_style() {
        let mut app = new_app(20, 3);
        for style in 0..=8 {
            feed(&mut app, format!("\x1b[{} q", style).as_bytes());
            assert_eq!(app.win.cursor, style as u32, "DECSCUSR {}", style);
        }
        assert!(!app.cursorblinks);
    }

    #[test]
    fn csi_subparameters_and_prefixes() {
        let mut app = new_app(20, 3);
        app.csiescseq.buf[..9].copy_from_slice(b"4:3;38:5m");
        app.csiescseq.len = 9;
        app.csiparse();
        assert_eq!(&app.csiescseq.arg[..4], &[4, 3, 38, 5]);
        assert_eq!(&app.csiescseq.sub[..4], &[false, true, false, true]);
        assert_eq!(app.csiescseq.mode[0], b'm');

        app.csireset();
        app.csiescseq.buf[..2].copy_from_slice(b">q");
        app.csiescseq.len = 2;
        app.csiparse();
        assert_eq!(app.csiescseq.prefix, b'>');
        assert_eq!(app.csiescseq.priv_, 0);
        assert_eq!(app.csiescseq.mode[0], b'q');
    }

    #[test]
    fn sgr_colon_and_semicolon_colours() {
        let mut app = new_app(20, 3);
        feed(&mut app, b"\x1b[38:2::1:2:3mA\x1b[38:2:0:4:5:6mB\x1b[38;2;7;8;9mC\x1b[38:5:42mD");
        assert_eq!(cell(&app, 0, 0).fg, truecolor(1, 2, 3));
        assert_eq!(cell(&app, 1, 0).fg, truecolor(4, 5, 6));
        assert_eq!(cell(&app, 2, 0).fg, truecolor(7, 8, 9));
        assert_eq!(cell(&app, 3, 0).fg, 42);
        // Mixed: colour sub-parameters must not leak into the next attribute.
        feed(&mut app, b"\x1b[0;38:2::10:20:30;1mE");
        let g = cell(&app, 4, 0);
        assert_eq!(g.fg, truecolor(10, 20, 30));
        assert_ne!(g.mode & ATTR_BOLD, 0);
        assert_eq!(g.mode & ATTR_ITALIC, 0);
    }

    #[test]
    fn underline_styles_and_colour() {
        let mut app = new_app(20, 3);
        feed(&mut app, b"\x1b[4mA\x1b[4:3mB\x1b[21mC\x1b[4:5;58:2::255:0:0mD\x1b[59mE\x1b[4:0mF\x1b[4m\x1b[24mG");
        let st = |x| (cell(&app, x, 0).ustyle, cell(&app, x, 0).mode & ATTR_UNDERLINE != 0);
        assert_eq!(st(0), (UL_SINGLE, true));
        assert_eq!(st(1), (UL_CURLY, true));
        assert_eq!(st(2), (UL_DOUBLE, true));
        assert_eq!(st(3), (UL_DASHED, true));
        assert_eq!(cell(&app, 3, 0).ucolor, truecolor(255, 0, 0) | UCOLOR_SET);
        assert_eq!(cell(&app, 4, 0).ucolor, 0);
        assert_eq!(st(5), (UL_NONE, false));
        assert_eq!(st(6), (UL_NONE, false));
        // `4:3` must not be read as underline + italic.
        assert_eq!(cell(&app, 1, 0).mode & ATTR_ITALIC, 0);

        feed(&mut app, b"\x1b[4:3;58;5;1m\x1b[0mH");
        assert_eq!(cell(&app, 7, 0).ustyle, UL_NONE);
        assert_eq!(cell(&app, 7, 0).ucolor, 0);

        // Erasing resets the cell's underline attributes.
        feed(&mut app, b"\x1b[4:3;58;5;1m\x1b[2J");
        assert_eq!(cell(&app, 1, 0).ustyle, UL_NONE);
        assert_eq!(cell(&app, 1, 0).ucolor, 0);
    }

    #[test]
    fn prefixed_settings_are_not_sgr() {
        let mut app = new_app(20, 3);
        // xterm modifyOtherKeys: must not turn on underline.
        feed(&mut app, b"\x1b[>4;1mA");
        assert_eq!(cell(&app, 0, 0).mode & ATTR_UNDERLINE, 0);
        // kitty keyboard query must not restore the cursor.
        feed(&mut app, b"\x1b[s\x1b[1;5H\x1b[<uB");
        assert_eq!(cell(&app, 4, 0).u, b'B' as Rune);
    }

    #[test]
    fn device_queries() {
        let mut app = new_app(20, 3);
        feed(&mut app, b"\x1b[>c");
        assert_eq!(app.test_out, config::VTIDEN2);
        app.test_out.clear();
        feed(&mut app, b"\x1b[c");
        assert_eq!(app.test_out, config::VTIDEN);
        app.test_out.clear();
        feed(&mut app, b"\x1b[>q");
        assert!(app.test_out.starts_with(b"\x1bP>|st "));
        assert!(app.test_out.ends_with(b"\x1b\\"));
    }

    #[test]
    fn decrqm_reports_modes() {
        let mut app = new_app(20, 3);
        feed(&mut app, b"\x1b[?2026$p");
        assert_eq!(app.test_out, b"\x1b[?2026;2$y");
        app.test_out.clear();
        feed(&mut app, b"\x1b[?2026h\x1b[?2026$p\x1b[?7$p\x1b[?9999$p\x1b[4$p\x1b[?1015$p");
        assert_eq!(
            String::from_utf8_lossy(&app.test_out),
            "\x1b[?2026;1$y\x1b[?7;1$y\x1b[?9999;0$y\x1b[4;2$y\x1b[?1015;4$y"
        );
    }

    #[test]
    fn synchronized_output_holds_and_expires() {
        let mut app = new_app(20, 3);
        let t0 = std::time::Instant::now();
        assert!(app.sync_pending(t0).is_none());
        feed(&mut app, b"\x1b[?2026h");
        let since = app.sync_since.unwrap();
        let wait = app.sync_pending(since).unwrap();
        assert!(wait > 0.0 && wait <= config::SYNCTIMEOUT);
        feed(&mut app, b"\x1b[?2026l");
        assert!(app.sync_pending(since).is_none());
        // A hold that is never released expires.
        feed(&mut app, b"\x1b[?2026h");
        let since = app.sync_since.unwrap();
        let late = since + std::time::Duration::from_millis(config::SYNCTIMEOUT as u64 + 1);
        assert!(app.sync_pending(late).is_none());
        assert!(!app.is_set(MODE_SYNC));
    }

    #[test]
    fn xtgettcap_answers() {
        let mut app = new_app(20, 3);
        // "TN" ; "Smulx" ; "zz" ; "RGB"
        let q = format!(
            "\x1bP+q{};{};{};{}\x1b\\",
            hexencode(b"TN"),
            hexencode(b"Smulx"),
            hexencode(b"zz"),
            hexencode(b"RGB")
        );
        feed(&mut app, q.as_bytes());
        let out = String::from_utf8_lossy(&app.test_out).into_owned();
        let tn = format!("\x1bP1+r{}={}\x1b\\", hexencode(b"TN"), hexencode(config::TERMNAME.as_bytes()));
        assert!(out.contains(&tn), "{:?}", out);
        assert!(out.contains(&format!("\x1bP1+r{}={}\x1b\\", hexencode(b"Smulx"), hexencode(b"\x1b[4:%p1%dm"))));
        assert!(out.contains(&format!("\x1bP0+r{}\x1b\\", hexencode(b"zz"))));
        assert!(out.contains(&format!("\x1bP1+r{}\x1b\\", hexencode(b"RGB"))));
        assert_eq!(hexdecode(b"544E"), Some(b"TN".to_vec()));
        assert_eq!(hexdecode(b"5"), None);
    }

    #[test]
    fn osc8_hyperlinks() {
        let mut app = new_app(30, 3);
        feed(&mut app, b"a\x1b]8;;http://x.test/a;b\x1b\\link\x1b]8;;\x1b\\ z");
        assert_eq!(cell(&app, 0, 0).link, 0);
        let l = cell(&app, 1, 0).link;
        assert_ne!(l, 0);
        for x in 1..5 {
            assert_eq!(cell(&app, x, 0).link, l);
        }
        assert_eq!(cell(&app, 5, 0).link, 0);
        assert_eq!(app.url_at(0, 2), Some(b"http://x.test/a;b".to_vec()));
        assert_eq!(app.url_at(0, 6), None);

        // Same id and URI share an entry; a different id does not.
        feed(&mut app, b"\x1b]8;;http://x.test/a;b\x07Q\x1b]8;id=2;http://x.test/a;b\x07R\x1b]8;;\x07");
        assert_eq!(cell(&app, 7, 0).link, l);
        assert_ne!(cell(&app, 8, 0).link, l);

        // Highlighting with Ctrl marks the linked cells.
        app.highlighturls();
        assert_ne!(cell(&app, 1, 0).mode & ATTR_URL, 0);
        assert_eq!(cell(&app, 0, 0).mode & ATTR_URL, 0);
        app.unhighlighturls();

        // Erasing drops the link from the cells.
        feed(&mut app, b"\x1b[2J");
        assert_eq!(cell(&app, 1, 0).link, 0);
    }

    #[test]
    fn hyperlink_table_is_compacted() {
        let mut app = new_app(10, 2);
        for i in 0..LINKMAX + 10 {
            let s = format!("\x1b]8;;http://h/{}\x07x\x1b]8;;\x07\r", i);
            feed(&mut app, s.as_bytes());
        }
        assert!(app.links.len() <= LINKMAX);
        // The cell on screen still points at its own URI.
        assert_eq!(app.url_at(0, 0), Some(format!("http://h/{}", LINKMAX + 9).into_bytes()));
        // A link in the middle of being written survives compaction.
        feed(&mut app, b"\x1b]8;;http://keep\x07");
        app.compactlinks();
        assert_eq!(app.links[app.term.c.attr.link as usize], b"http://keep");
    }

    #[test]
    fn osc7_reports_the_directory() {
        assert_eq!(osc7_path(b"file://host/home/me/a%20b"), Some(b"/home/me/a b".to_vec()));
        assert_eq!(osc7_path(b"file:///tmp"), Some(b"/tmp".to_vec()));
        assert_eq!(osc7_path(b"http://host/tmp"), None);
        assert_eq!(osc7_path(b"file://host/a%00b"), None);
        let mut app = new_app(10, 2);
        feed(&mut app, b"\x1b]7;file://box/srv/x\x1b\\");
        assert_eq!(app.cwd, Some(b"/srv/x".to_vec()));
    }
}
