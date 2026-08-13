//! Make the HUD window truly transparent so only the CSS capsule is visible.

use tauri::{webview::Color, Manager};

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
            clear_nswindow(ptr, 19.0);
        }
    }
}

pub fn set_corner_radius(app: &tauri::AppHandle, radius: f64) {
    #[cfg(target_os = "macos")]
    if let Some(panel) = app.get_webview_window("panel") {
        if let Ok(ptr) = panel.ns_window() {
            unsafe {
                clear_nswindow(ptr, radius);
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, radius);
}

#[cfg(target_os = "macos")]
unsafe fn clear_nswindow(ptr: *mut std::ffi::c_void, radius: f64) {
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
    let _: () = msg_send![layer, setCornerRadius: radius];
    let _: () = msg_send![layer, setMasksToBounds: true];
    // continuous squircles match the CSS capsule better
    let curve: *mut AnyObject =
        msg_send![class!(NSString), stringWithUTF8String: b"continuous\0".as_ptr()];
    let _: () = msg_send![layer, setCornerCurve: curve];
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
