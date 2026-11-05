// A plain top-level X11 window that a plugin draws its own interface into
// (CLAP `set_parent`). libX11 is loaded when the first window opens, so the
// studio still starts without it; under Wayland the window goes through
// XWayland.

use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void, CStr};
use std::sync::OnceLock;

type Display = c_void;
type XWindow = c_ulong;
/// `XEvent` is a union of 24 longs.
type XEvent = [c_long; 24];

extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

struct Lib {
    open_display: unsafe extern "C" fn(*const c_char) -> *mut Display,
    close_display: unsafe extern "C" fn(*mut Display) -> c_int,
    default_screen: unsafe extern "C" fn(*mut Display) -> c_int,
    root_window: unsafe extern "C" fn(*mut Display, c_int) -> XWindow,
    black_pixel: unsafe extern "C" fn(*mut Display, c_int) -> c_ulong,
    display_width: unsafe extern "C" fn(*mut Display, c_int) -> c_int,
    display_height: unsafe extern "C" fn(*mut Display, c_int) -> c_int,
    create_simple_window: unsafe extern "C" fn(*mut Display, XWindow, c_int, c_int, c_uint, c_uint, c_uint, c_ulong, c_ulong) -> XWindow,
    store_name: unsafe extern "C" fn(*mut Display, XWindow, *const c_char) -> c_int,
    intern_atom: unsafe extern "C" fn(*mut Display, *const c_char, c_int) -> c_ulong,
    set_wm_protocols: unsafe extern "C" fn(*mut Display, XWindow, *mut c_ulong, c_int) -> c_int,
    select_input: unsafe extern "C" fn(*mut Display, XWindow, c_long) -> c_int,
    map_window: unsafe extern "C" fn(*mut Display, XWindow) -> c_int,
    resize_window: unsafe extern "C" fn(*mut Display, XWindow, c_uint, c_uint) -> c_int,
    destroy_window: unsafe extern "C" fn(*mut Display, XWindow) -> c_int,
    flush: unsafe extern "C" fn(*mut Display) -> c_int,
    pending: unsafe extern "C" fn(*mut Display) -> c_int,
    next_event: unsafe extern "C" fn(*mut Display, *mut XEvent) -> c_int,
    set_wm_normal_hints: unsafe extern "C" fn(*mut Display, XWindow, *const SizeHints),
    change_property: unsafe extern "C" fn(*mut Display, XWindow, c_ulong, c_ulong, c_int, c_int, *const c_ulong, c_int) -> c_int,
}

/// `XSizeHints`.
#[repr(C)]
#[derive(Default)]
struct SizeHints {
    flags: c_long,
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    min_width: c_int,
    min_height: c_int,
    max_width: c_int,
    max_height: c_int,
    width_inc: c_int,
    height_inc: c_int,
    min_aspect: [c_int; 2],
    max_aspect: [c_int; 2],
    base_width: c_int,
    base_height: c_int,
    win_gravity: c_int,
}
const P_POSITION: c_long = 1 << 2;
const P_SIZE: c_long = 1 << 3;
const P_MIN_SIZE: c_long = 1 << 4;
const P_MAX_SIZE: c_long = 1 << 5;
const XA_ATOM: c_ulong = 4;

/// A symbol from dlsym as the function pointer type of the field it fills.
unsafe fn fn_ptr<T: Copy>(f: *mut c_void) -> T {
    assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<*mut c_void>());
    std::mem::transmute_copy(&f)
}

fn lib() -> Option<&'static Lib> {
    static LIB: OnceLock<Option<Lib>> = OnceLock::new();
    LIB.get_or_init(|| unsafe {
        let h = dlopen(c"libX11.so.6".as_ptr(), 2);
        if h.is_null() {
            return None;
        }
        macro_rules! sym {
            ($name:literal) => {{
                let f = dlsym(h, $name.as_ptr());
                if f.is_null() {
                    return None;
                }
                fn_ptr(f)
            }};
        }
        Some(Lib {
            open_display: sym!(c"XOpenDisplay"),
            close_display: sym!(c"XCloseDisplay"),
            default_screen: sym!(c"XDefaultScreen"),
            root_window: sym!(c"XRootWindow"),
            black_pixel: sym!(c"XBlackPixel"),
            display_width: sym!(c"XDisplayWidth"),
            display_height: sym!(c"XDisplayHeight"),
            create_simple_window: sym!(c"XCreateSimpleWindow"),
            store_name: sym!(c"XStoreName"),
            intern_atom: sym!(c"XInternAtom"),
            set_wm_protocols: sym!(c"XSetWMProtocols"),
            select_input: sym!(c"XSelectInput"),
            map_window: sym!(c"XMapWindow"),
            resize_window: sym!(c"XResizeWindow"),
            destroy_window: sym!(c"XDestroyWindow"),
            flush: sym!(c"XFlush"),
            pending: sym!(c"XPending"),
            next_event: sym!(c"XNextEvent"),
            set_wm_normal_hints: sym!(c"XSetWMNormalHints"),
            change_property: sym!(c"XChangeProperty"),
        })
    })
    .as_ref()
}

const STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;
const CONFIGURE_NOTIFY: c_int = 22;
const CLIENT_MESSAGE: c_int = 33;

/// What happened to the window since the last look.
#[derive(Default)]
pub struct Events {
    pub closed: bool,
    pub resized: Option<(u32, u32)>,
}

pub struct Window {
    display: *mut Display,
    id: XWindow,
    delete_atom: c_ulong,
    size: (u32, u32),
}

unsafe impl Send for Window {}

impl Window {
    /// A dialog window (tiling window managers let it float); `resizable`
    /// false fixes its size to the plugin's.
    pub fn new(title: &CStr, w: u32, h: u32, resizable: bool) -> Result<Window, String> {
        let x = lib().ok_or("libX11 is not installed")?;
        unsafe {
            let display = (x.open_display)(std::ptr::null());
            if display.is_null() {
                return Err("no X11 display (is XWayland running?)".into());
            }
            let screen = (x.default_screen)(display);
            let root = (x.root_window)(display, screen);
            let black = (x.black_pixel)(display, screen);
            // Centred on the screen (all monitors together under XWayland).
            let px = (((x.display_width)(display, screen) - w as c_int) / 2).max(0);
            let py = (((x.display_height)(display, screen) - h as c_int) / 2).max(0);
            let id = (x.create_simple_window)(display, root, px, py, w, h, 0, black, black);
            (x.store_name)(display, id, title.as_ptr());
            let mut delete_atom = (x.intern_atom)(display, c"WM_DELETE_WINDOW".as_ptr(), 0);
            (x.set_wm_protocols)(display, id, &mut delete_atom, 1);
            (x.select_input)(display, id, STRUCTURE_NOTIFY_MASK);
            let kind = (x.intern_atom)(display, c"_NET_WM_WINDOW_TYPE".as_ptr(), 0);
            let dialog = (x.intern_atom)(display, c"_NET_WM_WINDOW_TYPE_DIALOG".as_ptr(), 0);
            (x.change_property)(display, id, kind, XA_ATOM, 32, 0, &dialog, 1);
            let mut hints = SizeHints { flags: P_POSITION | P_SIZE, x: px, y: py, width: w as c_int, height: h as c_int, ..Default::default() };
            if !resizable {
                hints.flags |= P_MIN_SIZE | P_MAX_SIZE;
                (hints.min_width, hints.min_height) = (w as c_int, h as c_int);
                (hints.max_width, hints.max_height) = (w as c_int, h as c_int);
            }
            (x.set_wm_normal_hints)(display, id, &hints);
            (x.flush)(display);
            Ok(Window { display, id, delete_atom, size: (w, h) })
        }
    }

    pub fn id(&self) -> c_ulong {
        self.id
    }

    pub fn show(&self) {
        let Some(x) = lib() else { return };
        unsafe {
            (x.map_window)(self.display, self.id);
            (x.flush)(self.display);
        }
    }

    pub fn resize(&mut self, w: u32, h: u32, resizable: bool) {
        let Some(x) = lib() else { return };
        self.size = (w, h);
        unsafe {
            if !resizable {
                let (w, h) = (w as c_int, h as c_int);
                let flags = P_SIZE | P_MIN_SIZE | P_MAX_SIZE;
                let hints = SizeHints { flags, width: w, height: h, min_width: w, min_height: h, max_width: w, max_height: h, ..Default::default() };
                (x.set_wm_normal_hints)(self.display, self.id, &hints);
            }
            (x.resize_window)(self.display, self.id, w.max(1), h.max(1));
            (x.flush)(self.display);
        }
    }

    /// Reads waiting events without blocking.
    pub fn events(&mut self) -> Events {
        let mut out = Events::default();
        let Some(x) = lib() else { return out };
        let mut ev: XEvent = [0; 24];
        unsafe {
            while (x.pending)(self.display) > 0 {
                (x.next_event)(self.display, &mut ev);
                let kind = ev[0] as c_int;
                if kind == CLIENT_MESSAGE && ev[7] as c_ulong == self.delete_atom {
                    out.closed = true;
                } else if kind == CONFIGURE_NOTIFY {
                    // XConfigureEvent: width and height are the two ints after x, y.
                    let ints = ev.as_ptr() as *const c_int;
                    let (w, h) = (*ints.add(14) as u32, *ints.add(15) as u32);
                    if (w, h) != self.size && w > 0 && h > 0 {
                        self.size = (w, h);
                        out.resized = Some((w, h));
                    }
                }
            }
        }
        out
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        let Some(x) = lib() else { return };
        unsafe {
            (x.destroy_window)(self.display, self.id);
            (x.close_display)(self.display);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_layout_matches_xlib() {
        // XClientMessageEvent.data.l[0] is the 8th long, XConfigureEvent.width the 15th int.
        assert_eq!(std::mem::size_of::<XEvent>(), 192);
        assert_eq!(std::mem::size_of::<c_long>(), 8);
        assert_eq!(std::mem::size_of::<SizeHints>(), 80);
    }
}
