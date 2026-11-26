//! Scrollback search.
//!
//! `Ctrl+Shift+F` opens a search bar over the last row.  Matches in the
//! history and on the screen are highlighted, `Enter`/`Shift+Enter` step to
//! older/newer ones and `Esc` closes the bar, leaving the current match
//! selected.  The matcher works on logical lines, so a word that was
//! soft-wrapped across two rows is still found.

use crate::app::*;
use crate::config;
use crate::term::{line_is_wrapped, utf8decode};

/// A match from `start` to `end` (inclusive), as (row, column).  Rows are
/// "virtual": the absolute row (history first, then the screen) plus the
/// number of history lines dropped so far, so they stay put while the
/// history rotates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub start: (u64, i32),
    pub end: (u64, i32),
}

#[derive(Clone, Debug, Default)]
pub struct Search {
    pub query: Vec<Rune>,
    pub matches: Vec<Match>,
    /// Index of the current match.
    pub cur: Option<usize>,
    /// `Term::gen` the matches were computed for (`None`: recompute).
    pub gen: Option<u64>,
}

fn fold(u: Rune, ignore_case: bool) -> Rune {
    if !ignore_case {
        return u;
    }
    match char::from_u32(u) {
        Some(c) => c.to_lowercase().next().map(|l| l as Rune).unwrap_or(u),
        None => u,
    }
}

/// Smart case: a query without capitals ignores case.
pub fn ignores_case(query: &[Rune]) -> bool {
    !query
        .iter()
        .any(|&u| char::from_u32(u).is_some_and(|c| c.is_uppercase()))
}

/// Find the non-overlapping occurrences of `query` in `rows` (absolute row
/// order), joining soft-wrapped rows into logical lines.  Positions are
/// absolute rows plus `base`.
pub fn find<'a>(rows: impl Iterator<Item = &'a Line>, query: &[Rune], base: u64) -> Vec<Match> {
    let mut out = Vec::new();
    if query.is_empty() {
        return out;
    }
    let ic = ignores_case(query);
    let q: Vec<Rune> = query.iter().map(|&u| fold(u, ic)).collect();

    // Cells of the current logical line: (folded rune, row, column).
    let mut cells: Vec<(Rune, u64, i32)> = Vec::new();
    let mut flush = |cells: &mut Vec<(Rune, u64, i32)>| {
        let mut i = 0;
        while i + q.len() <= cells.len() {
            if cells[i..i + q.len()].iter().zip(&q).all(|(c, &u)| c.0 == u) {
                let a = cells[i];
                let b = cells[i + q.len() - 1];
                out.push(Match {
                    start: (a.1, a.2),
                    end: (b.1, b.2),
                });
                i += q.len();
            } else {
                i += 1;
            }
        }
        cells.clear();
    };

    for (r, line) in rows.enumerate() {
        let row = base + r as u64;
        for (x, g) in line.iter().enumerate() {
            if g.mode & ATTR_WDUMMY != 0 {
                continue;
            }
            let u = if g.u == 0 { b' ' as Rune } else { g.u };
            cells.push((fold(u, ic), row, x as i32));
        }
        if !line_is_wrapped(line) {
            flush(&mut cells);
        }
    }
    flush(&mut cells);
    out
}

impl App {
    fn absrows(&self) -> usize {
        self.term.hist.len() + self.term.row as usize
    }

    /// Virtual row of screen (view) row `y`.
    fn view_to_virt(&self, y: i32) -> u64 {
        (self.term.hist.len() as i64 - self.term.scr as i64 + y as i64) as u64 + self.term.histdrop
    }

    /// Screen (view) row of virtual row `v`; may be off screen.
    fn virt_to_view(&self, v: u64) -> i64 {
        v as i64 - self.term.histdrop as i64 - self.term.hist.len() as i64 + self.term.scr as i64
    }

    /// Open the search bar (`Ctrl+Shift+F`).
    pub fn searchstart(&mut self, _arg: &Arg) {
        if self.search.is_none() {
            self.search = Some(Search::default());
        }
        self.tfulldirt();
    }

    /// Close the search bar, selecting the current match.
    pub fn searchclose(&mut self) {
        let cur = self
            .search
            .take()
            .and_then(|s| s.cur.and_then(|i| s.matches.get(i).copied()));
        if let Some(m) = cur {
            let (y0, y1) = (self.virt_to_view(m.start.0), self.virt_to_view(m.end.0));
            if y0 >= 0 && y1 < self.term.row as i64 {
                self.selstart(m.start.1, y0 as i32, 0);
                self.selextend(m.end.1, y1 as i32, SEL_REGULAR, false);
                self.selextend(m.end.1, y1 as i32, SEL_REGULAR, true);
                if !self.xw.dpy.is_null() {
                    let s = self.getsel();
                    self.setsel(s, 0);
                }
            }
        }
        self.tfulldirt();
    }

    /// Recompute the matches if the query or the contents changed, keeping
    /// the current match where it was.
    fn searchupdate(&mut self) {
        let gen = self.term.gen;
        let (query, old) = match &self.search {
            Some(s) if s.gen != Some(gen) => (s.query.clone(), s.cur.and_then(|i| s.matches.get(i).copied())),
            _ => return,
        };
        let rows = self.term.hist.iter().chain(self.term.line.iter());
        let matches = find(rows, &query, self.term.histdrop);
        let cur = match old {
            Some(m) => matches.iter().position(|n| n.start == m.start),
            None => None,
        }
        .or_else(|| {
            // The nearest match above the bottom of the view.
            let bottom = self.view_to_virt(self.term.row - 1);
            matches.iter().rposition(|n| n.start.0 <= bottom).or(if matches.is_empty() {
                None
            } else {
                Some(0)
            })
        });
        if let Some(s) = self.search.as_mut() {
            s.matches = matches;
            s.cur = cur;
            s.gen = Some(gen);
        }
        self.searchshow();
        self.tfulldirt();
    }

    /// Scroll so the current match is visible above the search bar.
    fn searchshow(&mut self) {
        let m = match self.search.as_ref().and_then(|s| s.cur.and_then(|i| s.matches.get(i).copied())) {
            Some(m) => m,
            None => return,
        };
        let y0 = self.virt_to_view(m.start.0);
        let y1 = self.virt_to_view(m.end.0);
        // The last row is covered by the bar.
        if y0 >= 0 && y1 < (self.term.row - 1) as i64 {
            return;
        }
        let abs = m.start.0 as i64 - self.term.histdrop as i64;
        let want = self.term.hist.len() as i64 - abs + (self.term.row / 2) as i64;
        let want = want.clamp(0, self.term.hist.len() as i64) as i32;
        let d = want - self.term.scr;
        if d > 0 {
            self.kscrollup(&Arg::i(d));
        } else if d < 0 {
            self.kscrolldown(&Arg::i(-d));
        }
    }

    /// Step to an older (`dir < 0`) or newer (`dir > 0`) match, wrapping.
    fn searchstep(&mut self, dir: i32) {
        self.searchupdate();
        if let Some(s) = self.search.as_mut() {
            let n = s.matches.len();
            if n == 0 {
                return;
            }
            s.cur = Some(match s.cur {
                Some(i) => (i as i64 + dir as i64).rem_euclid(n as i64) as usize,
                None => {
                    if dir < 0 {
                        n - 1
                    } else {
                        0
                    }
                }
            });
        }
        self.searchshow();
        self.tfulldirt();
    }

    fn searchedit(&mut self, f: impl FnOnce(&mut Vec<Rune>)) {
        if let Some(s) = self.search.as_mut() {
            f(&mut s.query);
            s.gen = None;
            s.cur = None;
        }
        self.searchupdate();
    }

    /// Handle a key while the search bar is open.  `text` is what the key
    /// typed.
    pub fn searchkey(&mut self, ksym: u64, state: u32, text: &[u8]) {
        use x11::keysym::*;
        use x11::xlib::{ControlMask, Mod1Mask, ShiftMask};
        let ksym = ksym as u32;
        let ctrl = state & ControlMask != 0;
        let shift = state & ShiftMask != 0;
        match ksym {
            XK_Escape => self.searchclose(),
            XK_Return | XK_KP_Enter => self.searchstep(if shift { 1 } else { -1 }),
            XK_Up | XK_Prior => self.searchstep(-1),
            XK_Down | XK_Next => self.searchstep(1),
            XK_F | XK_f if ctrl => self.searchstep(if shift { -1 } else { 1 }),
            XK_g | XK_c if ctrl => self.searchclose(),
            XK_BackSpace => self.searchedit(|q| {
                q.pop();
            }),
            XK_u if ctrl => self.searchedit(|q| q.clear()),
            XK_w if ctrl => self.searchedit(|q| {
                while q.last() == Some(&(b' ' as Rune)) {
                    q.pop();
                }
                while q.last().is_some_and(|&u| u != b' ' as Rune) {
                    q.pop();
                }
            }),
            _ => {
                if ctrl || state & Mod1Mask != 0 || text.is_empty() {
                    return;
                }
                let mut add = Vec::new();
                let mut i = 0;
                while i < text.len() {
                    let mut u: Rune = 0;
                    let n = utf8decode(&text[i..], &mut u);
                    if n == 0 {
                        break;
                    }
                    if !is_control(u) {
                        add.push(u);
                    }
                    i += n;
                }
                if !add.is_empty() {
                    self.searchedit(|q| q.extend(add));
                }
            }
        }
    }

    /// Column ranges `[x0, x1]` of matches on view row `y`, with whether
    /// each is the current match.
    pub fn search_spans(&mut self, y: i32) -> Vec<(i32, i32, bool)> {
        let s = match &self.search {
            Some(s) => s,
            None => return Vec::new(),
        };
        let v = self.view_to_virt(y);
        let last = self.term.col - 1;
        s.matches
            .iter()
            .enumerate()
            .filter(|(_, m)| m.start.0 <= v && v <= m.end.0)
            .map(|(i, m)| {
                let x0 = if m.start.0 == v { m.start.1 } else { 0 };
                let x1 = if m.end.0 == v { m.end.1 } else { last };
                (x0, x1, s.cur == Some(i))
            })
            .collect()
    }

    /// Colour the matches on a copy of view row `y` before it is drawn.
    pub fn search_highlight(&mut self, y: i32, line: &mut Line) {
        for (x0, x1, cur) in self.search_spans(y) {
            for x in x0.max(0)..=x1.min(line.len() as i32 - 1) {
                let g = &mut line[x as usize];
                g.mode &= !(ATTR_REVERSE | ATTR_INVISIBLE | ATTR_FAINT);
                if cur {
                    g.fg = config::SEARCH_CUR_FG;
                    g.bg = config::SEARCH_CUR_BG;
                } else {
                    g.fg = config::SEARCH_FG;
                    g.bg = config::SEARCH_BG;
                }
            }
        }
    }

    /// Bring the matches up to date before drawing.
    pub fn search_predraw(&mut self) {
        if self.search.is_some() {
            self.searchupdate();
        }
    }

    /// The search bar, `term.col` cells wide.
    pub fn searchbar(&self) -> Option<Line> {
        let s = self.search.as_ref()?;
        let col = self.term.col.max(1) as usize;
        let blank = Glyph {
            u: b' ' as Rune,
            fg: config::SEARCH_BAR_FG,
            bg: config::SEARCH_BAR_BG,
            ..Glyph::default()
        };
        let mut line = vec![blank; col];
        let status = if s.query.is_empty() {
            String::new()
        } else if s.matches.is_empty() {
            "no matches".to_string()
        } else {
            format!("{}/{}", s.cur.map(|i| i + 1).unwrap_or(0), s.matches.len())
        };
        let status: Vec<Rune> = status.chars().map(|c| c as Rune).collect();
        let prompt: Vec<Rune> = config::SEARCH_PROMPT.chars().map(|c| c as Rune).collect();

        // Room for the query: leave a space and the status on the right.
        let room = col.saturating_sub(prompt.len() + status.len() + 2);
        let q = &s.query[s.query.len().saturating_sub(room)..];
        let mut x = 0;
        for &u in prompt.iter().chain(q.iter()) {
            if x >= col {
                break;
            }
            line[x].u = u;
            x += 1;
        }
        // Block cursor after the query.
        if x < col {
            line[x].mode |= ATTR_REVERSE;
        }
        let sx = col.saturating_sub(status.len());
        for (i, &u) in status.iter().enumerate() {
            if sx + i > x {
                line[sx + i].u = u;
            }
        }
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(s: &str, wrapped: bool, col: usize) -> Line {
        let mut l: Line = s
            .chars()
            .map(|c| Glyph {
                u: c as Rune,
                ..Glyph::default()
            })
            .collect();
        l.resize(
            col,
            Glyph {
                u: b' ' as Rune,
                ..Glyph::default()
            },
        );
        if wrapped {
            l[col - 1].mode |= ATTR_WRAP;
        }
        l
    }

    fn q(s: &str) -> Vec<Rune> {
        s.chars().map(|c| c as Rune).collect()
    }

    #[test]
    fn finds_plain_and_smart_case() {
        let rows = [line("foo bar Foo", false, 12), line("xfoo", false, 12)];
        let m = find(rows.iter(), &q("foo"), 0);
        assert_eq!(m.len(), 3);
        assert_eq!(m[0], Match { start: (0, 0), end: (0, 2) });
        assert_eq!(m[1], Match { start: (0, 8), end: (0, 10) });
        assert_eq!(m[2], Match { start: (1, 1), end: (1, 3) });
        // A capital makes the search case-sensitive.
        let m = find(rows.iter(), &q("Foo"), 10);
        assert_eq!(m, vec![Match { start: (10, 8), end: (10, 10) }]);
        assert!(find(rows.iter(), &[], 0).is_empty());
    }

    #[test]
    fn finds_across_soft_wraps_only() {
        let rows = [line("abcdhel", true, 7), line("lo", false, 7), line("x", false, 7)];
        let m = find(rows.iter(), &q("hello"), 0);
        assert_eq!(m, vec![Match { start: (0, 4), end: (1, 1) }]);
        // A hard line break separates words.
        let rows = [line("abcdhel", false, 7), line("lo", false, 7)];
        assert!(find(rows.iter(), &q("hello"), 0).is_empty());
    }

    #[test]
    fn wide_characters_are_one_cell() {
        let mut l = line("", false, 6);
        l[0] = Glyph { u: 0x4e16, mode: ATTR_WIDE, ..Glyph::default() };
        l[1] = Glyph { u: 0, mode: ATTR_WDUMMY, ..Glyph::default() };
        l[2] = Glyph { u: 0x754c, mode: ATTR_WIDE, ..Glyph::default() };
        l[3] = Glyph { u: 0, mode: ATTR_WDUMMY, ..Glyph::default() };
        let m = find([l].iter(), &[0x4e16, 0x754c], 0);
        assert_eq!(m, vec![Match { start: (0, 0), end: (0, 2) }]);
    }

    fn app_with_history() -> App {
        unsafe {
            libc::setlocale(libc::LC_CTYPE, c"".as_ptr());
        }
        let mut app = App::new(crate::x::Options::default());
        app.tnew(20, 5);
        for i in 0..30 {
            let s = format!("line {}\r\n", i);
            app.twrite(s.as_bytes(), s.len() as i32, false);
        }
        app.twrite(b"needle here", 11, false);
        app
    }

    fn typ(app: &mut App, s: &str) {
        for c in s.chars() {
            let mut b = [0u8; 4];
            let t = c.encode_utf8(&mut b);
            app.searchkey(c as u64, 0, t.as_bytes());
        }
    }

    #[test]
    fn search_scrolls_to_matches_and_selects() {
        use x11::keysym::*;
        let mut app = app_with_history();
        app.searchstart(&Arg::i(0));
        typ(&mut app, "line 3");
        let s = app.search.as_ref().unwrap();
        // Only "line 3" itself: the lines go up to 29.
        assert_eq!(s.matches.len(), 1);
        assert_eq!(s.cur, Some(0));
        // Stepping keeps the only match; it lies deep in the history.
        app.searchkey(XK_Return as u64, 0, b"\r");
        let s = app.search.as_ref().unwrap();
        let m = s.matches[s.cur.unwrap()];
        let y = app.virt_to_view(m.start.0);
        assert!(y >= 0 && y < (app.term.row - 1) as i64, "match on screen: {}", y);
        assert!(app.term.scr > 0);
        let spans = app.search_spans(y as i32);
        assert!(spans.iter().any(|&(x0, x1, cur)| cur && x0 == m.start.1 && x1 == m.end.1));

        // Escape selects the match and keeps the view.
        let scr = app.term.scr;
        app.searchkey(XK_Escape as u64, 0, b"\x1b");
        assert!(app.search.is_none());
        assert_eq!(app.term.scr, scr);
        assert_eq!(app.getsel(), Some(b"line 3".to_vec()));
    }

    #[test]
    fn search_editing_and_wrap_around() {
        use x11::keysym::*;
        use x11::xlib::{ControlMask, ShiftMask};
        let mut app = app_with_history();
        app.searchstart(&Arg::i(0));
        typ(&mut app, "needle");
        assert_eq!(app.search.as_ref().unwrap().matches.len(), 1);
        app.searchkey(XK_BackSpace as u64, 0, b"\x08");
        assert_eq!(app.search.as_ref().unwrap().query.len(), 5);
        app.searchkey(XK_u as u64, ControlMask, b"\x15");
        assert!(app.search.as_ref().unwrap().query.is_empty());
        assert!(app.search.as_ref().unwrap().matches.is_empty());
        typ(&mut app, "line 2");
        // line 2, line 20..29 -> 11 matches; newest first, Shift+Enter wraps.
        let n = app.search.as_ref().unwrap().matches.len();
        assert_eq!(n, 11);
        app.searchkey(XK_Return as u64, ShiftMask, b"\r");
        assert_eq!(app.search.as_ref().unwrap().cur, Some(0));
        let bar = app.searchbar().unwrap();
        let text: String = bar.iter().map(|g| char::from_u32(g.u).unwrap()).collect();
        assert!(text.starts_with("Search: line 2"), "{:?}", text);
        assert!(text.trim_end().ends_with("1/11"), "{:?}", text);
        // New output keeps the current match on the same text.
        let before = app.search.as_ref().unwrap().matches[0];
        app.twrite(b"\r\nmore\r\n", 8, false);
        app.search_predraw();
        let s = app.search.as_ref().unwrap();
        assert_eq!(s.matches[s.cur.unwrap()], before);
    }
}
