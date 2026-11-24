#!/usr/bin/env bash
#
# Full test suite for the Rust st port.
#
# Runs headless inside the Docker test image: unit tests, differential fuzzing
# against the C original, pixel-identical rendering checks, keyboard mapping,
# selection/clipboard/bracketed paste, runtime zoom and custom per-style fonts.
# Xvfb provides a private display; nothing is shown on the host desktop.

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

step "building the C reference implementation"
C_REF="$WORK/st-c"
cp -R original_source "$C_REF"
(
	cd "$C_REF"
	make clean >/dev/null 2>&1 || true
	make -j"$(nproc)" >/dev/null
) || fail "C reference build failed"
CST="$C_REF/st"
[ -x "$CST" ] || fail "missing C reference binary"

# ---------------------------------------------------------------------------
# Command-line compatibility
# ---------------------------------------------------------------------------

step "command-line compatibility"
mkdir -p "$WORK/cli-r" "$WORK/cli-c"
cp "$BIN" "$WORK/cli-r/st"
cp "$CST" "$WORK/cli-c/st"
for opt in -v -Z; do
	( cd "$WORK/cli-r" && ./st "$opt" >"$WORK/cli.stdout" 2>"$WORK/cli.stderr" ) || true
	( cd "$WORK/cli-c" && ./st "$opt" >"$WORK/cli-c.stdout" 2>"$WORK/cli-c.stderr" ) || true
	cmp -s "$WORK/cli.stdout" "$WORK/cli-c.stdout" || fail "stdout differs for $opt"
	cmp -s "$WORK/cli.stderr" "$WORK/cli-c.stderr" || fail "stderr differs for $opt"
done
echo "-v and usage output are identical"

# ---------------------------------------------------------------------------
# Differential fuzzing against the C original
# ---------------------------------------------------------------------------

step "differential escape-sequence fuzzing"
FUZZ_CASES=${ST_FUZZ_CASES:-80}
FUZZ_SEED=${ST_FUZZ_SEED:-0xC0FFEE}
python3 tests/docker/difftest.py "$BIN" "$CST" "$WORK/fuzz" "$FUZZ_CASES" "$FUZZ_SEED"

# ---------------------------------------------------------------------------
# Pixel-identical rendering
# ---------------------------------------------------------------------------

render_run() {
	local bin=$1 tag=$2 font=$3 zoom=$4
	local widf="$WORK/wid_$tag" errf="$WORK/err_$tag" xwdf="$SHOTS/$tag.xwd"
	local args=()
	rm -f "$widf" "$errf" "$xwdf"
	if [ "$bin" = "$BIN" ]; then
		# The Rust build defaults to a translucent background (the alpha
		# patch); the C reference does not know -A. Compare text rendering
		# with opacity disabled; the alpha behaviour has its own test.
		args+=(-A 1)
	fi
	if [ -n "$font" ]; then
		args+=(-f "$font")
	fi
	"$bin" -g 80x24 "${args[@]}" -e sh tests/docker/render.sh "$WORK" "$tag" 4 >/dev/null 2>"$errf" &
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

step "configured font starts"
# Uses whatever FONT the source configuration specifies; if that family is not
# installed, fontconfig substitutes one. No cross-implementation comparison is
# made here because the C reference has its own config.h.
render_run "$BIN" configured "" no

DEFAULT_FONT="Liberation Mono:pixelsize=12:antialias=true:autohint=true"
step "pixel comparison: Liberation Mono 12"
render_run "$BIN" default_r "$DEFAULT_FONT" no
render_run "$CST" default_c "$DEFAULT_FONT" no
python3 tests/docker/xwd.py compare "$SHOTS/default_r.xwd" "$SHOTS/default_c.xwd" ||
	fail "default-font rendering differs"

step "pixel comparison: DejaVu Sans Mono 16"
render_run "$BIN" dejavu_r "DejaVu Sans Mono:pixelsize=16" no
render_run "$CST" dejavu_c "DejaVu Sans Mono:pixelsize=16" no
python3 tests/docker/xwd.py compare "$SHOTS/dejavu_r.xwd" "$SHOTS/dejavu_c.xwd" ||
	fail "DejaVu rendering differs"

step "pixel comparison: font zoom (Ctrl+Shift+PageUp)"
render_run "$BIN" zoom_r "$DEFAULT_FONT" yes
render_run "$CST" zoom_c "$DEFAULT_FONT" yes
python3 tests/docker/xwd.py compare "$SHOTS/zoom_r.xwd" "$SHOTS/zoom_c.xwd" ||
	fail "zoomed rendering differs"

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
# Keyboard mapping
# ---------------------------------------------------------------------------

key_test() {
	local bin=$1 tag=$2 app=$3
	local widf="$WORK/wid_keys_$tag" outf="$WORK/keys_$tag" errf="$WORK/keys_err_$tag"
	rm -f "$widf" "$outf" "$errf"
	"$bin" -g 80x24 -e sh -c "echo \$WINDOWID > '$widf'; stty raw -echo; printf '$app'; cat > '$outf'" >/dev/null 2>"$errf" &
	local pid=$!
	wait_file "$widf" || { cat "$errf"; fail "key test did not start ($tag)"; }
	local wid
	wid=$(cat "$widf")
	"$XTOOL" focus "$wid"
	sleep 0.4
	"$XTOOL" keys "$wid" mapping
	# Wait until st has processed the queued events and cat's output has been
	# stable for about a second.
	local last=-1 size stable=0
	for _ in $(seq 1 60); do
		size=$(stat -c %s "$outf" 2>/dev/null || echo 0)
		if [ "$size" = "$last" ]; then
			stable=$((stable + 1))
			[ "$stable" -ge 4 ] && break
		else
			stable=0
			last=$size
		fi
		sleep 0.25
	done
	kill "$pid" 2>/dev/null || true
	wait "$pid" 2>/dev/null || true
	[ -s "$outf" ] || { cat "$errf"; fail "no keys captured ($tag)"; }
}

step "keyboard mapping: normal mode"
key_test "$BIN" norm_r ""
key_test "$CST" norm_c ""
if ! cmp -s "$WORK/keys_norm_r" "$WORK/keys_norm_c"; then
	echo "--- Rust output ---"
	od -c "$WORK/keys_norm_r" | head -20
	echo "--- C output ---"
	od -c "$WORK/keys_norm_c" | head -20
	echo "--- byte differences ---"
	cmp -l "$WORK/keys_norm_r" "$WORK/keys_norm_c" | head -20
	fail "normal key output differs"
fi
echo "normal keyboard output is byte-identical"

step "keyboard mapping: application cursor/keypad modes"
key_test "$BIN" app_r '\033[?1h\033='
key_test "$CST" app_c '\033[?1h\033='
if ! cmp -s "$WORK/keys_app_r" "$WORK/keys_app_c"; then
	echo "--- Rust output ---"
	od -c "$WORK/keys_app_r" | head -20
	echo "--- C output ---"
	od -c "$WORK/keys_app_c" | head -20
	echo "--- byte differences ---"
	cmp -l "$WORK/keys_app_r" "$WORK/keys_app_c" | head -20
	fail "application key output differs"
fi
echo "application mode keyboard output is byte-identical"

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
selection_test "$CST" c
cmp -s "$WORK/primary_r" "$WORK/primary_c" || fail "PRIMARY differs between implementations"
cmp -s "$WORK/clipboard_r" "$WORK/clipboard_c" || fail "CLIPBOARD differs between implementations"
cmp -s "$WORK/paste_r" "$WORK/paste_c" || fail "paste differs between implementations"
echo "selection and paste match the C reference"

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
