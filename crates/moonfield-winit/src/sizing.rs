//! Windows-only: keep frames running while the OS holds the thread in a
//! modal size/move loop (`WM_ENTERSIZEMOVE`…`WM_EXITSIZEMOVE`).
//!
//! During the modal loop `DefWindowProc` pumps messages internally: winit's
//! event loop never reaches `about_to_wait`, and `WM_PAINT` is not delivered
//! either (measured: `RedrawRequested` stops for the whole drag), so without
//! intervention the window shows nothing until the mouse is released and the
//! compositor fills it with the window background. A window subclass starts
//! a timer on `WM_ENTERSIZEMOVE`; `WM_TIMER` is dispatched inside the modal
//! loop, and each tick runs one app frame through the same `App::update`
//! entry point as normal frames.
//!
//! Everything here is main-thread-only: the subclass proc, the timer, and
//! the frame driver all run on the event-loop thread.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::Cell;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    KillTimer, SetTimer, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE, WM_NCDESTROY, WM_TIMER,
};

/// Timer cadence while the modal loop holds the thread (about 60 Hz).
const SIZING_TIMER_MS: u32 = 16;
/// Window timer ID for the sizing-loop frame driver.
const SIZING_TIMER_ID: usize = 1;
/// Subclass ID (per-window unique cookie for `RemoveWindowSubclass`).
const SIZING_SUBCLASS_ID: usize = 1;

/// Type-erased frame driver: a context pointer plus the shim that calls it.
/// The shim erases the handler's type (and lifetime) for the thread-local.
#[derive(Clone, Copy)]
struct Driver {
    context: *mut (),
    shim: unsafe fn(*mut ()),
}

thread_local! {
    /// The live frame driver while `winit_run` is on the stack.
    static FRAME_DRIVER: Cell<Option<Driver>> = const { Cell::new(None) };
}

/// Register the frame driver for the duration of the event loop.
pub(crate) fn set_frame_driver(context: *mut (), shim: unsafe fn(*mut ())) {
    FRAME_DRIVER.with(|driver| driver.set(Some(Driver { context, shim })));
}

/// Clear the frame driver after the event loop has exited.
pub(crate) fn clear_frame_driver() {
    FRAME_DRIVER.with(|driver| driver.set(None));
}

/// Subclass a winit window so the modal loop's boundaries start/stop the
/// frame timer. No-ops on non-Win32 handles.
pub(crate) fn attach(window: &impl HasWindowHandle) {
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as HWND;
    // SAFETY: `hwnd` is a live window owned by this thread; the proc outlives
    // the subclass (static fn) and removes itself on `WM_NCDESTROY`.
    unsafe { SetWindowSubclass(hwnd, Some(sizing_subclass_proc), SIZING_SUBCLASS_ID, 0) };
}

/// Subclass proc: starts the frame timer when the modal size/move loop
/// begins, stops it when the loop ends, and runs one frame per timer tick.
unsafe extern "system" fn sizing_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _uid: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_ENTERSIZEMOVE => {
            // SAFETY: `hwnd` is alive (we are its window proc) and the timer
            // belongs to this thread.
            unsafe { SetTimer(hwnd, SIZING_TIMER_ID, SIZING_TIMER_MS, None) };
        }
        WM_EXITSIZEMOVE => {
            // SAFETY: as above; killing a dead timer is a no-op.
            unsafe { KillTimer(hwnd, SIZING_TIMER_ID) };
        }
        WM_TIMER if wparam == SIZING_TIMER_ID => {
            drive_frame();
            // Consumed: the default window proc does nothing with WM_TIMER.
            return 0;
        }
        WM_NCDESTROY => {
            // SAFETY: `hwnd` is being destroyed; both calls are the paired
            // teardown of what `attach` and `WM_ENTERSIZEMOVE` installed.
            unsafe {
                KillTimer(hwnd, SIZING_TIMER_ID);
                RemoveWindowSubclass(hwnd, Some(sizing_subclass_proc), SIZING_SUBCLASS_ID);
            }
        }
        _ => {}
    }
    // SAFETY: forwards to the next proc in the subclass chain (winit's).
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Run one frame through the registered driver, if the event loop is alive.
fn drive_frame() {
    FRAME_DRIVER.with(|driver| {
        if let Some(driver) = driver.get() {
            // SAFETY: the context is the `WinitHandler` owned by `winit_run`,
            // alive for the event loop's duration and cleared right after;
            // the timer fires on the event-loop thread, and only while that
            // thread is blocked in the modal loop's message pump, where no
            // frame is on the stack.
            unsafe { (driver.shim)(driver.context) };
        }
    });
}
