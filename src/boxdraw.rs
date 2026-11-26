//! Built-in drawing of box-drawing characters (U+2500–U+257F) and block
//! elements (U+2580–U+259F).
//!
//! Fonts rarely make these glyphs exactly one cell wide and tall, which leaves
//! gaps between neighbouring cells.  Drawing them as rectangles that reach the
//! cell edges makes frames and bars join up whatever the font.  Diagonals are
//! left to the font.

use crate::app::Rune;

/// A filled rectangle relative to the cell origin.  `shade` is the share of
/// the foreground mixed into the background, in 1/4 steps (4 = solid).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub shade: u8,
}

impl BoxRect {
    fn solid(x: i32, y: i32, w: i32, h: i32) -> BoxRect {
        BoxRect { x, y, w, h, shade: 4 }
    }
}

// Arm weights.
const N: u8 = 0; // none
const L: u8 = 1; // light
const H: u8 = 2; // heavy
const D: u8 = 3; // double

/// Arms (up, right, down, left) of U+2500..=U+257F; `None` for characters
/// drawn another way (dashes) or left to the font (diagonals).
#[rustfmt::skip]
const LINES: [Option<[u8; 4]>; 128] = [
    // 2500 ─ ━ │ ┃ ┄ ┅ ┆ ┇ ┈ ┉ ┊ ┋ ┌ ┍ ┎ ┏
    Some([N,L,N,L]), Some([N,H,N,H]), Some([L,N,L,N]), Some([H,N,H,N]),
    None, None, None, None, None, None, None, None,
    Some([N,L,L,N]), Some([N,H,L,N]), Some([N,L,H,N]), Some([N,H,H,N]),
    // 2510 ┐ ┑ ┒ ┓ └ ┕ ┖ ┗ ┘ ┙ ┚ ┛ ├ ┝ ┞ ┟
    Some([N,N,L,L]), Some([N,N,L,H]), Some([N,N,H,L]), Some([N,N,H,H]),
    Some([L,L,N,N]), Some([L,H,N,N]), Some([H,L,N,N]), Some([H,H,N,N]),
    Some([L,N,N,L]), Some([L,N,N,H]), Some([H,N,N,L]), Some([H,N,N,H]),
    Some([L,L,L,N]), Some([L,H,L,N]), Some([H,L,L,N]), Some([L,L,H,N]),
    // 2520 ┠ ┡ ┢ ┣ ┤ ┥ ┦ ┧ ┨ ┩ ┪ ┫ ┬ ┭ ┮ ┯
    Some([H,L,H,N]), Some([H,H,L,N]), Some([L,H,H,N]), Some([H,H,H,N]),
    Some([L,N,L,L]), Some([L,N,L,H]), Some([H,N,L,L]), Some([L,N,H,L]),
    Some([H,N,H,L]), Some([H,N,L,H]), Some([L,N,H,H]), Some([H,N,H,H]),
    Some([N,L,L,L]), Some([N,L,L,H]), Some([N,H,L,L]), Some([N,H,L,H]),
    // 2530 ┰ ┱ ┲ ┳ ┴ ┵ ┶ ┷ ┸ ┹ ┺ ┻ ┼ ┽ ┾ ┿
    Some([N,L,H,L]), Some([N,L,H,H]), Some([N,H,H,L]), Some([N,H,H,H]),
    Some([L,L,N,L]), Some([L,L,N,H]), Some([L,H,N,L]), Some([L,H,N,H]),
    Some([H,L,N,L]), Some([H,L,N,H]), Some([H,H,N,L]), Some([H,H,N,H]),
    Some([L,L,L,L]), Some([L,L,L,H]), Some([L,H,L,L]), Some([L,H,L,H]),
    // 2540 ╀ ╁ ╂ ╃ ╄ ╅ ╆ ╇ ╈ ╉ ╊ ╋ ╌ ╍ ╎ ╏
    Some([H,L,L,L]), Some([L,L,H,L]), Some([H,L,H,L]), Some([H,L,L,H]),
    Some([H,H,L,L]), Some([L,L,H,H]), Some([L,H,H,L]), Some([H,H,L,H]),
    Some([L,H,H,H]), Some([H,L,H,H]), Some([H,H,H,L]), Some([H,H,H,H]),
    None, None, None, None,
    // 2550 ═ ║ ╒ ╓ ╔ ╕ ╖ ╗ ╘ ╙ ╚ ╛ ╜ ╝ ╞ ╟
    Some([N,D,N,D]), Some([D,N,D,N]), Some([N,D,L,N]), Some([N,L,D,N]),
    Some([N,D,D,N]), Some([N,N,L,D]), Some([N,N,D,L]), Some([N,N,D,D]),
    Some([L,D,N,N]), Some([D,L,N,N]), Some([D,D,N,N]), Some([L,N,N,D]),
    Some([D,N,N,L]), Some([D,N,N,D]), Some([L,D,L,N]), Some([D,L,D,N]),
    // 2560 ╠ ╡ ╢ ╣ ╤ ╥ ╦ ╧ ╨ ╩ ╪ ╫ ╬ ╭ ╮ ╯
    Some([D,D,D,N]), Some([L,N,L,D]), Some([D,N,D,L]), Some([D,N,D,D]),
    Some([N,D,L,D]), Some([N,L,D,L]), Some([N,D,D,D]), Some([L,D,N,D]),
    Some([D,L,N,L]), Some([D,D,N,D]), Some([L,D,L,D]), Some([D,L,D,L]),
    Some([D,D,D,D]), Some([N,L,L,N]), Some([N,N,L,L]), Some([L,N,N,L]),
    // 2570 ╰ ╱ ╲ ╳ ╴ ╵ ╶ ╷ ╸ ╹ ╺ ╻ ╼ ╽ ╾ ╿
    Some([L,L,N,N]), None, None, None,
    Some([N,N,N,L]), Some([L,N,N,N]), Some([N,L,N,N]), Some([N,N,L,N]),
    Some([N,N,N,H]), Some([H,N,N,N]), Some([N,H,N,N]), Some([N,N,H,N]),
    Some([N,H,N,L]), Some([L,N,H,N]), Some([N,L,N,H]), Some([H,N,L,N]),
];

/// Whether `u` is drawn by [`boxdraw`] rather than taken from the font.
pub fn is_boxdraw(u: Rune) -> bool {
    match u {
        0x2500..=0x257f => LINES[(u - 0x2500) as usize].is_some() || dashes(u).is_some(),
        0x2580..=0x259f => true,
        _ => false,
    }
}

/// Light line thickness for a cell `cw` pixels wide.
pub fn light_thickness(cw: i32) -> i32 {
    ((cw + 4) / 10).max(1)
}

/// The rectangles making up `u` in a `cw` x `ch` cell, or `None` if the
/// font should draw it.
pub fn boxdraw(u: Rune, cw: i32, ch: i32) -> Option<Vec<BoxRect>> {
    if cw <= 0 || ch <= 0 {
        return None;
    }
    match u {
        0x2500..=0x257f => {
            if let Some(arms) = LINES[(u - 0x2500) as usize] {
                Some(lines(arms, cw, ch))
            } else {
                dashes(u).map(|(n, heavy, vertical)| dashed(n, heavy, vertical, cw, ch))
            }
        }
        0x2580..=0x259f => Some(block(u, cw, ch)),
        _ => None,
    }
}

/// Dashed lines: (segments, heavy, vertical).
fn dashes(u: Rune) -> Option<(i32, bool, bool)> {
    Some(match u {
        0x2504 => (3, false, false),
        0x2505 => (3, true, false),
        0x2506 => (3, false, true),
        0x2507 => (3, true, true),
        0x2508 => (4, false, false),
        0x2509 => (4, true, false),
        0x250a => (4, false, true),
        0x250b => (4, true, true),
        0x254c => (2, false, false),
        0x254d => (2, true, false),
        0x254e => (2, false, true),
        0x254f => (2, true, true),
        _ => return None,
    })
}

fn thickness(weight: u8, lt: i32) -> i32 {
    match weight {
        L | D => lt,
        H => (2 * lt).max(lt + 1),
        _ => 0,
    }
}

/// Swap the axes of a rectangle.
fn transpose(r: BoxRect) -> BoxRect {
    BoxRect { x: r.y, y: r.x, w: r.h, h: r.w, shade: r.shade }
}

/// The two arms along one axis, drawn horizontally: `len` is the size along
/// the arms, `across` the size across them; `neg`/`pos` are the arms towards
/// 0 and towards `len`, `pneg`/`ppos` the perpendicular arms.
fn axis_arms(len: i32, across: i32, neg: u8, pos: u8, pneg: u8, ppos: u8, lt: i32) -> Vec<BoxRect> {
    let mut out = Vec::new();
    let ml = len / 2; // middle along the arms
    let ma = across / 2; // middle across them
    let o = 2 * lt; // distance between the centres of a double line's strokes
    // Perpendicular double strokes, as start positions along this axis.
    let p_near = ml - lt / 2 - o; // the stroke towards 0
    let p_far = ml - lt / 2 + o; // the stroke towards len
    let pboth = pneg == D && ppos == D;
    let pany = pneg == D || ppos == D;
    // Widest perpendicular single/heavy arm, to cover its stem.
    let pm = thickness(if pneg == D { N } else { pneg }, lt).max(thickness(if ppos == D { N } else { ppos }, lt));

    for (weight, towards_pos) in [(neg, false), (pos, true)] {
        if weight == N {
            continue;
        }
        let span = |start: i32, end: i32| -> (i32, i32) {
            // `end` is the inner end for the negative arm, `start` for the
            // positive one.
            if towards_pos {
                (start, len)
            } else {
                (0, end)
            }
        };
        if weight == D {
            // Strokes at the "neg" side (towards 0 across) and "pos" side.
            let strokes = [(ma - lt / 2 - o, pneg), (ma - lt / 2 + o, ppos)];
            for (i, &(y, side)) in strokes.iter().enumerate() {
                let other = if i == 0 { ppos } else { pneg };
                // Inner end along the axis.
                let (start, end) = if side == D {
                    // Meets the perpendicular stroke on this side: stop at
                    // the near stroke.
                    (p_far, p_near + lt)
                } else if other == D {
                    // Corner: run to the far stroke.
                    (p_near, p_far + lt)
                } else if pm > 0 {
                    (ml - pm / 2, ml - pm / 2 + pm)
                } else {
                    (ml - lt / 2, ml - lt / 2 + lt)
                };
                let (a, b) = span(start, end);
                out.push(BoxRect::solid(a, y, b - a, lt));
            }
        } else {
            let t = thickness(weight, lt);
            let y = ma - t / 2;
            let (start, end) = if pany {
                if pboth {
                    (p_far, p_near + lt)
                } else {
                    (p_near, p_far + lt)
                }
            } else if pm > 0 {
                (ml - pm / 2, ml - pm / 2 + pm)
            } else {
                (ml - t / 2, ml - t / 2 + t)
            };
            let (a, b) = span(start, end);
            out.push(BoxRect::solid(a, y, b - a, t));
        }
    }
    out
}

fn lines(arms: [u8; 4], cw: i32, ch: i32) -> Vec<BoxRect> {
    let [up, right, down, left] = arms;
    let lt = light_thickness(cw);
    let mut out = axis_arms(cw, ch, left, right, up, down, lt);
    out.extend(
        axis_arms(ch, cw, up, down, left, right, lt)
            .into_iter()
            .map(transpose),
    );
    out.retain(|r| r.w > 0 && r.h > 0);
    out
}

fn dashed(n: i32, heavy: bool, vertical: bool, cw: i32, ch: i32) -> Vec<BoxRect> {
    let lt = light_thickness(cw);
    let t = thickness(if heavy { H } else { L }, lt);
    let (len, across) = if vertical { (ch, cw) } else { (cw, ch) };
    let gap = (len / (n * 4)).max(1);
    let mut out = Vec::new();
    for i in 0..n {
        let a = i * len / n + gap / 2;
        let b = (i + 1) * len / n - (gap - gap / 2);
        if b > a {
            let r = BoxRect::solid(a, across / 2 - t / 2, b - a, t);
            out.push(if vertical { transpose(r) } else { r });
        }
    }
    out
}

fn block(u: Rune, w: i32, h: i32) -> Vec<BoxRect> {
    let hw = w / 2;
    let hh = h / 2;
    let ul = BoxRect::solid(0, 0, hw, hh);
    let ur = BoxRect::solid(hw, 0, w - hw, hh);
    let ll = BoxRect::solid(0, hh, hw, h - hh);
    let lr = BoxRect::solid(hw, hh, w - hw, h - hh);
    let r = match u {
        0x2580 => vec![BoxRect::solid(0, 0, w, hh)],
        0x2581..=0x2587 => {
            let k = (u - 0x2580) as i32;
            let bh = h * k / 8;
            vec![BoxRect::solid(0, h - bh, w, bh)]
        }
        0x2588 => vec![BoxRect::solid(0, 0, w, h)],
        0x2589..=0x258f => {
            let k = 8 - (u - 0x2588) as i32;
            vec![BoxRect::solid(0, 0, w * k / 8, h)]
        }
        0x2590 => vec![BoxRect::solid(hw, 0, w - hw, h)],
        0x2591..=0x2593 => vec![BoxRect { x: 0, y: 0, w, h, shade: (u - 0x2590) as u8 }],
        0x2594 => vec![BoxRect::solid(0, 0, w, (h / 8).max(1))],
        0x2595 => {
            let bw = (w / 8).max(1);
            vec![BoxRect::solid(w - bw, 0, bw, h)]
        }
        0x2596 => vec![ll],
        0x2597 => vec![lr],
        0x2598 => vec![ul],
        0x2599 => vec![ul, ll, lr],
        0x259a => vec![ul, lr],
        0x259b => vec![ul, ur, ll],
        0x259c => vec![ul, ur, lr],
        0x259d => vec![ur],
        0x259e => vec![ur, ll],
        0x259f => vec![ur, ll, lr],
        _ => Vec::new(),
    };
    r.into_iter().filter(|r| r.w > 0 && r.h > 0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pixel coverage of a character as a `ch` x `cw` grid.
    fn raster(u: Rune, cw: i32, ch: i32) -> Vec<Vec<bool>> {
        let mut g = vec![vec![false; cw as usize]; ch as usize];
        for r in boxdraw(u, cw, ch).unwrap() {
            assert!(r.x >= 0 && r.y >= 0 && r.x + r.w <= cw && r.y + r.h <= ch, "{:x}: {:?}", u, r);
            for y in r.y..r.y + r.h {
                for x in r.x..r.x + r.w {
                    g[y as usize][x as usize] = true;
                }
            }
        }
        g
    }

    fn row_full(g: &[Vec<bool>], y: usize) -> bool {
        g[y].iter().all(|&b| b)
    }

    fn col_full(g: &[Vec<bool>], x: usize) -> bool {
        g.iter().all(|r| r[x])
    }

    #[test]
    fn lines_reach_the_cell_edges() {
        for &(cw, ch) in &[(8, 16), (11, 23), (20, 41)] {
            let h = raster(0x2500, cw, ch); // ─
            assert!((0..ch as usize).any(|y| row_full(&h, y)), "─ {}x{}", cw, ch);
            let v = raster(0x2502, cw, ch); // │
            assert!((0..cw as usize).any(|x| col_full(&v, x)), "│ {}x{}", cw, ch);
            let c = raster(0x253c, cw, ch); // ┼
            assert!((0..ch as usize).any(|y| row_full(&c, y)));
            assert!((0..cw as usize).any(|x| col_full(&c, x)));
            // ═ has two separate full rows.
            let d = raster(0x2550, cw, ch);
            let full: Vec<usize> = (0..ch as usize).filter(|&y| row_full(&d, y)).collect();
            assert!(full.len() >= 2 && full.windows(2).any(|w| w[1] > w[0] + 1), "═ {:?}", full);
            // Heavy is thicker than light.
            let thick = |g: &Vec<Vec<bool>>| (0..ch as usize).filter(|&y| row_full(g, y)).count();
            assert!(thick(&raster(0x2501, cw, ch)) > thick(&h));
        }
    }

    #[test]
    fn corners_join() {
        let (cw, ch) = (11, 23);
        let g = raster(0x250c, cw, ch); // ┌
        // Reaches the right and bottom edges, not the left or top.
        assert!(g.iter().any(|r| r[cw as usize - 1]));
        assert!(g[ch as usize - 1].iter().any(|&b| b));
        assert!(!g.iter().any(|r| r[0]));
        assert!(!g[0].iter().any(|&b| b));
        // The horizontal and vertical strokes are connected: flood fill.
        let mut seen = vec![vec![false; cw as usize]; ch as usize];
        let start = (0..ch as usize)
            .flat_map(|y| (0..cw as usize).map(move |x| (y, x)))
            .find(|&(y, x)| g[y][x])
            .unwrap();
        let mut stack = vec![start];
        let mut n = 0;
        while let Some((y, x)) = stack.pop() {
            if seen[y][x] || !g[y][x] {
                continue;
            }
            seen[y][x] = true;
            n += 1;
            if y > 0 { stack.push((y - 1, x)); }
            if x > 0 { stack.push((y, x - 1)); }
            if y + 1 < ch as usize { stack.push((y + 1, x)); }
            if x + 1 < cw as usize { stack.push((y, x + 1)); }
        }
        let total: usize = g.iter().map(|r| r.iter().filter(|&&b| b).count()).sum();
        assert_eq!(n, total);
    }

    #[test]
    fn double_corner_has_a_hole() {
        // ╔: the inner corner leaves the area between the strokes empty.
        let (cw, ch) = (12, 24);
        let g = raster(0x2554, cw, ch);
        assert!(!g[ch as usize - 1][cw as usize / 2]);
        assert!(g.iter().any(|r| r[cw as usize - 1]));
    }

    #[test]
    fn blocks_and_shades() {
        let (cw, ch) = (10, 20);
        let full = raster(0x2588, cw, ch);
        assert!(full.iter().all(|r| r.iter().all(|&b| b)));
        let top = raster(0x2580, cw, ch);
        assert!(row_full(&top, 0) && !row_full(&top, ch as usize - 1));
        let low = boxdraw(0x2584, cw, ch).unwrap(); // ▄ lower half
        assert_eq!(low, vec![BoxRect::solid(0, 10, 10, 10)]);
        assert_eq!(boxdraw(0x2592, cw, ch).unwrap()[0].shade, 2);
        assert!(boxdraw(0x2571, cw, ch).is_none());
        assert!(!is_boxdraw(0x2571));
        assert!(is_boxdraw(0x2504) && is_boxdraw(0x259f) && !is_boxdraw('a' as Rune));
    }

    #[test]
    fn every_supported_char_stays_in_the_cell() {
        for u in 0x2500..=0x259f {
            if is_boxdraw(u) {
                for &(cw, ch) in &[(5, 9), (11, 23), (24, 50)] {
                    raster(u, cw, ch);
                }
            }
        }
    }
}
