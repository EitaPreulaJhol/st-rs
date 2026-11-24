#!/usr/bin/env bash
#
# End-to-end test suite for the Rust st port.
#
# Runs headless inside the Docker test image: unit tests, rendering smoke
# checks, configured hotkeys, scrollback, selection/clipboard/bracketed paste,
# runtime zoom and custom per-style fonts. Xvfb provides a private display;
# nothing is shown on the host desktop.

set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$ROOT"

TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target}
BIN="$TARGET_DIR/release/st"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/st-tests.XXXXXX")
export DISPLAY=${DISPLAY:-:99}
export PYTHONDONTWRITEBYTECODE=1

SHOTS="$WORK/shots"
HELPER_DIR="$WORK/bin"
mkdir -p "$SHOTS" "$HELPER_DIR"
XTOOL="$HELPER_DIR/xtool"
XVFB_PID=""

fail() {
	printf '\nFAIL: %s\n' "$*" >&2
	exit 1
}

step() {
	printf '\n\033[1m== %s ==\033[0m\n' "$*"
}

cleanup() {
	set +e
	if [ -n "$XVFB_PID" ]; then
		kill "$XVFB_PID" 2>/dev/null
		wait "$XVFB_PID" 2>/dev/null
	fi
	pkill -f "$BIN" 2>/dev/null
	rm -rf "$WORK"
}
trap cleanup EXIT

wait_file() {
	local f=$1
	for _ in $(seq 1 200); do
		[ -s "$f" ] && return 0
		sleep 0.05
	done
	return 1
}

# ---------------------------------------------------------------------------
# Private X display
# ---------------------------------------------------------------------------

step "starting Xvfb on $DISPLAY"
Xvfb "$DISPLAY" -screen 0 1280x800x24 -nolisten tcp >"$WORK/xvfb.log" 2>&1 &
XVFB_PID=$!
for _ in $(seq 1 100); do
	xdpyinfo -display "$DISPLAY" >/dev/null 2>&1 && break
	sleep 0.1
done
if ! xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then
	cat "$WORK/xvfb.log"
	fail "Xvfb did not start"
fi
echo "display $DISPLAY is ready"

step "building X test helper"
cc -O2 -Wall -Wextra -o "$XTOOL" tests/helpers/xtool.c -lX11 -ldl

# ---------------------------------------------------------------------------
# Unit tests and builds
# ---------------------------------------------------------------------------

step "Rust unit tests"
cargo test --release --locked

step "building st"
cargo build --release --locked
[ -x "$BIN" ] || fail "missing $BIN"

# ---------------------------------------------------------------------------
# Rendering smoke checks
# ---------------------------------------------------------------------------

render_run() { # TAG FONT ZOOM
	local tag=$1 font=$2 zoom=$3
	local widf="$WORK/wid_$tag" errf="$WORK/err_$tag" xwdf="$SHOTS/$tag.xwd"
	local args=(-A 1)
	rm -f "$widf" "$errf" "$xwdf"
	if [ -n "$font" ]; then
		args+=(-f "$font")
	fi
	"$BIN" -g 80x24 "${args[@]}" -e sh tests/docker/render.sh "$WORK" "$tag" 4 >/dev/null 2>"$errf" &
	local pid=$!
	wait_file "$widf" || { cat "$errf"; fail "render did not start ($tag)"; }
	local wid
	wid=$(cat "$widf")
	"$XTOOL" focus "$wid"
	sleep 0.6
	if [ "$zoom" = yes ]; then
		"$XTOOL" key "$wid" cs Prior
		sleep 1.0
	fi
	xwd -id "$wid" -silent -out "$xwdf"
	kill "$pid" 2>/dev/null || true
	wait "$pid" 2>/dev/null || true
	[ -s "$xwdf" ] || fail "no screenshot ($tag)"
	if [ -s "$errf" ]; then
		cat "$errf"
		fail "stderr from render test ($tag)"
	fi
}

step "configured font renders"
# Uses whatever FONT the source configuration specifies; if that family is not
# installed, fontconfig substitutes one.
render_run configured "" no

DEFAULT_FONT="Liberation Mono:pixelsize=12:antialias=true:autohint=true"
step "rendering with an explicit font"
render_run default "$DEFAULT_FONT" no

step "rendering after font zoom (Ctrl+Shift+PageUp)"
render_run zoom "$DEFAULT_FONT" yes

# ---------------------------------------------------------------------------
# Background opacity
# ---------------------------------------------------------------------------

alpha_run() { # TAG EXTRA WANT_ALPHA WANT_RGB [OUTPUT] [X] [Y]
	local tag=$1 extra=$2 want_alpha=$3 want_rgb=$4 output=${5:-bg opacity}
	local px=${6:-1} py=${7:-1}
	local widf="$WORK/wid_alpha_$tag" errf="$WORK/err_alpha_$tag"
	local xwdf="$SHOTS/alpha_$tag.xwd"
	rm -f "$widf" "$errf" "$xwdf"
	"$BIN" -g 40x10 -f "$DEFAULT_FONT" $extra \
		-e sh -c "echo \$WINDOWID > '$widf'; printf '%b' '$output'; sleep 30" \
		>/dev/null 2>"$errf" &
	local pid=$!
	wait_file "$widf" || { cat "$errf"; fail "alpha test did not start ($tag)"; }
	local wid
	wid=$(cat "$widf")
	"$XTOOL" focus "$wid"
	sleep 0.6
	xwininfo -id "$wid" | grep -q "Depth: 32" ||
		fail "window is not 32-bit ARGB ($tag)"
	xwd -id "$wid" -silent -out "$xwdf"
	local r g b alpha rgb
	read -r r g b alpha <<<"$(python3 tests/docker/xwd.py pixel "$xwdf" "$px" "$py")"
	rgb="$r $g $b"
	[ "$rgb" = "$want_rgb" ] || fail "bg rgb '$rgb', expected '$want_rgb' ($tag)"
	[ "$alpha" = "$want_alpha" ] || fail "bg alpha $alpha, expected $want_alpha ($tag)"
	printf 'ok: %s alpha %s, rgb %s\n' "$tag" "$alpha" "$rgb"
	kill "$pid" 2>/dev/null || true
	wait "$pid" 2>/dev/null || true
}

step "background opacity and gradient (-A and OSC 11)"
# Border pixels keep the plain opacity, cell backgrounds use the gradient.
alpha_run default "" 204 "255 255 255"
alpha_run half "-A 0.5" 127 "255 255 255"
alpha_run opaque "-A 1" 255 "255 255 255"
alpha_run transparent "-A 0" 0 "255 255 255"
# OSC 11 background changes keep the configured opacity.
alpha_run osc11 "" 204 "255 0 0" '\033]11;rgb:ff/00/00\007'
alpha_run osc11-grad "" 135 "255 0 0" '\033]11;rgb:ff/00/00\007' 250 140
# The gradient is opaque at the top and fades towards the bottom.
alpha_run grad-top "" 255 "255 255 255" "bg opacity" 250 9
alpha_run grad-bottom "" 135 "255 255 255" "bg opacity" 250 140
echo "background opacity applies to -A, OSC 11 and the vertical gradient"

# ---------------------------------------------------------------------------
# Configured hotkeys
# ---------------------------------------------------------------------------

step "configured hotkeys"
HOTKEY_WIDF="$WORK/wid_hotkeys"
HOTKEY_OUT="$WORK/hotkeys"
HOTKEY_ERR="$WORK/hotkeys_err"
rm -f "$HOTKEY_WIDF" "$HOTKEY_OUT" "$HOTKEY_ERR"
"$BIN" -g 80x24 -e sh -c "echo \$WINDOWID > '$HOTKEY_WIDF'; stty raw -echo; cat > '$HOTKEY_OUT'" >/dev/null 2>"$HOTKEY_ERR" &
HOTKEY_PID=$!
wait_file "$HOTKEY_WIDF" || { cat "$HOTKEY_ERR"; fail "hotkey test did not start"; }
HOTKEY_WID=$(cat "$HOTKEY_WIDF")
"$XTOOL" focus "$HOTKEY_WID"
sleep 0.4
"$XTOOL" key "$HOTKEY_WID" s Return
last=-1 size=0 stable=0
for _ in $(seq 1 60); do
	size=$(stat -c %s "$HOTKEY_OUT" 2>/dev/null || echo 0)
	if [ "$size" = "$last" ]; then
		stable=$((stable + 1))
		[ "$stable" -ge 4 ] && break
	else
		stable=0
		last=$size
	fi
	sleep 0.05
done
kill "$HOTKEY_PID" 2>/dev/null || true
wait "$HOTKEY_PID" 2>/dev/null || true
[ ! -s "$HOTKEY_ERR" ] || { cat "$HOTKEY_ERR"; fail "stderr from hotkey test"; }
printf '\033\r' >"$WORK/hotkeys.expected"
cmp -s "$HOTKEY_OUT" "$WORK/hotkeys.expected" || {
	echo "--- hotkey output ---"
	od -c "$HOTKEY_OUT"
	fail "configured hotkeys sent unexpected bytes"
}
echo "Shift-Enter sends the configured sequence"

# ---------------------------------------------------------------------------
# Scrollback
# ---------------------------------------------------------------------------

scrollback_run() { # TAG SECOND_MOD SECOND_KEY WANT
	local tag=$1 m2=$2 k2=$3 want=$4
	local widf="$WORK/wid_scroll_$tag" readyf="$WORK/ready_scroll_$tag"
	local dump="$WORK/scroll_$tag" errf="$WORK/scroll_err_$tag"
	rm -f "$widf" "$readyf" "$dump" "$errf"
	"$BIN" -g 20x5 -o "$dump" -e sh -c \
		"echo \$WINDOWID > '$widf'; i=1; while [ \$i -le 20 ]; do printf 'line%02d\\r\\n' \$i; i=\$((i+1)); done; echo ready > '$readyf'; sleep 30" \
		>/dev/null 2>"$errf" &
	local pid=$!
	wait_file "$widf" || { cat "$errf"; fail "scrollback test did not start ($tag)"; }
	wait_file "$readyf" || { cat "$errf"; fail "scrollback output did not finish ($tag)"; }
	local wid
	wid=$(cat "$widf")
	"$XTOOL" focus "$wid"
	sleep 0.3
	# With -o the terminal output is printed too; remember where it ends so
	# only the Shift-PrintScreen view is examined.
	local base
	base=$(stat -c %s "$dump")
	"$XTOOL" key "$wid" c Home
	if [ -n "$m2" ]; then
		sleep 0.3
		"$XTOOL" key "$wid" "$m2" "$k2"
	fi
	sleep 0.3
	"$XTOOL" key "$wid" s Print
	for _ in $(seq 1 60); do
		[ "$(stat -c %s "$dump" 2>/dev/null || echo "$base")" -gt "$base" ] && break
		sleep 0.05
	done
	kill "$pid" 2>/dev/null || true
	wait "$pid" 2>/dev/null || true
	[ ! -s "$errf" ] || { cat "$errf"; fail "stderr from scrollback test ($tag)"; }
	tail -c +$((base + 1)) "$dump" >"$WORK/scroll_view_$tag"
	grep -q "$want" "$WORK/scroll_view_$tag" || {
		echo "--- scrollback dump ($tag) ---"
		cat "$WORK/scroll_view_$tag"
		fail "expected '$want' in the scrollback view ($tag)"
	}
}

step "scrollback: Ctrl+Home and Ctrl+End"
scrollback_run top "" "" "line01"
scrollback_run bottom c End "line20"
echo "scrollback history is reachable and Ctrl+End returns to the live screen"

# ---------------------------------------------------------------------------
# Selection, clipboard, bracketed paste
# ---------------------------------------------------------------------------

selection_test() {
	local bin=$1 tag=$2
	local widf="$WORK/wid_sel_$tag" sizef="$WORK/size_sel_$tag"
	local pastef="$WORK/paste_$tag" errf="$WORK/sel_err_$tag"
	local primf="$WORK/primary_$tag" clipf="$WORK/clipboard_$tag"
	rm -f "$widf" "$sizef" "$pastef" "$errf" "$primf" "$clipf"
	"$bin" -g 80x24 -e sh -c "echo \$WINDOWID > '$widf'; stty raw -echo; sleep 0.3; stty size > '$sizef'; printf '\033[?2004h'; printf 'hello world'; cat > '$pastef'" >/dev/null 2>"$errf" &
	local pid=$!
	wait_file "$widf" || { cat "$errf"; fail "selection test did not start ($tag)"; }
	wait_file "$sizef" || fail "no tty size ($tag)"
	local wid
	wid=$(cat "$widf")
	local rows cols
	read -r rows cols <"$sizef"
	[ -n "${rows:-}" ] && [ -n "${cols:-}" ] || fail "bad tty size ($tag)"
	local geom w h
	geom=$(xwininfo -id "$wid")
	w=$(printf '%s\n' "$geom" | awk '/Width:/ {print $2}')
	h=$(printf '%s\n' "$geom" | awk '/Height:/ {print $2}')
	[ -n "$w" ] && [ -n "$h" ] || fail "cannot read window geometry ($tag)"
	local cw=$(((w - 4) / cols)) ch=$(((h - 4) / rows))
	[ "$cw" -gt 0 ] && [ "$ch" -gt 0 ] || fail "bad cell size ($tag)"
	"$XTOOL" focus "$wid"
	sleep 0.5
	"$XTOOL" select "$wid" 2 "$cw" "$ch" 0 0 4 0 "$primf" "$clipf"
	[ "$(cat "$primf")" = "hello" ] || fail "PRIMARY selection ($tag)"
	[ "$(cat "$clipf")" = "hello" ] || fail "CLIPBOARD selection ($tag)"
	"$XTOOL" key "$wid" cs v
	sleep 0.8
	kill "$pid" 2>/dev/null || true
	wait "$pid" 2>/dev/null || true
	printf '\033[200~hello\033[201~' >"$WORK/expected_paste"
	cmp -s "$pastef" "$WORK/expected_paste" || {
		od -c "$pastef"
		fail "bracketed paste ($tag)"
	}
}

step "mouse selection, PRIMARY/CLIPBOARD copy and bracketed paste"
selection_test "$BIN" r
echo "selection and bracketed paste work"

# ---------------------------------------------------------------------------
# Runtime zoom
# ---------------------------------------------------------------------------

zoom_test() {
	local bin=$1 tag=$2
	local widf="$WORK/wid_zoom_$tag" logf="$WORK/zoom_$tag" errf="$WORK/zoom_err_$tag"
	rm -f "$widf" "$logf" "$errf"
	"$bin" -g 80x24 -e sh tests/docker/zoom.sh "$WORK" "$tag" >/dev/null 2>"$errf" &
	local pid=$!
	wait_file "$widf" || { cat "$errf"; fail "zoom test did not start ($tag)"; }
	local wid
	wid=$(cat "$widf")
	"$XTOOL" focus "$wid"
	sleep 1.0
	local base
	base=$(tail -n 1 "$logf")
	[ -n "$base" ] || fail "no initial terminal size"

	"$XTOOL" zoom "$wid" plus 2 40
	sleep 0.8
	[ "$(tail -n 1 "$logf")" != "$base" ] || fail "Ctrl+Plus did not resize"

	"$XTOOL" zoom "$wid" reset
	sleep 0.8
	[ "$(tail -n 1 "$logf")" = "$base" ] || fail "Ctrl+0 did not restore the size"

	"$XTOOL" zoom "$wid" equal 1 40
	sleep 0.8
	[ "$(tail -n 1 "$logf")" != "$base" ] || fail "Ctrl+= did not resize"

	"$XTOOL" zoom "$wid" reset
	sleep 0.8
	"$XTOOL" zoom "$wid" underscore 1 40
	sleep 0.8
	[ "$(tail -n 1 "$logf")" != "$base" ] || fail "Ctrl+Shift+- did not resize"

	"$XTOOL" zoom "$wid" reset
	sleep 0.8
	"$XTOOL" zoom "$wid" plus 90 20
	sleep 3
	local maxsz
	maxsz=$(tail -n 1 "$logf")
	"$XTOOL" zoom "$wid" plus 1 40
	sleep 0.8
	[ "$(tail -n 1 "$logf")" = "$maxsz" ] || fail "maximum font size did not clamp"

	"$XTOOL" zoom "$wid" minus 120 20
	sleep 3
	local minsz
	minsz=$(tail -n 1 "$logf")
	"$XTOOL" zoom "$wid" minus 1 40
	sleep 0.8
	[ "$(tail -n 1 "$logf")" = "$minsz" ] || fail "minimum font size did not clamp"

	"$XTOOL" zoom "$wid" reset
	sleep 0.8
	[ "$(tail -n 1 "$logf")" = "$base" ] || fail "final reset did not restore the size"

	kill "$pid" 2>/dev/null || true
	wait "$pid" 2>/dev/null || true
	[ ! -s "$errf" ] || { cat "$errf"; fail "stderr from zoom test"; }
}

step "runtime font zoom: Ctrl+Plus/Minus/0, limits and reset"
zoom_test "$BIN" zoom

# ---------------------------------------------------------------------------
# Custom per-style fonts
# ---------------------------------------------------------------------------

step "custom per-style fonts and zoom"
VARIANT_TARGET="$WORK/variant-target"

python3_patch_config() { # path bold italic bolditalic
	python3 - "$@" <<'PYEOF'
import re
import sys

path, bold, italic, bi = sys.argv[1:5]
text = open(path).read()
value = 'Liberation Mono:pixelsize=12:antialias=true:autohint=true'

def set_const(text, name, val):
    pat = re.compile(r'^pub const %s: &str = .*;$' % name, re.M)
    if not pat.search(text):
        raise SystemExit(f"missing {name} in {path}")
    return pat.sub(lambda _: 'pub const %s: &str = "%s";' % (name, val), text)

text = set_const(text, 'FONT', value)
text = set_const(text, 'FONT_BOLD', bold)
text = set_const(text, 'FONT_ITALIC', italic)
text = set_const(text, 'FONT_BOLD_ITALIC', bi)
open(path, 'w').write(text)
PYEOF
}

build_variant() { # name bold italic bolditalic
	local name=$1 bold=$2 italic=$3 bi=$4
	local src="$WORK/variant-$name-src"
	rm -rf "$src"
	mkdir -p "$src"
	cp -R src Cargo.toml Cargo.lock "$src"/
	python3_patch_config "$src/src/config.rs" "$bold" "$italic" "$bi"
	(
		cd "$src"
		CARGO_TARGET_DIR="$VARIANT_TARGET" cargo build --release --locked >/dev/null
	) || fail "variant $name build failed"
	cp "$VARIANT_TARGET/release/st" "$WORK/bin/st-$name"
}

build_variant derived "" "" ""
build_variant custom \
	"DejaVu Sans Mono:style=Bold" \
	"DejaVu Sans Mono:style=Oblique" \
	"DejaVu Sans Mono:style=Bold Oblique"

font_run() {
	local bin=$1 tag=$2
	local widf="$WORK/wid_font_$tag" logf="$WORK/fontsize_$tag" errf="$WORK/font_err_$tag"
	local xwdf="$SHOTS/font_$tag.xwd"
	rm -f "$widf" "$logf" "$errf" "$xwdf"
	"$bin" -g 80x24 -e sh tests/docker/font_test.sh "$WORK" "$tag" >/dev/null 2>"$errf" &
	local pid=$!
	wait_file "$widf" || { cat "$errf"; fail "font test did not start ($tag)"; }
	local wid
	wid=$(cat "$widf")
	"$XTOOL" focus "$wid"
	sleep 0.8
	xwd -id "$wid" -silent -out "$xwdf"
	local base
	base=$(tail -n 1 "$logf")
	[ -n "$base" ] || fail "no initial size ($tag)"

	"$XTOOL" zoom "$wid" plus 3 50
	sleep 1.2
	[ "$(tail -n 1 "$logf")" != "$base" ] || fail "font zoom did not resize ($tag)"
	"$XTOOL" zoom "$wid" reset
	sleep 1.0
	[ "$(tail -n 1 "$logf")" = "$base" ] || fail "font zoom reset failed ($tag)"

	kill "$pid" 2>/dev/null || true
	wait "$pid" 2>/dev/null || true
	[ ! -s "$errf" ] || { cat "$errf"; fail "stderr from font test ($tag)"; }
}

font_run "$WORK/bin/st-derived" derived
font_run "$WORK/bin/st-custom" custom
python3 tests/docker/xwd.py different "$SHOTS/font_derived.xwd" "$SHOTS/font_custom.xwd" 100 ||
	fail "custom fonts did not change bold/italic rendering"
echo "custom fonts render differently from derived styles and survive zoom"

step "all tests passed"
