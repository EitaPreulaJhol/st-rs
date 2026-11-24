/*
 * xtool - small X11 testing utility used by the Docker test suite.
 *
 * Subcommands:
 *   focus WID
 *   key   WID MODS KEYSYM      MODS is any of c (control), s (shift), a (alt)
 *   type  WID STRING
 *   zoom  WID MODE [N] [MS]    MODE: plus|equal|minus|underscore|kpadd|kpsub|reset
 *   select WID BORDER CW CH C0 R0 C1 R1 PRIMARY_OUT [CLIPBOARD_OUT]
 *
 * XTest is loaded through dlopen so the helper only needs libX11 at link time.
 */
#define _DEFAULT_SOURCE

#include <X11/Xlib.h>
#include <X11/Xatom.h>
#include <X11/keysym.h>

#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

typedef int (*fakekey_t)(Display *, unsigned int, Bool, unsigned long);
typedef int (*fakebutton_t)(Display *, unsigned int, Bool, unsigned long);

static Display *dpy;
static fakekey_t fakekey;
static fakebutton_t fakebutton;

static void die(const char *msg)
{
	fprintf(stderr, "xtool: %s\n", msg);
	exit(1);
}

static void usage(void)
{
	fprintf(stderr,
	        "usage: xtool focus WID\n"
	        "       xtool key WID MODS KEYSYM\n"
	        "       xtool type WID STRING\n"
	        "       xtool zoom WID MODE [N] [MS]\n"
	        "       xtool wheel WID MODS BUTTON [N] [MS]\n"
	        "       xtool select WID BORDER CW CH C0 R0 C1 R1 PRIMARY [CLIPBOARD]\n");
	exit(2);
}

static void load_xtest(void)
{
	void *h = dlopen("libXtst.so.6", RTLD_NOW);
	if (!h)
		die(dlerror());
	fakekey = (fakekey_t)dlsym(h, "XTestFakeKeyEvent");
	fakebutton = (fakebutton_t)dlsym(h, "XTestFakeButtonEvent");
	if (!fakekey || !fakebutton)
		die("XTest extension is unavailable");
}

static void focus(Window w)
{
	XSetInputFocus(dpy, w, RevertToParent, CurrentTime);
	XSync(dpy, False);
	usleep(250000);
}

/* Returns 0 (without failing) when the keycode does not exist. */
static int press(KeySym ks, int shift, int ctrl, int alt, int delay_ms)
{
	KeyCode kc = XKeysymToKeycode(dpy, ks);
	KeyCode sh = XKeysymToKeycode(dpy, XK_Shift_L);
	KeyCode ct = XKeysymToKeycode(dpy, XK_Control_L);
	KeyCode al = XKeysymToKeycode(dpy, XK_Alt_L);

	if (!kc)
		return 0;
	if ((shift && !sh) || (ctrl && !ct) || (alt && !al))
		die("modifier keycode is missing");

	if (ctrl)
		fakekey(dpy, ct, True, 0);
	if (alt)
		fakekey(dpy, al, True, 0);
	if (shift)
		fakekey(dpy, sh, True, 0);
	fakekey(dpy, kc, True, 0);
	fakekey(dpy, kc, False, 0);
	if (shift)
		fakekey(dpy, sh, False, 0);
	if (alt)
		fakekey(dpy, al, False, 0);
	if (ctrl)
		fakekey(dpy, ct, False, 0);
	XSync(dpy, False);
	usleep((useconds_t)delay_ms * 1000);
	return 1;
}

static void parse_mods(const char *mods, int *shift, int *ctrl, int *alt)
{
	*shift = *ctrl = *alt = 0;
	for (; *mods; mods++) {
		switch (*mods) {
		case 's': *shift = 1; break;
		case 'c': *ctrl = 1; break;
		case 'a': *alt = 1; break;
		default:
			fprintf(stderr, "xtool: bad modifier '%c'\n", *mods);
			exit(2);
		}
	}
}

static KeySym char_keysym(unsigned char c, int *shift)
{
	*shift = 0;
	if (c >= 'a' && c <= 'z')
		return XK_a + (c - 'a');
	if (c >= 'A' && c <= 'Z') {
		*shift = 1;
		return XK_A + (c - 'A');
	}
	if (c >= '0' && c <= '9')
		return XK_0 + (c - '0');
	switch (c) {
	case ' ': return XK_space;
	case '\t': return XK_Tab;
	case '.': return XK_period;
	case ',': return XK_comma;
	case '/': return XK_slash;
	case '-': return XK_minus;
	case '=': return XK_equal;
	case ';': return XK_semicolon;
	case '\'': return XK_apostrophe;
	case '[': return XK_bracketleft;
	case ']': return XK_bracketright;
	case '\\': return XK_backslash;
	case '`': return XK_grave;
	default: return NoSymbol;
	}
}

static void type_string(Window w, const char *s)
{
	focus(w);
	for (; *s; s++) {
		unsigned char c = (unsigned char)*s;
		KeySym ks;
		int shift = 0;

		if (c == '\n') {
			if (!press(XK_Return, 0, 0, 0, 20))
				die("Return keycode is missing");
			continue;
		}
		ks = char_keysym(c, &shift);
		if (ks == NoSymbol) {
			fprintf(stderr, "xtool: cannot type byte 0x%02x\n", c);
			exit(1);
		}
		if (!press(ks, shift, 0, 0, 20))
			die("keycode for typed character is missing");
	}
}

static int read_selection(Atom sel, Atom target, const char *outfile)
{
	Window helper = XCreateSimpleWindow(dpy, DefaultRootWindow(dpy),
	                                    0, 0, 1, 1, 0, 0, 0);
	Atom prop = XInternAtom(dpy, "_ST_TEST_SELECTION", False);
	XEvent ev;
	Atom type;
	int fmt, got = 0, i;
	unsigned long nitems, rem;
	unsigned char *data = NULL;
	FILE *f;

	XConvertSelection(dpy, sel, target, prop, helper, CurrentTime);
	XFlush(dpy);

	for (i = 0; i < 300 && !got; i++) {
		while (XPending(dpy)) {
			XNextEvent(dpy, &ev);
			if (ev.type == SelectionNotify) {
				got = 1;
				break;
			}
		}
		if (!got)
			usleep(10000);
	}
	if (!got) {
		XDestroyWindow(dpy, helper);
		return 0;
	}

	if (XGetWindowProperty(dpy, helper, prop, 0, 65536 / 4, True,
	                       AnyPropertyType, &type, &fmt, &nitems, &rem,
	                       &data) != Success || type == None || fmt == 0) {
		if (data)
			XFree(data);
		XDestroyWindow(dpy, helper);
		return 0;
	}

	f = fopen(outfile, "wb");
	if (!f)
		die("cannot open selection output file");
	if (nitems && data)
		fwrite(data, 1, (size_t)nitems * (size_t)(fmt / 8), f);
	fclose(f);
	if (data)
		XFree(data);
	XDestroyWindow(dpy, helper);
	return 1;
}

static void select_text(Window w, int border, int cw, int ch,
                        int c0, int r0, int c1, int r1,
                        const char *primary_out, const char *clip_out)
{
	int rx, ry, x0, y0, x1, y1, i;
	Window child;
	Atom utf8;

	focus(w);
	XTranslateCoordinates(dpy, w, DefaultRootWindow(dpy), 0, 0, &rx, &ry, &child);

	x0 = rx + border + c0 * cw + cw / 2;
	y0 = ry + border + r0 * ch + ch / 2;
	x1 = rx + border + c1 * cw + cw / 2;
	y1 = ry + border + r1 * ch + ch / 2;

	XWarpPointer(dpy, None, DefaultRootWindow(dpy), 0, 0, 0, 0, x0, y0);
	XSync(dpy, False);
	usleep(200000);

	fakebutton(dpy, Button1, True, 0);
	XSync(dpy, False);
	usleep(120000);
	for (i = 1; i <= 8; i++) {
		XWarpPointer(dpy, None, DefaultRootWindow(dpy), 0, 0, 0, 0,
		             x0 + (x1 - x0) * i / 8, y0 + (y1 - y0) * i / 8);
		XSync(dpy, False);
		usleep(40000);
	}
	fakebutton(dpy, Button1, False, 0);
	XSync(dpy, False);
	usleep(300000);

	utf8 = XInternAtom(dpy, "UTF8_STRING", False);
	if (!read_selection(XA_PRIMARY, utf8, primary_out))
		die("no PRIMARY selection was offered");

	if (clip_out && strcmp(clip_out, "-") != 0) {
		if (!press(XK_C, 1, 1, 0, 120))
			die("C keycode is missing");
		usleep(250000);
		if (!read_selection(XInternAtom(dpy, "CLIPBOARD", False), utf8,
		                    clip_out))
			die("no CLIPBOARD selection was offered");
	}
}

static void zoom_mode(Window w, const char *mode, int count, int delay_ms)
{
	KeySym ks;
	int shift = 0, i;

	if (!strcmp(mode, "plus")) {
		ks = XK_plus;
		shift = 1;
	} else if (!strcmp(mode, "equal")) {
		ks = XK_equal;
	} else if (!strcmp(mode, "minus")) {
		ks = XK_minus;
	} else if (!strcmp(mode, "underscore")) {
		ks = XK_underscore;
		shift = 1;
	} else if (!strcmp(mode, "kpadd")) {
		ks = XK_KP_Add;
	} else if (!strcmp(mode, "kpsub")) {
		ks = XK_KP_Subtract;
	} else if (!strcmp(mode, "reset")) {
		ks = XK_0;
	} else {
		die("unknown zoom mode");
	}

	focus(w);
	for (i = 0; i < count; i++) {
		if (!press(ks, shift, 1, 0, delay_ms))
			die("zoom keycode is missing");
	}
}

static void wheel(Window w, const char *mods, unsigned int button, int count,
                  int delay_ms)
{
	int shift, ctrl, alt, i;
	int rx, ry;
	Window child;
	XWindowAttributes attr;
	KeyCode sh = XKeysymToKeycode(dpy, XK_Shift_L);
	KeyCode ct = XKeysymToKeycode(dpy, XK_Control_L);
	KeyCode al = XKeysymToKeycode(dpy, XK_Alt_L);

	parse_mods(mods, &shift, &ctrl, &alt);
	if (button != Button4 && button != Button5)
		die("wheel button must be 4 or 5");

	focus(w);
	if (!XGetWindowAttributes(dpy, w, &attr))
		die("cannot get window attributes");
	XTranslateCoordinates(dpy, w, DefaultRootWindow(dpy), attr.width / 2,
	                      attr.height / 2, &rx, &ry, &child);
	XWarpPointer(dpy, None, DefaultRootWindow(dpy), 0, 0, 0, 0, rx, ry);
	XSync(dpy, False);
	usleep(150000);

	if (ctrl)
		fakekey(dpy, ct, True, 0);
	if (alt)
		fakekey(dpy, al, True, 0);
	if (shift)
		fakekey(dpy, sh, True, 0);
	for (i = 0; i < count; i++) {
		fakebutton(dpy, button, True, 0);
		fakebutton(dpy, button, False, 0);
		XSync(dpy, False);
		usleep((useconds_t)delay_ms * 1000);
	}
	if (shift)
		fakekey(dpy, sh, False, 0);
	if (alt)
		fakekey(dpy, al, False, 0);
	if (ctrl)
		fakekey(dpy, ct, False, 0);
	XSync(dpy, False);
}

int main(int argc, char **argv)
{
	const char *cmd;

	if (argc < 2)
		usage();

	dpy = XOpenDisplay(NULL);
	if (!dpy)
		die("cannot open display");
	load_xtest();

	cmd = argv[1];
	if (!strcmp(cmd, "focus") && argc == 3) {
		focus((Window)strtoul(argv[2], NULL, 0));
	} else if (!strcmp(cmd, "key") && argc == 5) {
		int shift, ctrl, alt;
		KeySym ks;
		Window w = (Window)strtoul(argv[2], NULL, 0);

		parse_mods(argv[3], &shift, &ctrl, &alt);
		ks = XStringToKeysym(argv[4]);
		if (ks == NoSymbol)
			die("unknown keysym");
		focus(w);
		if (!press(ks, shift, ctrl, alt, 80))
			die("keycode is missing");
	} else if (!strcmp(cmd, "type") && argc == 4) {
		type_string((Window)strtoul(argv[2], NULL, 0), argv[3]);
	} else if (!strcmp(cmd, "zoom") && (argc == 4 || argc == 5 || argc == 6)) {
		int count = argc >= 5 ? atoi(argv[4]) : 1;
		int delay_ms = argc >= 6 ? atoi(argv[5]) : 40;
		zoom_mode((Window)strtoul(argv[2], NULL, 0), argv[3], count, delay_ms);
	} else if (!strcmp(cmd, "wheel") && (argc == 5 || argc == 6 || argc == 7)) {
		int count = argc >= 6 ? atoi(argv[5]) : 1;
		int delay_ms = argc >= 7 ? atoi(argv[6]) : 40;
		wheel((Window)strtoul(argv[2], NULL, 0), argv[3],
		      (unsigned int)atoi(argv[4]), count, delay_ms);
	} else if (!strcmp(cmd, "select") && (argc == 11 || argc == 12)) {
		Window w = (Window)strtoul(argv[2], NULL, 0);
		select_text(w, atoi(argv[3]), atoi(argv[4]), atoi(argv[5]),
		            atoi(argv[6]), atoi(argv[7]), atoi(argv[8]), atoi(argv[9]),
		            argv[10], argc == 12 ? argv[11] : "-");
	} else {
		usage();
	}

	XCloseDisplay(dpy);
	return 0;
}
