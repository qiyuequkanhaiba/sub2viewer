//! Make the HUD window truly transparent so only the CSS capsule is visible,
//! and shape the panel into a Dynamic-Island-style overlay:
//! raised window level, presence on all Spaces and over fullscreen apps,
//! notch geometry lookup, and cursor click-through outside the capsule.
//!
//! Window behaviour values mirror boring.notch (read as parameters, not
//! copied code): level = NSMainMenuWindowLevel + 3, collectionBehavior =
//! fullScreenAuxiliary | stationary | canJoinAllSpaces | ignoresCycle.

use std::sync::Mutex;
use tauri::{webview::Color, Manager};

#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotchGeometry {
    pub has_notch: bool,
    pub width: f64,
    pub height: f64,
}

/// Used when the screen reports no notch (or off macOS): a 185x32
/// virtual island, matching boring.notch's defaults.
pub const FALLBACK_GEOMETRY: NotchGeometry = NotchGeometry {
    has_notch: false,
    width: 185.0,
    height: 32.0,
};

pub fn apply_clear(app: &tauri::AppHandle) {
    let Some(panel) = app.get_webview_window("panel") else {
        return;
    };
    let _ = panel.set_background_color(Some(Color(0, 0, 0, 0)));
    let _ = panel.with_webview(|wv| {
        #[cfg(target_os = "macos")]
        unsafe {
            clear_wkwebview(wv.inner());
        }
        #[cfg(not(target_os = "macos"))]
        let _ = wv;
    });
    #[cfg(target_os = "macos")]
    if let Ok(ptr) = panel.ns_window() {
        unsafe {
            clear_nswindow(ptr);
        }
    }
}

/// Raise the panel above the menu bar and keep it present on every Space
/// and over fullscreen apps, without ever stealing focus.
pub fn apply_island_window(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    if let Some(panel) = app.get_webview_window("panel") {
        if let Ok(ptr) = panel.ns_window() {
            unsafe {
                island_nswindow(ptr);
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

#[cfg(target_os = "macos")]
unsafe fn island_nswindow(ptr: *mut std::ffi::c_void) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    if ptr.is_null() {
        return;
    }
    let win = ptr as *mut AnyObject;
    // NSWindow.Level.mainMenu is 24; boring.notch floats at +3 above it.
    let _: () = msg_send![win, setLevel: 27i64];
    // fullScreenAuxiliary (1<<8) | stationary (1<<4)
    // | canJoinAllSpaces (1<<0) | ignoresCycle (1<<6) = 0x151 = 337
    let _: () = msg_send![win, setCollectionBehavior: 337u64];
}

/// Notch size of the main screen. Width follows boring.notch's formula:
/// screen width minus the two auxiliary areas beside the notch, plus 4pt
/// of overlap so the capsule fully covers the hardware notch seam.
pub fn notch_geometry() -> NotchGeometry {
    #[cfg(target_os = "macos")]
    {
        if let Some(geometry) = unsafe { macos_notch_geometry() } {
            return geometry;
        }
    }
    FALLBACK_GEOMETRY
}

#[cfg(target_os = "macos")]
unsafe fn macos_notch_geometry() -> Option<NotchGeometry> {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use objc2_foundation::{NSEdgeInsets, NSRect};

    let screen_cls = class!(NSScreen);
    let screen: *mut AnyObject = msg_send![screen_cls, mainScreen];
    if screen.is_null() {
        return None;
    }
    let frame: NSRect = msg_send![screen, frame];
    let insets: NSEdgeInsets = msg_send![screen, safeAreaInsets];
    let has_notch = insets.top > 0.0;

    let left: *mut NSRect = msg_send![screen, auxiliaryTopLeftArea];
    let right: *mut NSRect = msg_send![screen, auxiliaryTopRightArea];
    let width = if !left.is_null() && !right.is_null() {
        frame.size.width - (*left).size.width - (*right).size.width + 4.0
    } else {
        FALLBACK_GEOMETRY.width
    };
    let height = if has_notch {
        insets.top
    } else {
        FALLBACK_GEOMETRY.height
    };
    Some(NotchGeometry {
        has_notch,
        width,
        height,
    })
}

/// The visible capsule's rect in window-relative logical points
/// (top-left origin), reported by the frontend whenever it changes.
static PILL_RECT: Mutex<Option<(f64, f64, f64, f64)>> = Mutex::new(None);

pub fn set_pill_rect(x: f64, y: f64, width: f64, height: f64) {
    if let Ok(mut slot) = PILL_RECT.lock() {
        *slot = Some((x, y, width, height));
    }
}

/// Poll the global cursor and make the fixed-size island window
/// click-through everywhere except over the visible capsule, so the
/// transparent part of the window never blocks clicks beneath it.
pub fn spawn_click_through_guard(app: tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    std::thread::spawn(move || {
        let mut applied: Option<bool> = None;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(50));
            if let Some(ignore) = desired_ignore(&app) {
                if applied != Some(ignore) {
                    applied = Some(ignore);
                    let app2 = app.clone();
                    let _ = app.run_on_main_thread(move || {
                        if let Some(win) = app2.get_webview_window("panel") {
                            let _ = win.set_ignore_cursor_events(ignore);
                        }
                    });
                }
            }
        }
    });
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// `Some(ignore)` while the panel is visible; `None` leaves state alone.
/// A hidden panel always reports `Some(false)` so the next show starts
/// interactive.
#[cfg(target_os = "macos")]
fn desired_ignore(app: &tauri::AppHandle) -> Option<bool> {
    let panel = app.get_webview_window("panel")?;
    if !panel.is_visible().unwrap_or(false) {
        return Some(false);
    }
    let rect = match PILL_RECT.lock() {
        Ok(guard) => *guard,
        Err(_) => None,
    }?;
    let (rx, ry, rw, rh) = rect;
    unsafe {
        use objc2::runtime::AnyObject;
        use objc2::{class, msg_send};
        use objc2_foundation::{NSPoint, NSRect};

        let ptr = panel.ns_window().ok()?;
        if ptr.is_null() {
            return None;
        }
        let win = ptr as *mut AnyObject;
        let frame: NSRect = msg_send![win, frame];
        let event_cls = class!(NSEvent);
        let mouse: NSPoint = msg_send![event_cls, mouseLocation];
        // Window-relative rect (top-left origin) -> AppKit screen coords
        // (bottom-left origin).
        let px = frame.origin.x + rx;
        let py = frame.origin.y + frame.size.height - ry - rh;
        let margin = 8.0;
        let inside = mouse.x >= px - margin
            && mouse.x <= px + rw + margin
            && mouse.y >= py - margin
            && mouse.y <= py + rh + margin;
        Some(!inside)
    }
}

#[cfg(target_os = "macos")]
unsafe fn clear_nswindow(ptr: *mut std::ffi::c_void) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};

    if ptr.is_null() {
        return;
    }
    let win = ptr as *mut AnyObject;
    let clear: *mut AnyObject = msg_send![class!(NSColor), clearColor];
    let _: () = msg_send![win, setOpaque: false];
    let _: () = msg_send![win, setHasShadow: false];
    let _: () = msg_send![win, setBackgroundColor: clear];
    let _: () = msg_send![win, setTitlebarAppearsTransparent: true];

    let view: *mut AnyObject = msg_send![win, contentView];
    if view.is_null() {
        return;
    }
    let _: () = msg_send![view, setOpaque: false];
    let _: () = msg_send![view, setWantsLayer: true];
    let layer: *mut AnyObject = msg_send![view, layer];
    if layer.is_null() {
        return;
    }
    let _: () = msg_send![layer, setOpaque: false];
    // No native corner radius: the island shape (asymmetric top-attached
    // corners) is drawn entirely by CSS inside the webview.
    let _: () = msg_send![layer, setCornerRadius: 0.0f64];
    let _: () = msg_send![layer, setMasksToBounds: true];
    let cg_clear: *mut std::ffi::c_void = msg_send![clear, CGColor];
    let _: () = msg_send![layer, setBackgroundColor: cg_clear];
}

#[cfg(target_os = "macos")]
unsafe fn clear_wkwebview(ptr: *mut std::ffi::c_void) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};

    if ptr.is_null() {
        return;
    }
    let wv = ptr as *mut AnyObject;
    let clear: *mut AnyObject = msg_send![class!(NSColor), clearColor];
    let no: *mut AnyObject = msg_send![class!(NSNumber), numberWithBool: false];
    let key: *mut AnyObject =
        msg_send![class!(NSString), stringWithUTF8String: b"drawsBackground\0".as_ptr()];
    let _: () = msg_send![wv, setValue: no, forKey: key];
    let _: () = msg_send![wv, setOpaque: false];
    let _: () = msg_send![wv, setUnderPageBackgroundColor: clear];

    let _: () = msg_send![wv, setWantsLayer: true];
    let layer: *mut AnyObject = msg_send![wv, layer];
    if !layer.is_null() {
        let _: () = msg_send![layer, setOpaque: false];
        let cg_clear: *mut std::ffi::c_void = msg_send![clear, CGColor];
        let _: () = msg_send![layer, setBackgroundColor: cg_clear];
    }
}
