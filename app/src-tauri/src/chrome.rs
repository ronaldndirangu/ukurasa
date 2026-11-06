//! Window chrome. On Windows the title bar draws its own caption buttons, so an invisible child
//! window over the maximize button tells Windows it is one, which keeps Snap Layouts working.

use serde::Deserialize;
use tauri::WebviewWindow;

/// A rectangle in CSS pixels from the top-left corner of the window's content.
#[derive(Clone, Copy, Debug, Deserialize)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[cfg(windows)]
pub use snap::{attach, rescale};

#[cfg(not(windows))]
pub fn attach(_: &WebviewWindow) {}

#[cfg(not(windows))]
pub fn rescale(_: &tauri::Window) {}

/// Covers the title bar's maximize button, or nothing when `rect` is none.
#[tauri::command]
pub fn set_maximize_button(window: WebviewWindow, rect: Option<Rect>) {
    #[cfg(windows)]
    snap::place(&window, rect);
    #[cfg(not(windows))]
    let _ = (window, rect);
}

/// Follows Windows Terminal: a layered child window without a redirection surface draws nothing,
/// answers `HTMAXBUTTON` to hit tests, and reports hover and press to the page, which draws the
/// button.
#[cfg(windows)]
mod snap {
    use std::mem;
    use std::ptr;
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
    use std::sync::{Mutex, OnceLock, PoisonError};

    use tauri::{AppHandle, Emitter, Manager, WebviewWindow, Window};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, GetParent, HTMAXBUTTON, HWND_BOTTOM, HWND_TOP, IDC_ARROW,
        IsZoomed, LWA_ALPHA, LoadCursorW, PostMessageW, RegisterClassW, SC_MAXIMIZE, SC_RESTORE,
        SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
        SetLayeredWindowAttributes, SetWindowPos, WM_NCHITTEST, WM_NCLBUTTONDBLCLK,
        WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_NCRBUTTONDBLCLK,
        WM_NCRBUTTONDOWN, WM_NCRBUTTONUP, WM_SYSCOMMAND, WNDCLASSW, WS_CHILD, WS_EX_LAYERED,
        WS_EX_NOREDIRECTIONBITMAP,
    };
    use windows_sys::core::w;

    use super::Rect;

    /// Tells the page how to draw the button: `hover`, `press`, or `leave`.
    const EVENT: &str = "maximize-button";

    static APP: OnceLock<AppHandle> = OnceLock::new();
    static SINK: AtomicIsize = AtomicIsize::new(0);
    static RECT: Mutex<Option<Rect>> = Mutex::new(None);
    static TRACKING: AtomicBool = AtomicBool::new(false);
    static PRESSED: AtomicBool = AtomicBool::new(false);

    /// Creates the hidden child window. Without it the button still works, minus Snap Layouts.
    pub fn attach(window: &WebviewWindow) {
        let Ok(parent) = window.hwnd() else {
            return;
        };
        let _ = APP.set(window.app_handle().clone());
        let class_name = w!("UkurasaMaximizeButton");
        // SAFETY: plain Win32 calls with valid arguments; the class and window live as long as
        // the process.
        unsafe {
            let instance = GetModuleHandleW(ptr::null());
            let class = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(procedure),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: ptr::null_mut(),
                hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
                hbrBackground: ptr::null_mut(),
                lpszMenuName: ptr::null(),
                lpszClassName: class_name,
            };
            if RegisterClassW(&class) == 0 {
                return;
            }
            let sink = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_NOREDIRECTIONBITMAP,
                class_name,
                ptr::null(),
                WS_CHILD,
                0,
                0,
                0,
                0,
                parent.0 as HWND,
                ptr::null_mut(),
                instance,
                ptr::null(),
            );
            if sink.is_null() {
                return;
            }
            SetLayeredWindowAttributes(sink, 0, 255, LWA_ALPHA);
            SINK.store(sink as isize, Ordering::Relaxed);
        }
    }

    pub fn place(window: &WebviewWindow, rect: Option<Rect>) {
        *RECT.lock().unwrap_or_else(PoisonError::into_inner) = rect;
        update(window.scale_factor().unwrap_or(1.0));
    }

    /// Keeps the child window over the button when the window moves to a display with another
    /// scale, which leaves the page's layout unchanged.
    pub fn rescale(window: &Window) {
        update(window.scale_factor().unwrap_or(1.0));
    }

    fn update(scale: f64) {
        let sink = SINK.load(Ordering::Relaxed) as HWND;
        if sink.is_null() {
            return;
        }
        let rect = *RECT.lock().unwrap_or_else(PoisonError::into_inner);
        let px = |value: f64| (value * scale).round() as i32;
        // SAFETY: `sink` is a live window created by `attach`.
        unsafe {
            match rect {
                Some(rect) => {
                    let (left, top) = (px(rect.x), px(rect.y));
                    let (right, bottom) = (px(rect.x + rect.width), px(rect.y + rect.height));
                    SetWindowPos(
                        sink,
                        HWND_TOP,
                        left,
                        top,
                        right - left,
                        bottom - top,
                        SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    );
                }
                None => {
                    SetWindowPos(
                        sink,
                        HWND_BOTTOM,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_HIDEWINDOW,
                    );
                }
            }
        }
    }

    fn notify(state: &str) {
        if let Some(app) = APP.get() {
            let _ = app.emit(EVENT, state);
        }
    }

    unsafe extern "system" fn procedure(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_NCHITTEST => return HTMAXBUTTON as LRESULT,
            WM_NCMOUSEMOVE if !TRACKING.swap(true, Ordering::Relaxed) => {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE | TME_NONCLIENT,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                // SAFETY: `track` is a valid, initialized structure.
                unsafe { TrackMouseEvent(&mut track) };
                notify("hover");
            }
            WM_NCMOUSELEAVE => {
                TRACKING.store(false, Ordering::Relaxed);
                PRESSED.store(false, Ordering::Relaxed);
                notify("leave");
            }
            // The default handling would start a modal loop on this window instead of the parent.
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => {
                PRESSED.store(true, Ordering::Relaxed);
                notify("press");
                return 0;
            }
            WM_NCLBUTTONUP => {
                if PRESSED.swap(false, Ordering::Relaxed) {
                    notify("hover");
                    // SAFETY: `hwnd` is the sink, whose parent is the app window.
                    unsafe {
                        let parent = GetParent(hwnd);
                        let command = if IsZoomed(parent) != 0 {
                            SC_RESTORE
                        } else {
                            SC_MAXIMIZE
                        };
                        PostMessageW(parent, WM_SYSCOMMAND, command as WPARAM, 0);
                    }
                }
                return 0;
            }
            WM_NCRBUTTONDOWN | WM_NCRBUTTONUP | WM_NCRBUTTONDBLCLK => return 0,
            _ => {}
        }
        // SAFETY: forwards the message unchanged to the default procedure.
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }
}
