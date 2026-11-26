# st - simple terminal (Rust port)

A faithful Rust port of [st](https://st.suckless.org/) 0.9.3, the suckless
simple terminal for X.

The emulator behaves like the original: the same escape-sequence parser,
screen/selection model, pty handling, X11 rendering through Xft, etc. I've 
added some modifications based on what I felt was missing.

## Requirements

* Rust 1.77 or newer (C string literals) and Cargo
* Xlib, Xft, fontconfig and freetype development packages
* `tic` (ncurses) for installing the terminfo entry

On Debian/Ubuntu:

```sh
sudo apt install build-essential pkg-config libx11-dev libxft-dev \
                 libfontconfig1-dev libfreetype6-dev ncurses-bin
```

## Building

```sh
cargo build --release
# or
make
```

The resulting binary is `target/release/st` (and a copy named `./st` when built
with `make`).

## Installing

```sh
sudo make install              # PREFIX=/usr/local by default
# or
sudo make install PREFIX=/usr
```

The Makefile installs the binary, the man page and runs `tic -sx st.info` so the
`st-256color` terminfo entry is available.

## Configuration

`src/config.rs` mirrors the original `config.h`: font, border width, shell,
colours, cursor shape, key bindings, mouse shortcuts, click timeouts, etc.
Edit it and rebuild (`cargo build --release` or `make`). A pristine copy of the
defaults is kept in `config.def.rs`; restore them with
`cp config.def.rs src/config.rs`.

The key/mouse/shortcut/hotkey tables are static arrays, just like the C
version, so adding a binding is a matter of appending an entry.

### Fonts and runtime zoom
		
`FONT` accepts any fontconfig pattern (for example
`DejaVu Sans Mono:pixelsize=13`) or an XLFD string; use `fc-list` to discover families. Bold, italic and bold-italic
are derived from it by fontconfig by default. For full control, set
`FONT_BOLD`, `FONT_ITALIC` and `FONT_BOLD_ITALIC` to independent patterns:

```rust
pub const FONT: &str = "Liberation Mono:pixelsize=12:antialias=true:autohint=true";
pub const FONT_BOLD: &str = "DejaVu Sans Mono:style=Bold";
pub const FONT_ITALIC: &str = "DejaVu Sans Mono:style=Oblique";
pub const FONT_BOLD_ITALIC: &str = "DejaVu Sans Mono:style=Bold Oblique";
```

Custom style fonts are used at `FONT`'s pixel size, so the character grid stays
consistent and all four styles scale together. Leave an entry empty to keep
deriving that style from `FONT` (the original behaviour).

At runtime:

* `Ctrl+Plus` (also `Ctrl+=` and keypad `+`) increases the font size.
* `Ctrl+Minus` (also `Ctrl+Shift+-`, i.e. `_`, and keypad `-`) decreases it.
* `Ctrl+0` restores the configured size.
* `Ctrl+Shift+PageUp` / `Ctrl+Shift+PageDown` / `Ctrl+Shift+Home` do the same
  (the original st bindings).

The zoom step and limits are `FONT_SIZE_STEP`, `FONT_SIZE_MIN` and
`FONT_SIZE_MAX` in `src/config.rs`. Remove or edit the corresponding `SHORTCUTS`
entries if you prefer those keys to reach applications.

### Hotkeys

`HOTKEYS` in `src/config.rs` maps key combinations to byte sequences that are
sent to the foreground program. Entries are checked before `SHORTCUTS` and the
`KEY` table, so they override the default binding for the same combination.
The default table adds the common terminal hotkeys:

```rust
pub static HOTKEYS: &[Hotkey] = &[
    // Shift+Enter: newline (ESC CR) instead of submitting the line.
    Hotkey {
        mask: ShiftMask,
        keysym: x11::keysym::XK_Return as u64,
        seq: b"\x1b\r",
    },
    // Ctrl+Home/End: application transcript top/bottom (rxvt encoding).
    Hotkey {
        mask: ControlMask,
        keysym: x11::keysym::XK_Home as u64,
        seq: b"\x1b[7^",
    },
    Hotkey {
        mask: ControlMask,
        keysym: x11::keysym::XK_End as u64,
        seq: b"\x1b[8^",
    },
];
```

Keypad Enter is bound the same way. `mask` is an X11 modifier mask
(`ShiftMask`, `ControlMask`, `Mod1Mask`, ...; combine them with `|`, or use
`XK_ANY_MOD` for any modifiers) and `keysym` is the X11 keysym. NumLock and
Caps Lock do not affect hotkeys. Change `seq` to what your program expects: for
example, some editors want a bare `b"\n"` for Shift+Enter. Bindings that call
st's own actions (copy, paste, zoom, scrollback, ...) belong in `SHORTCUTS`;
hotkeys only send bytes. Comment out an entry to restore the original st
mapping.

The Ctrl+Home/End sequences are the rxvt spellings of the modified keys. They
let full-screen programs that maintain their own transcript (pi, for example)
jump to its beginning and end; plain Home/End still reach those programs as
unmodified keys.

### Scrollback

The main screen keeps up to `SCROLLBACK` lines of history (10000 by default;
set it to 0 to disable it). Lines are saved when they scroll off the top of
the full-screen main buffer; the alternate screen is not saved. Default
controls:

* `Shift+PageUp` / `Shift+PageDown` scroll by a page.
* `Shift+Home` / `Shift+End` jump to the oldest line / back to the live screen.
* The mouse wheel scrolls line by line on the main screen, with or without
  `Shift`. On the alternate screen the wheel belongs to the application, which
  can use it to scroll its own view (as full-screen editors and pagers do).
* Any input, paste or mouse report returns to the live screen.

Ctrl+Home and Ctrl+End are not scrollback shortcuts: they are sent to the
foreground program through `HOTKEYS`, since full-screen programs commonly use
them. The history is still reachable with `Shift+PageUp` while an alternate
screen program is running; scrolling there shows the hidden main screen above
the application.

Scrolling only changes the view: the terminal keeps parsing output and the
live screen keeps updating. Selection and copying, URL clicking, and the
print-screen shortcut use the viewed lines, so text from history can be
selected and copied. The cursor is hidden while scrolled back.

Resizing re-wraps the text: lines that were cut at the old width are joined
back and split at the new one, scrollback included, so growing the window
restores what shrinking it wrapped. The cursor follows its cell; when
re-wrapping needs fewer rows the space is filled from history, and when it
needs more, the surplus rows move into history. The alternate screen keeps
st's original truncating behaviour. Zooming the font changes how many columns
and rows fit in the window, so it re-wraps the same way.

### Colours

`THEME` in `src/config.rs` picks the active colour scheme. The default is
`WHITE_THEME`, a light scheme with white background and dark text based on One
Half Light; `DARK_THEME` restores st's original palette. A `Theme` holds the 16
ANSI colours plus the cursor, reverse cursor and default foreground/background,
so the whole terminal changes from that one place. `DEFAULTFG`, `DEFAULTBG`,
`DEFAULTCS` and `DEFAULTRCS` are the indices (258, 259, 256 and 257) that
applications see in OSC queries. As before, OSC 4, 10, 11 and 104 can change
colours at runtime.

### Cursor

The cursor style follows xterm's DECSCUSR (`CSI Ps SP q`). The style used at
startup is `CURSORSTYLE` in `src/config.rs`:

| Value | Style |
| --- | --- |
| 0 | blinking block |
| 1 | blinking block (default) |
| 2 | steady block (`█`) |
| 3 | blinking underline |
| 4 | steady underline (`_`) |
| 5 | blinking bar |
| 6 | steady bar (`|`) |
| 7 | blinking st cursor |
| 8 | steady st cursor |

`STCURSOR` sets the rune drawn for styles 7 and 8 (a snowman by default).
Blinks use `BLINKTIMEOUT` (milliseconds) and share their phase with blinking
text; new input makes the cursor visible again and restarts the cycle. Set
`CURSORSTYLE` to 2 for the steady block of the original st. Applications can
switch styles at runtime with DECSCUSR, and `BLINKTIMEOUT = 0` disables
blinking everywhere.

### URLs

As with the `clickurl` patch for st, URLs on screen are underlined while
`Ctrl` is held, and `Ctrl+Click` opens the one under the pointer. The handler,
which characters count as part of a URL, and the recognised prefixes live in
`src/config.rs`:

```rust
pub const URLHANDLER: &str = "xdg-open";
pub const URLCHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ...";
pub static URLPREFIXES: &[&[u8]] = &[b"http://", b"https://"];
```

Add `b"ftp://"`, `b"mailto:"` or similar to `URLPREFIXES` for other schemes.
A URL ends at the first cell whose character is not in `URLCHARS`, so trailing
punctuation is included only if `URLCHARS` allows it (`,` and `;` are valid URL
characters and therefore part of the URL). `Ctrl+Click` anywhere else does
nothing.

### Hyperlinks (OSC 8)

Programs can attach a link to text with OSC 8
(`ESC ] 8 ; params ; URI ST` … `ESC ] 8 ; ; ST`), as `ls --hyperlink=auto`,
GCC diagnostics, systemd and many others do. Linked text looks normal; while
`Ctrl` is held it is underlined together with the detected URLs, and
`Ctrl+Click` opens its URI with `URLHANDLER`, whatever the scheme (`file://`
included). Links with the same `id=` and URI share one entry; up to `LINKMAX`
(4096) are kept and the ones no longer on screen or in the history are
dropped when the table fills up.

### Scrollback search

`Ctrl+Shift+F` opens a search bar over the last row. Matches in the history
and on the screen are highlighted as you type:

* `Enter` / `Up` jumps to the previous (older) match, `Shift+Enter` / `Down` to
  the next one; both wrap around and scroll the view to the match.
* `Backspace`, `Ctrl+U` (clear) and `Ctrl+W` (delete word) edit the query.
* `Esc` closes the bar, keeps the view where it is and selects the current
  match into PRIMARY, so `Ctrl+Shift+C` copies it.

The search ignores case unless the query contains a capital letter, and it
works on logical lines, so text that was soft-wrapped across rows is found.
New output keeps the current match in place. The prompt and colours are
`SEARCH_PROMPT` and `SEARCH_*` in `src/config.rs`.

### New terminal in the current directory

`Ctrl+Shift+Enter` opens another st window in the current directory. The
directory is the one the shell last reported with OSC 7, or else the working
directory of the foreground process (`/proc/<pid>/cwd`), which works without any
shell setup. OSC 7 reports from other hosts (a shell behind ssh) are ignored.
The new window gets this one's `-f`, `-c`, `-A` and `-a` options.

Most shells can send OSC 7 from their prompt:

```sh
# bash (~/.bashrc)
PROMPT_COMMAND='printf "\033]7;file://%s%s\033\\" "$HOSTNAME" "$PWD"'${PROMPT_COMMAND:+";$PROMPT_COMMAND"}

# zsh (~/.zshrc)
autoload -Uz add-zsh-hook
_osc7() { printf '\033]7;file://%s%s\033\\' "$HOST" "$PWD" }
add-zsh-hook chpwd _osc7; _osc7
```

fish sends OSC 7 by itself. Paths with unusual characters may be
percent-encoded; st decodes `%xx`.

### Underline styles

Besides the plain underline (SGR 4), applications can ask for double (`4:2`
or SGR 21), curly (`4:3`), dotted (`4:4`) and dashed (`4:5`) underlines, and
colour them independently of the text with SGR 58 (`58:2::r:g:b`, `58;5;n`,
…; SGR 59 resets it). Editors use this for spelling and LSP diagnostics. CSI
parameters keep their `:` sub-parameter structure, so `38:2::r:g:b` colours
work as well as `38;2;r;g;b`.

The `st-256color` terminfo entry (`st.info`) advertises these with `Smulx`,
`Setulc` and `Su`. If you use another `TERMNAME` (for example
`xterm-256color`), Neovim and Vim may need to be told about undercurl
themselves; Neovim can also find out through XTGETTCAP (below).

### Box drawing

Box-drawing characters (U+2500–U+257F: light, heavy, double and dashed lines
and their junctions) and block elements (U+2580–U+259F: halves, eighths,
quadrants and the `░▒▓` shades) are drawn by st with rectangles that reach the
cell edges, so frames from `htop`, `tmux`, `mc` or `btop` join without gaps at
any font and zoom. Rounded corners are drawn square and the diagonals
(U+2571–U+2573) still come from the font. Set `BOXDRAW = false` to use the
font for everything.

### Synchronized output and queries

Applications can bracket a redraw with mode 2026 (`CSI ? 2026 h` … `CSI ? 2026
l`); st then holds the screen until the update is complete, so full-screen
programs do not flicker or tear. A hold that is never released ends after
`SYNCTIMEOUT` (150 ms).

st also answers the queries programs use to discover these features:

| Query | Reply |
| --- | --- |
| DECRQM, `CSI ? Ps $ p` / `CSI Ps $ p` | `CSI ? Ps ; Pm $ y` (1 set, 2 reset, 4 permanently reset, 0 unknown) |
| DA2, `CSI > c` | `VTIDEN2` (`CSI > 1 ; 903 ; 0 c`) |
| XTVERSION, `CSI > q` | `DCS > \| st 0.9.3-rs ST` |
| XTGETTCAP, `DCS + q <hex names> ST` | `TN`, `Co`, `RGB`, `Tc`, `Su`, `Smulx`, `Setulc`, `Sync`, `Ss`, `Se` |

Other sequences with a `>`, `=` or `<` prefix (xterm's modifyOtherKeys,
the kitty keyboard protocol) are ignored instead of being misread as SGR or
cursor commands.

### Drag and drop

Dropping files (`text/uri-list`) or plain text (`text/plain`) from another X
client pastes the dropped paths into the terminal, space-separated and
shell-escaped, so they can be used as command arguments straight away. The
`file://` prefix and `%xx` escapes are decoded; the characters listed in
`XDNDESCCHAR` (space and shell metacharacters such as quotes, `$`, `&`, `;`,
parentheses, backtick, braces and `~`) are prefixed with a backslash. The
window advertises XDND version 5 through its `XdndAware` property.

### Background opacity and gradient

The window uses a 32-bit ARGB visual and paints the default background with
the opacity in `ALPHA` (`src/config.rs`, 0.8 by default), which a compositing
manager blends with what is behind the window. `st -A <opacity>` overrides it
at startup (0.0 transparent, 1.0 opaque), and the opacity is re-applied when
`OSC 11` changes the background colour.

Cell backgrounds also get a vertical gradient, so the top of the window is
more opaque than the bottom. Row `y` is drawn with
`GRAD_ALPHA * (window_height - y*ch) / window_height + STAT_ALPHA`; the
defaults (0.54 and 0.46) make the first row opaque and fade to about 0.46
towards the bottom. Borders and cleared regions keep the plain `ALPHA`. Set
`GRAD_ALPHA = 0.0` and `STAT_ALPHA = 1.0` for a uniform background, or swap
`win_h - y*ch` for `y*ch` in `gradient_alpha` for an inverted gradient.

Without a compositing manager the window is still drawn opaque. If the X
server has no 32-bit TrueColor visual, the terminal falls back to its default
visual and nothing is translucent.

## Testing

The complete test suite runs inside Docker under a private Xvfb display, so no
test windows appear on or interact with your desktop session. Docker Engine and
Compose v2 are required.

```sh
make test                     # build the test image if needed and run everything
make test-local               # only the Rust unit tests, on the host
make test-clean               # drop the cached Cargo target volume
```

The first `make test` builds the test image (Rust toolchain, X11/Xft development
libraries, Xvfb, Liberation/DejaVu/Droid fonts). Subsequent runs reuse it and a
named Docker volume for Cargo's target directory.

The suite covers:

* Rust unit tests: UTF-8 encode/decode (including malformed input), base64,
  text output, wrapping, wide characters, tabs, scrolling, alternate screen,
  erase modes, SGR/256/true colour, scrollback history/viewport/selection,
  selection snapping/rectangular/wrapped selections, reflow on width changes
  (wrapping, unwrapping, history, wide characters), resize fuzzing,
  selections surviving (or being cleared on) resize, CSI sub-parameters and
  prefixes, underline styles and colours, OSC 8 links and their table, OSC 7,
  DECRQM/DA2/XTVERSION/XTGETTCAP replies, synchronized output, box-drawing
  geometry, search matching (smart case, wrapped lines, wide characters) and
  navigation, and the new-terminal directory and arguments.
* Rendering: screenshots of the configured font, an explicit font, and a zoomed
  terminal exercise bold/faint/italic/underline/struck/reverse/invisible
  attributes, palette/true colour cells, DEC line drawing, wide characters and
  reverse video.
* Configured hotkeys: `Shift+Enter` and `Ctrl+Home`/`Ctrl+End` deliver their
  configured byte sequences to the child process.
* Scrollback: after 20 lines on a 5-row terminal, `Shift+Home` followed by
  `Shift+PrintScreen` dumps the oldest lines, and `Shift+End` returns to the
  live screen. The plain mouse wheel scrolls the history, and in the alternate
  screen `Shift+wheel` is forwarded to the application as a mouse report.
* Mouse selection, PRIMARY/CLIPBOARD copy, and bracketed paste are exercised
  end to end.
* Runtime font zoom (Ctrl+Plus/Minus/0, Ctrl+=, Ctrl+Shift+-), clamping at the
  configured limits, and reset.
* Custom per-style fonts: a variant is built with DejaVu bold/italic patterns,
  rendered, compared against the derived-style build, and zoomed.
* Box drawing: a row of `─` is checked pixel by pixel to be continuous across
  cells; underline styles, double frames, shades and an OSC 8 link are part of
  the rendering screenshots.
* Terminal queries: DECRQM, DA2 and XTGETTCAP replies are read back through
  the pty.
* Scrollback search: `Ctrl+Shift+F`, typing a query and `Esc` leave the view
  on a match in the history.
* `Ctrl+Shift+Enter` starts a new st whose working directory is the shell's.

## Source layout

| File | Purpose |
| --- | --- |
| `src/main.rs` | startup, command-line (`ARGBEGIN`-compatible) parsing |
| `src/app.rs` | shared state (`App`), glyphs/cursor/term types, helpers |
| `src/term.rs` | terminal core: UTF-8, escape sequences, screen, selection, pty |
| `src/x.rs` | X11 frontend: window, events, fonts, colours, drawing, clipboard |
| `src/config.rs` | configuration, generated from the original `config.h` |
| `src/icon.rs` | the 16x16 and 32x32 images published as `_NET_WM_ICON` |
| `src/boxdraw.rs` | rectangles for box-drawing characters and block elements |
| `src/search.rs` | scrollback search: matcher, keys, highlights and search bar |
| `src/ffi.rs` | fontconfig bindings and variadic Xlib entry points |

The C globals (`term`, `sel`, `win`, `xw`, `dc`, …) live in one `App` value.
Methods on `App` are split across `term.rs` and `x.rs` along the same lines as
`st.c` and `x.c`. The one process-wide pointer (`app::APP`) exists only so XIM
destroy callbacks can reach the state; the rest is regular Rust ownership.

## Intentional differences

* Screen rows, tab stops and fonts are held in bounds-checked `Vec`s instead of
  raw pointers. Behaviour on valid input is unchanged; malformed sequences that
  would be undefined behaviour in C are handled safely.
* Selections that no longer fit after the window is shrunk are cleared (the C
  original reads past the resized screen buffers in that case).
* `st -e` with no command starts the configured shell instead of dying on a
  NULL title (the C version dereferences NULL in that edge case).
* A few diagnostic messages printed to stderr have slightly different wording.
* The SIGCHLD handler exits with the same statuses but does not print the
  "child exited with status" text (printf is not async-signal-safe).
* The default cursor is a blinking block and styles 7/8 draw `STCURSOR`, as in
  the `blinking_cursor` patch; the original st defaults to a steady block.
  Set `CURSORSTYLE = 2` for the original look.
* URLs are underlined while `Ctrl` is held and opened on `Ctrl+Click`, as in the
  `clickurl` patch.
* `Shift+Enter` is a hotkey in `HOTKEYS` (ESC CR) instead of st's default
  `\r` mapping.
* The main screen keeps a scrollback history; `Shift+Home` / `Shift+End` and
  `Shift+PageUp` / `Shift+PageDown` are scrollback shortcuts instead of the
  original key sequences, and the wheel scrolls the history on the main
  screen. Set `SCROLLBACK = 0` to disable history.
* `Ctrl+Home` / `Ctrl+End` are hotkeys that send the rxvt modified Home/End
  sequences (`ESC[7^`, `ESC[8^`) instead of st's default `ESC[J`/`ESC[4~`
  mappings, so full-screen programs receive the modified keys.
* Drag-and-drop follows the `drag-n-drop` patch: dropped URIs are decoded and
  shell-escaped before being pasted. XdndFinished reports success per the XDND
  specification instead of the item count.
* Windows use a 32-bit ARGB visual with a translucent default background and a
  vertical gradient over cell backgrounds, as in the `alpha` and `gradient`
  patches; the original st uses the default visual. Set `ALPHA`, `GRAD_ALPHA`
  and `STAT_ALPHA` (or `-A`) to disable the transparency.
* The default palette is a light/white theme (`WHITE_THEME`); change
  `THEME` to `DARK_THEME` in `src/config.rs` for the original st colours. The
  window also publishes a simple terminal `_NET_WM_ICON` in two sizes for
  task switchers and panels.
* Styled/coloured underlines, OSC 8 hyperlinks, OSC 7, synchronized output
  (mode 2026) and the DECRQM, DA2, XTVERSION and XTGETTCAP queries are
  supported; the original ignores them (and reads `4:3` as underline plus
  italic). CSI sequences with a `>`, `=` or `<` prefix are only answered when
  they are queries.
* Box-drawing characters and block elements are drawn natively, as in the
  `boxdraw` patch (`BOXDRAW`).
* `Ctrl+Shift+F` searches the scrollback and `Ctrl+Shift+Enter` opens a new
  terminal in the current directory, as in the `newterm` patch.
* Colours are allocated per drawn true-colour/derived attribute exactly as in
  the original (including the original's small Xft colour leak).

## License

MIT. See `LICENSE`; the original copyright holders are retained.

The C original is based on Aurélien APTEL's bt source code.
