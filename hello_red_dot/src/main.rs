use std::error::Error;
use std::thread;
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::shape::{ConnectionExt as ShapeExt, SK, SO};
use x11rb::protocol::xproto::{
    ClipOrdering, ConfigureWindowAux, ConnectionExt, CreateWindowAux, Rectangle, StackMode,
    WindowClass,
};

const DOT_SIZE: u16 = 24; // dot diameter in pixels
const POLL_MS: u64 = 4;   // pointer polling interval
const DISPLAY_MS: u64 = 1_000; // mouse position display interval

fn create_dot_window<C: Connection>(
    conn: &C,
    root: u32,
    screen_root_depth: u8,
    screen_root_visual: u32,
    color: u32,
) -> Result<u32, Box<dyn Error>> {
    let win = conn.generate_id()?;
    conn.create_window(
        screen_root_depth,
        win,
        root,
        0,
        0,
        DOT_SIZE,
        DOT_SIZE,
        0,
        WindowClass::INPUT_OUTPUT,
        screen_root_visual,
        &CreateWindowAux::new()
            .background_pixel(color)
            .override_redirect(1),
    )?;

    // Clip the square window into a circle using the SHAPE extension.
    let r = f64::from(DOT_SIZE) / 2.0;
    let rows: Vec<Rectangle> = (0..DOT_SIZE)
        .map(|row| {
            let dy = f64::from(row) + 0.5 - r;
            let dx = (r * r - dy * dy).sqrt();
            let x0 = (r - dx).floor() as i16;
            let x1 = ((r + dx).ceil() as i16).min(DOT_SIZE as i16);
            Rectangle { x: x0, y: row as i16, width: (x1 - x0) as u16, height: 1 }
        })
        .collect();
    conn.shape_rectangles(SO::SET, SK::BOUNDING, ClipOrdering::UNSORTED, win, 0, 0, &rows)?;

    // Empty input shape -> clicks pass straight through the dot.
    let empty: [Rectangle; 0] = [];
    conn.shape_rectangles(SO::SET, SK::INPUT, ClipOrdering::UNSORTED, win, 0, 0, &empty)?;

    // The window must be mapped, otherwise it is never shown and moving it
    // has no visible effect (which looks like "the dot isn't tracking").
    conn.map_window(win)?;

    Ok(win)
}

fn main() -> Result<(), Box<dyn Error>> {
    // In a Wayland session, DISPLAY may point to XWayland. Try it rather than rejecting the
    // session outright, but XWayland cannot track the compositor cursor over native Wayland
    // surfaces or place this window above them.
    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;

    // Detect up front whether we are likely running under XWayland, so the user gets an
    // accurate diagnostic immediately instead of only after 3s of a stuck pointer.
    let is_wayland_session = std::env::var("WAYLAND_DISPLAY").is_ok()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|v| v.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false);
    if is_wayland_session {
        eprintln!(
            "Note: detected a Wayland session (WAYLAND_DISPLAY/XDG_SESSION_TYPE). This program \
             is connecting via XWayland. XWayland only forwards pointer motion to the X11 root \
             window while the cursor is over an X11 (Xwayland) window; it cannot see the cursor \
             over native Wayland surfaces. If the dot does not track your mouse, move the mouse \
             over an X11 application window, or run this program under a native X11 session."
        );
    }

    // Create multiple red dots for different mouse positions in Wayland compositor
    let win_primary = create_dot_window(&conn, root, screen.root_depth, screen.root_visual, 0xFF0000)?;
    let win_secondary = create_dot_window(&conn, root, screen.root_depth, screen.root_visual, 0xFF6666)?;
    let win_tertiary = create_dot_window(&conn, root, screen.root_depth, screen.root_visual, 0xFF9999)?;

    // 4) Poll the pointer and move the dots.
    let mut last_primary: Option<(i32, i32)> = None;
    let mut last_secondary: Option<(i32, i32)> = None;
    let mut last_tertiary: Option<(i32, i32)> = None;
    let mut last_display = std::time::Instant::now();
    let mut last_seen_pos: Option<(i16, i16)> = None;
    let mut unchanged_since = std::time::Instant::now();
    let mut warned_stuck = false;

    loop {
        // Always query the pointer relative to the root window so coordinates stay
        // consistent regardless of which window currently has focus/is under the cursor.
        let p = conn.query_pointer(root)?.reply()?;
        if p.same_screen == false {
            // Pointer is on a different screen; skip this update rather than drawing
            // the dot at a stale/incorrect position.
            thread::sleep(Duration::from_millis(POLL_MS));
            continue;
        }
        if last_display.elapsed() >= Duration::from_millis(DISPLAY_MS) {
            println!("Mouse position: ({}, {})", p.root_x, p.root_y);
            last_display = std::time::Instant::now();
        }

        // Detect the common case where we are running under XWayland and the
        // compositor does not forward real cursor motion to the X11 root
        // pointer (this happens when the cursor stays over native Wayland
        // surfaces). In that situation `query_pointer` reports the same
        // coordinates forever, which looks like "the dot isn't moving" even
        // though this program is working correctly.
        let current_pos = (p.root_x, p.root_y);
        if last_seen_pos != Some(current_pos) {
            last_seen_pos = Some(current_pos);
            unchanged_since = std::time::Instant::now();
            warned_stuck = false;
        } else if !warned_stuck && unchanged_since.elapsed() >= Duration::from_secs(3) {
            eprintln!(
                "Warning: pointer position has not changed for 3s. If you are moving the \
                 mouse, this likely means you're running under XWayland and the compositor \
                 cursor is over a native Wayland surface, which XWayland cannot see. Try \
                 moving the mouse over an X11 window, or run this program under a native X11 \
                 session."
            );
            warned_stuck = true;
        }

        // Primary dot - exact pointer position
        let x = i32::from(p.root_x) - i32::from(DOT_SIZE) / 2;
        let y = i32::from(p.root_y) - i32::from(DOT_SIZE) / 2;

        if last_primary != Some((x, y)) {
            conn.configure_window(
                win_primary,
                &ConfigureWindowAux::new().x(x).y(y).stack_mode(StackMode::ABOVE),
            )?
            .check()?;
            conn.flush()?;
            last_primary = Some((x, y));
        }

        // Secondary dot - offset for multi-pointer awareness
        let x2 = x + 30;
        let y2 = y;
        if last_secondary != Some((x2, y2)) {
            conn.configure_window(
                win_secondary,
                &ConfigureWindowAux::new().x(x2).y(y2).stack_mode(StackMode::ABOVE),
            )?
            .check()?;
            conn.flush()?;
            last_secondary = Some((x2, y2));
        }

        // Tertiary dot - different offset for additional pointer
        let x3 = x;
        let y3 = y + 30;
        if last_tertiary != Some((x3, y3)) {
            conn.configure_window(
                win_tertiary,
                &ConfigureWindowAux::new().x(x3).y(y3).stack_mode(StackMode::ABOVE),
            )?
            .check()?;
            conn.flush()?;
            last_tertiary = Some((x3, y3));
        }

        thread::sleep(Duration::from_millis(POLL_MS));
    }
}