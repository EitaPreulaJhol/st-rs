//! st - simple terminal (Rust port)

#![allow(non_upper_case_globals, dead_code)]

mod app;
mod config;
mod ffi;
mod search;
mod icon;
mod term;
mod x;

use std::ffi::CString;

fn main() {
    let argv: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let argv0 = argv
        .first()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "st".to_string());

    // Build the application state before parsing so that geometry options can
    // update the window fields in place, exactly like the C globals.
    let mut app = Box::new(app::App::new(x::Options::default()));
    app.xsetcursor(config::CURSORSTYLE as i32);

    let parsed = x::parse_args(&argv, &mut app, &argv0);
    app.opts = parsed.opts;

    let cols = parsed.cols.max(1) as i32;
    let rows = parsed.rows.max(1) as i32;

    if app.opts.title.is_none() {
        let t = if app.opts.line.is_some() || app.opts.cmd.is_empty() {
            "st".to_string()
        } else {
            app.opts.cmd[0].to_string_lossy().into_owned()
        };
        app.opts.title = Some(CString::new(t).unwrap());
    }

    unsafe {
        libc::setlocale(libc::LC_CTYPE, c"".as_ptr());
        x11::xlib::XSetLocaleModifiers(c"".as_ptr());
    }

    // Stable address for XIM destroy callbacks.
    let app: &'static mut app::App = Box::leak(app);
    app::APP.store(app as *mut app::App, std::sync::atomic::Ordering::Relaxed);

    app.tnew(cols, rows);
    app.xinit(cols, rows);
    app.xsetenv();
    app.selinit();
    app.run();
}
