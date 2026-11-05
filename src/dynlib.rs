// Loading shared libraries at run time (CLAP plugins, LAME, libX11):
// dlopen on Linux and macOS, LoadLibrary on Windows. Libraries are never
// closed: plugins may keep threads running.

use std::ffi::{c_char, c_void, CStr};
use std::path::Path;

#[derive(Clone, Copy)]
pub struct Lib(*mut c_void);

unsafe impl Send for Lib {}

#[cfg(unix)]
mod sys {
    use std::ffi::{c_char, c_int, c_void};
    extern "C" {
        pub fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
        pub fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        pub fn dlerror() -> *const c_char;
    }
    pub const RTLD_NOW: c_int = 2;
}

#[cfg(windows)]
mod sys {
    use std::ffi::{c_char, c_void};
    #[link(name = "kernel32")]
    extern "system" {
        pub fn LoadLibraryW(name: *const u16) -> *mut c_void;
        pub fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
        pub fn GetLastError() -> u32;
    }
}

impl Lib {
    /// Opens a library by file path or by plain name (searched the usual way).
    pub fn open(path: &Path) -> Result<Lib, String> {
        #[cfg(unix)]
        {
            let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(|_| "bad file name")?;
            let h = unsafe { sys::dlopen(c.as_ptr(), sys::RTLD_NOW) };
            if h.is_null() {
                let e = unsafe { sys::dlerror() };
                let msg = if e.is_null() { "unknown error".into() } else { unsafe { CStr::from_ptr(e) }.to_string_lossy().into_owned() };
                return Err(format!("cannot load: {msg}"));
            }
            Ok(Lib(h))
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let h = unsafe { sys::LoadLibraryW(wide.as_ptr()) };
            if h.is_null() {
                return Err(format!("cannot load (error {})", unsafe { sys::GetLastError() }));
            }
            Ok(Lib(h))
        }
    }

    /// The first of these names that loads.
    pub fn open_any(names: &[&str]) -> Option<Lib> {
        names.iter().find_map(|n| Lib::open(Path::new(n)).ok())
    }

    /// A symbol's address, or null.
    pub fn sym(self, name: &CStr) -> *mut c_void {
        let n: *const c_char = name.as_ptr();
        #[cfg(unix)]
        unsafe {
            sys::dlsym(self.0, n)
        }
        #[cfg(windows)]
        unsafe {
            sys::GetProcAddress(self.0, n)
        }
    }
}
