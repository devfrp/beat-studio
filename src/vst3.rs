// VST3 plugin hosting: scanning, loading, sound processing, parameters and
// saved state, mirroring the CLAP layer in plugin.rs. The plugin's own window
// is not opened yet (its parameters still get generic sliders). Scanning runs
// each file in a child process (`beatstudio --vst3-scan <file>`) so a plugin
// that crashes cannot take the studio down with it.
//
// The ABI is the one in the official plugin headers (pluginterfaces/, VST 3
// SDK): C++ objects reached through a vtable, an FUnknown query function and
// 16-byte GUIDs. Interface functions are declared `extern "system"`: this is
// member-call style on Windows and plain C on Linux and macOS. Layout facts
// from the headers, pinned by a compiled probe and by the tests here:
// enums are int32, TBool is one byte, String128 is 128 u16, GUIDs from
// INLINE_UID write every 32-bit word big-endian.
//
// Slots (vtable positions from 0): every object answers query_interface (0),
// add_ref (1), release (2). Slots after that belong to the base classes in
// order: IPluginBase adds initialize (3) and terminate (4). So IComponent's
// own functions start at 5 (getControllerClassId 5 ... getState 13), as do
// IEditController's (setComponentState 5 ... setComponentHandler 16).
// Everything extending FUnknown directly starts at 3: IAudioProcessor,
// IHostApplication, IComponentHandler, IBStream, IEventList,
// IParameterChanges, IParamValueQueue, IPluginFactory (+ getClassInfo2 at 7
// on IPluginFactory2, which extends IPluginFactory).

use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

// ---------------------------------------------------------------- constants

const K_RESULT_OK: i32 = 0;
const K_RESULT_FALSE: i32 = 1;
const K_RESULT_INVALID_ARGUMENT: i32 = 2;
const K_NOT_IMPLEMENTED: i32 = 3;
const K_NO_INTERFACE: i32 = -1;

const MAX_FRAMES: u32 = 4096;

// Buses (ivstcomponent.h).
const BUS_MEDIA_AUDIO: i32 = 0;
const BUS_MEDIA_EVENT: i32 = 1;
const BUS_DIRECTION_INPUT: i32 = 0;
const BUS_DIRECTION_OUTPUT: i32 = 1;
const BUS_TYPE_MAIN: i32 = 0;

// Parameters (ivsteditcontroller.h).
const PARAM_IS_READONLY: i32 = 1 << 1;
const PARAM_IS_HIDDEN: i32 = 1 << 4;

// Events (ivstevents.h).
const EVENT_NOTE_ON: u16 = 0;
const EVENT_NOTE_OFF: u16 = 1;

// Process context flags (ivstprocesscontext.h).
const CONTEXT_PLAYING: u32 = 1 << 1;
const CONTEXT_SYSTEM_TIME_VALID: u32 = 1 << 8;
const CONTEXT_PROJECT_TIME_MUSIC_VALID: u32 = 1 << 9;
const CONTEXT_TEMPO_VALID: u32 = 1 << 10;
const CONTEXT_BAR_POSITION_VALID: u32 = 1 << 11;
const CONTEXT_TIME_SIG_VALID: u32 = 1 << 13;
const CONTEXT_CONTINUOUS_TIME_VALID: u32 = 1 << 17;

// restartComponent flags (ivsteditcontroller.h).
const RESTART_IO_CHANGED: i32 = 1 << 1;
const RESTART_RELOAD: i32 = 1 << 0;

// GUIDs as the hex text of their bytes (INLINE_UID word order).
const U_UNKNOWN: &[u8; 32] = b"0000000000000000C000000000000046";
const U_IPLUGIN_FACTORY_2: &[u8; 32] = b"0007B650F24B4C0BA464EDB9F00B2ABB";
const U_IHOST_APPLICATION: &[u8; 32] = b"58E595CCDB2D49698B6AAF8C36A664E5";
const U_ICOMPONENT: &[u8; 32] = b"E831FF31F2D54301928EBBEE25697802";
const U_IAUDIO_PROCESSOR: &[u8; 32] = b"42043F99B7DA453CA569E79D9AAEC33D";
const U_IEDIT_CONTROLLER: &[u8; 32] = b"3F7B700A2B6F41E2922BDAD76118B916";
// Older JUCE wrappers (and a few 3.6.x-era SDK builds) still key the edit
// controller under this iid; Dexed only answers it, so both are tried.
const U_IEDIT_CONTROLLER_ALT: &[u8; 32] = b"DCD7BBE37742448DA874AACC979C759E";
const U_ICOMPONENT_HANDLER: &[u8; 32] = b"93A0BEA30BD045DB8E890B0CC1E46AC6";
const U_IPARAMETER_CHANGES: &[u8; 32] = b"A47796630BB64A56B44384A8466FEB9D";
const U_IPARAM_VALUE_QUEUE: &[u8; 32] = b"01263A18ED074F6F98C9D3564686F9BA";
const U_IEVENT_LIST: &[u8; 32] = b"3A2C4214346349FEB2C4F397B9695A44";
const U_ICONNECTION_POINT: &[u8; 32] = b"70A4156F6E6E4026989148BFAA60D8D1";
const U_IBSTREAM: &[u8; 32] = b"C3BF6EA2309947529B6BF9901EE33E9B";

/// A 16-byte GUID: the bytes of the hex text.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Tuid([u8; 16]);

impl Tuid {
    const fn hex(hex: &[u8; 32]) -> Tuid {
        let mut out = [0u8; 16];
        let mut i = 0;
        while i < 16 {
            out[i] = (hex_digit(hex[i * 2]) * 16 + hex_digit(hex[i * 2 + 1])) as u8;
            i += 1;
        }
        Tuid(out)
    }

    fn as_ptr(&self) -> *const c_void {
        self.0.as_ptr() as *const c_void
    }

    /// The 32 upper-case hex digits of a class id, like the scan lines use.
    fn text(&self) -> String {
        self.0.iter().map(|b| format!("{b:02X}")).collect()
    }
}

const fn hex_digit(c: u8) -> u32 {
    match c {
        b'0'..=b'9' => (c - b'0') as u32,
        b'a'..=b'f' => (c - b'a' + 10) as u32,
        b'A'..=b'F' => (c - b'A' + 10) as u32,
        _ => 0,
    }
}

/// A class id read from a 32-hex-digit string (projects, cache).
fn uid_from_hex(text: &str) -> Option<Tuid> {
    let b = text.as_bytes();
    (b.len() == 32 && b.iter().all(|c| c.is_ascii_hexdigit())).then(|| {
        let mut hex = [b'0'; 32];
        hex.copy_from_slice(b);
        Tuid::hex(&hex)
    })
}

// ---------------------------------------------------------------- call side

/// An object of the plugin. Slot n of `*obj` is the vtable entry n.
#[derive(Clone, Copy)]
struct Obj(*mut c_void);

impl Obj {
    fn of(p: *mut c_void) -> Option<Obj> {
        (!p.is_null()).then_some(Obj(p))
    }

    unsafe fn slot<T>(&self, index: usize) -> T {
        let vt = *(self.0 as *mut *mut c_void) as *mut *const c_void;
        let fp = *vt.add(index);
        std::mem::transmute_copy::<*const c_void, T>(&fp)
    }

    fn query_bytes(&self, bytes: &[u8; 16]) -> Option<Obj> {
        let mut out: *mut c_void = ptr::null_mut();
        let f: unsafe extern "system" fn(*mut c_void, *const c_void, *mut *mut c_void) -> i32 =
            unsafe { self.slot(0) };
        let r = unsafe { f(self.0, bytes.as_ptr() as *const c_void, &mut out) };
        if r == K_RESULT_OK && !out.is_null() {
            Obj::of(out)
        } else {
            None
        }
    }

    fn query(&self, iid_hex: &[u8; 32]) -> Option<Obj> {
        let mut out: *mut c_void = ptr::null_mut();
        let f: unsafe extern "system" fn(*mut c_void, *const c_void, *mut *mut c_void) -> i32 =
            unsafe { self.slot(0) };
        // A plugin hands over the 16 raw GUID bytes, so that is what we do
        // too (the text form is only how we store ids around the studio).
        let bytes = Tuid::hex(iid_hex);
        let r = unsafe { f(self.0, bytes.0.as_ptr() as *const c_void, &mut out) };
        if r == K_RESULT_OK && !out.is_null() {
            Obj::of(out)
        } else {
            None
        }
    }

    fn release(&self) {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { self.slot(2) };
        unsafe { f(self.0) };
    }
}

// ---------------------------------------------------------------- host objects

// A host object is a `#[repr(C)]` struct whose first field is a pointer to a
// static vtable; the plugin receives the object address and calls slots on
// it. Objects are not ref counted (they live exactly as long as their
// owner): add_ref/release answer 1.

type TRef = unsafe extern "system" fn(*mut c_void) -> u32;
type TQuery = unsafe extern "system" fn(*mut c_void, *const c_void, *mut *mut c_void) -> i32;

unsafe extern "system" fn host_ref(_: *mut c_void) -> u32 {
    1
}

/// query_interface for a host object: it hands `this` back for FUnknown and
/// its own interfaces, refuses everything else (kNoInterface, like SDK code).
/// Only ever called from our own vtable entries with a literal list, so no
/// stable-ABI signature here.
unsafe fn hand_out(
    this: *mut c_void,
    iid: *const c_void,
    out: *mut *mut c_void,
    own: &[&'static [u8; 32]],
) -> i32 {
    if !out.is_null() {
        *out = ptr::null_mut();
    }
    let wanted = Tuid(*(iid as *const [u8; 16]));
    if own.iter().any(|hex| Tuid::hex(hex) == wanted) {
        if !out.is_null() {
            *out = this;
        }
        return K_RESULT_OK;
    }
    K_NO_INTERFACE
}

unsafe extern "system" fn message_query(this: *mut c_void, iid: *const c_void, out: *mut *mut c_void) -> i32 {
    hand_out(this, iid, out, &[U_UNKNOWN, U_IMESSAGE])
}

unsafe extern "system" fn attrs_query(this: *mut c_void, iid: *const c_void, out: *mut *mut c_void) -> i32 {
    hand_out(this, iid, out, &[U_UNKNOWN, U_IATTRIBUTE_LIST])
}

// ---- IHostApplication: the "context" the plugin classes get ---------------

#[repr(C)]
struct HostAppVt {
    query: TQuery,
    add_ref: TRef,
    release: TRef,
    get_name: unsafe extern "system" fn(*mut c_void, *mut u16) -> i32,
    create_instance: unsafe extern "system" fn(*mut c_void, *const c_void, *const c_void, *mut *mut c_void) -> i32,
}

#[repr(C)]
struct HostAppObj {
    vt: &'static HostAppVt,
    /// Host name, as UTF-16 with a zero at the end (String128).
    name: [u16; 128],
}

unsafe extern "system" fn hostapp_query(this: *mut c_void, iid: *const c_void, out: *mut *mut c_void) -> i32 {
    unsafe { hand_out(this, iid, out, &[U_UNKNOWN, U_IHOST_APPLICATION]) }
}

unsafe extern "system" fn hostapp_name(this: *mut c_void, name: *mut u16) -> i32 {
    let obj = &*(this as *const HostAppObj);
    if !name.is_null() {
        std::ptr::copy_nonoverlapping(obj.name.as_ptr(), name, 128);
    }
    K_RESULT_OK
}

unsafe extern "system" fn hostapp_create(_: *mut c_void, cid: *const c_void, iid: *const c_void, out: *mut *mut c_void) -> i32 {
    // Host objects the studio makes fresh for one plugin call: a message
    // (with its attribute list) is how the two parts of divided plugins
    // introduce themselves to each other, and some plugins ask for other
    // services (buffers, pools) that stay unanswered.
    if out.is_null() {
        return K_RESULT_INVALID_ARGUMENT;
    }
    *out = ptr::null_mut();
    if cid.is_null() || iid.is_null() {
        return K_RESULT_INVALID_ARGUMENT;
    }
    let asked = Tuid(*(cid as *const [u8; 16]));
    let iface = Tuid(*(iid as *const [u8; 16]));
    if asked == Tuid::hex(U_IMESSAGE) && iface == Tuid::hex(U_IMESSAGE) {
        let m = new_message();
        *out = &*m as *const MessageObj as *mut c_void;
        std::mem::forget(m); // one reference held by the caller now
        return K_RESULT_OK;
    }
    K_NOT_IMPLEMENTED
}

struct HostAppVtKnown(HostAppVt);
unsafe impl Sync for HostAppVtKnown {}

unsafe impl Send for HostAppObj {}

/// The one host application object, given to the component and the
/// controller, kept alive for the whole instance. The raw pointer handed to
/// the plugin is its address.
struct Host(Box<HostAppObj>);

impl Host {
    /// The raw pointer handed to the plugin.
    fn ptr(&self) -> *mut c_void {
        &*self.0 as *const HostAppObj as *mut c_void
    }
}

unsafe impl Send for Host {}

fn new_host() -> Host {
    static VT: OnceLock<HostAppVtKnown> = OnceLock::new();
    let vt = &VT.get_or_init(|| HostAppVtKnown(HostAppVt {
        query: hostapp_query,
        add_ref: host_ref,
        release: host_ref,
        get_name: hostapp_name,
        create_instance: hostapp_create,
    })).0;
    let mut name = [0u16; 128];
    for (i, ch) in "Beat Studio".encode_utf16().take(127).enumerate() {
        name[i] = ch;
    }
    Host(Box::new(HostAppObj { vt, name }))
}

// ---- IMessage and its attribute list (host side) ---------------------------
//
// Verbs: a divided plugin's controller discovers the component's inner object
// either by querying the connection point directly, or by sending a message
// through the connection point ("JuceVST3EditController" and friends). The
// message travels with an attribute list, both created here on demand.

const U_IMESSAGE: &[u8; 32] = b"936F033BC6C047DBBB0882F813C1E613";
const U_IATTRIBUTE_LIST: &[u8; 32] = b"1E5F0AEBCC7F4533A254401138AD5EE4";

enum AttrValue {
    Int(i64),
    Float(f64),
    Text(Vec<u16>),
    Bin(Vec<u8>),
}

// The fields are the plugin-facing sequence of vtable entries; the studio
// side writes them once and never calls them itself.
#[allow(dead_code)]
struct MessageVt {
    query: TQuery,
    add_ref: TRef,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    get_id: unsafe extern "system" fn(*mut c_void) -> *const c_char,
    set_id: unsafe extern "system" fn(*mut c_void, *const c_char),
    get_attrs: unsafe extern "system" fn(*mut c_void) -> *mut c_void,
}

#[allow(dead_code)]
struct AttrVt {
    query: TQuery,
    add_ref: TRef,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    set_int: unsafe extern "system" fn(*mut c_void, *const c_char, i64) -> i32,
    get_int: unsafe extern "system" fn(*mut c_void, *const c_char, *mut i64) -> i32,
    set_float: unsafe extern "system" fn(*mut c_void, *const c_char, f64) -> i32,
    get_float: unsafe extern "system" fn(*mut c_void, *const c_char, *mut f64) -> i32,
    set_string: unsafe extern "system" fn(*mut c_void, *const c_char, *const u16) -> i32,
    get_string: unsafe extern "system" fn(*mut c_void, *const c_char, *mut u16, u32) -> i32,
    set_binary: unsafe extern "system" fn(*mut c_void, *const c_char, *const c_void, u32) -> i32,
    get_binary: unsafe extern "system" fn(*mut c_void, *const c_char, *mut *const c_void, *mut u32) -> i32,
}

/// Attribute values of one message; keys are the C strings the plugin passes.
#[repr(C)]
struct AttrListObj {
    vt: &'static AttrVt,
    count: std::cell::Cell<u32>,
    map: std::collections::HashMap<Vec<u8>, AttrValue>,
}

/// One message in flight: an id string and the attribute list beside it.
#[repr(C)]
struct MessageObj {
    vt: &'static MessageVt,
    count: std::cell::Cell<u32>,
    id: Vec<u8>, // c-string bytes, without the nul
    attrs: Box<AttrListObj>,
}

/// Refcount helpers for the host-made message objects: they start at one
/// (the caller of createInstance owns that reference) and die at zero.
fn bump(count: &std::cell::Cell<u32>) -> u32 {
    let n = count.get() + 1;
    count.set(n);
    n
}

unsafe extern "system" fn message_ref(this: *mut c_void) -> u32 {
    bump(&(&*(this as *const MessageObj)).count)
}

unsafe extern "system" fn message_release(this: *mut c_void) -> u32 {
    let obj = &mut *(this as *mut MessageObj);
    let n = obj.count.get().saturating_sub(1);
    obj.count.set(n);
    if n == 0 {
        drop(Box::from_raw(this as *mut MessageObj));
    }
    n
}

unsafe extern "system" fn attrs_ref(this: *mut c_void) -> u32 {
    bump(&(&*(this as *const AttrListObj)).count)
}

// The attribute list lives inside the message; the plugin never gets a
// reference of its own, so the release only counts and never frees.
unsafe extern "system" fn attrs_release(this: *mut c_void) -> u32 {
    let n = (&*(this as *const AttrListObj)).count.get().saturating_sub(1);
    (&*(this as *const AttrListObj)).count.set(n);
    n
}

unsafe extern "system" fn message_get_id(this: *mut c_void) -> *const c_char {
    let obj = &*(this as *const MessageObj);
    obj.id.as_ptr() as *const c_char
}

unsafe extern "system" fn message_set_id(this: *mut c_void, id: *const c_char) {
    let obj = &mut *(this as *mut MessageObj);
    if id.is_null() {
        obj.id = Vec::new();
    } else {
        obj.id = CStr::from_ptr(id).to_bytes().to_vec();
    }
}

unsafe extern "system" fn message_get_attrs(this: *mut c_void) -> *mut c_void {
    let obj = &*(this as *const MessageObj);
    &*obj.attrs as *const AttrListObj as *mut c_void
}

unsafe fn attr_key(id: *const c_char) -> Vec<u8> {
    CStr::from_ptr(id).to_bytes().to_vec()
}

unsafe extern "system" fn attrs_set_int(this: *mut c_void, id: *const c_char, value: i64) -> i32 {
    let obj = &mut *(this as *mut AttrListObj);
    obj.map.insert(attr_key(id), AttrValue::Int(value));
    K_RESULT_OK
}

unsafe extern "system" fn attrs_get_int(this: *mut c_void, id: *const c_char, value: *mut i64) -> i32 {
    let obj = &*(this as *const AttrListObj);
    match obj.map.get(&attr_key(id)) {
        Some(AttrValue::Int(v)) => {
            if !value.is_null() {
                *value = *v;
            }
            K_RESULT_OK
        }
        _ => K_RESULT_FALSE,
    }
}

unsafe extern "system" fn attrs_set_float(this: *mut c_void, id: *const c_char, value: f64) -> i32 {
    let obj = &mut *(this as *mut AttrListObj);
    obj.map.insert(attr_key(id), AttrValue::Float(value));
    K_RESULT_OK
}

unsafe extern "system" fn attrs_get_float(this: *mut c_void, id: *const c_char, value: *mut f64) -> i32 {
    let obj = &*(this as *const AttrListObj);
    match obj.map.get(&attr_key(id)) {
        Some(AttrValue::Float(v)) => {
            if !value.is_null() {
                *value = *v;
            }
            K_RESULT_OK
        }
        _ => K_RESULT_FALSE,
    }
}

unsafe extern "system" fn attrs_set_string(this: *mut c_void, id: *const c_char, text: *const u16) -> i32 {
    if text.is_null() {
        return K_RESULT_INVALID_ARGUMENT;
    }
    let mut chars = Vec::new();
    let mut at = 0;
    loop {
        let c = *text.add(at);
        if c == 0 {
            break;
        }
        chars.push(c);
        at += 1;
        if at > 4096 {
            break;
        }
    }
    let obj = &mut *(this as *mut AttrListObj);
    obj.map.insert(attr_key(id), AttrValue::Text(chars));
    K_RESULT_OK
}

unsafe extern "system" fn attrs_get_string(this: *mut c_void, id: *const c_char, out: *mut u16, size_bytes: u32) -> i32 {
    let obj = &*(this as *const AttrListObj);
    let dirs = if let Some(AttrValue::Text(v)) = obj.map.get(&attr_key(id)) {
        v.clone()
    } else {
        return K_RESULT_FALSE;
    };
    // Size is in bytes; a nul always ends the copy.
    let room = (size_bytes / 2) as usize;
    let mut at = 0;
    for c in dirs {
        if at + 1 >= room {
            break;
        }
        *out.add(at) = c;
        at += 1;
    }
    if room > 0 {
        *out.add(at) = 0;
    }
    K_RESULT_OK
}

unsafe extern "system" fn attrs_set_binary(this: *mut c_void, id: *const c_char, data: *const c_void, size: u32) -> i32 {
    if data.is_null() {
        return K_RESULT_INVALID_ARGUMENT;
    }
    let bytes = std::slice::from_raw_parts(data as *const u8, size as usize).to_vec();
    let obj = &mut *(this as *mut AttrListObj);
    obj.map.insert(attr_key(id), AttrValue::Bin(bytes));
    K_RESULT_OK
}

unsafe extern "system" fn attrs_get_binary(this: *mut c_void, id: *const c_char, data: *mut *const c_void, size: *mut u32) -> i32 {
    // The data pointer handed back borrows the map entry, which lives until
    // the message is released: fine for the plugin's immediate read.
    let obj = &*(this as *const AttrListObj);
    match obj.map.get(&attr_key(id)) {
        Some(AttrValue::Bin(v)) => {
            if !data.is_null() {
                *data = v.as_ptr() as *const c_void;
            }
            if !size.is_null() {
                *size = v.len() as u32;
            }
            K_RESULT_OK
        }
        _ => K_RESULT_FALSE,
    }
}

impl MessageObj {
    /// The raw object pointer the plugin-side code sees.
    fn addr(&self) -> *mut c_void {
        self as *const MessageObj as *mut c_void
    }

    fn attr_addr(&self) -> *mut c_void {
        &*self.attrs as *const AttrListObj as *mut c_void
    }
}

unsafe impl Send for AttrListObj {}
unsafe impl Send for MessageObj {}

fn message_vt() -> &'static MessageVt {
    static VT: OnceLock<MessageVtKnown> = OnceLock::new();
    &VT.get_or_init(|| MessageVtKnown(MessageVt {
        query: message_query,
        add_ref: message_ref,
        release: message_release,
        get_id: message_get_id,
        set_id: message_set_id,
        get_attrs: message_get_attrs,
    })).0
}

struct AttrVtKnown(AttrVt);
unsafe impl Sync for AttrVtKnown {}

struct MessageVtKnown(MessageVt);
unsafe impl Sync for MessageVtKnown {}

fn attrs_vt() -> &'static AttrVt {
    static VT: OnceLock<AttrVtKnown> = OnceLock::new();
    &VT.get_or_init(|| AttrVtKnown(AttrVt {
        query: attrs_query,
        add_ref: attrs_ref,
        release: attrs_release,
        set_int: attrs_set_int,
        get_int: attrs_get_int,
        set_float: attrs_set_float,
        get_float: attrs_get_float,
        set_string: attrs_set_string,
        get_string: attrs_get_string,
        set_binary: attrs_set_binary,
        get_binary: attrs_get_binary,
    })).0
}

fn new_message() -> Box<MessageObj> {
    Box::new(MessageObj {
        vt: message_vt(),
        count: std::cell::Cell::new(1),
        id: Vec::new(),
        attrs: Box::new(AttrListObj {
            vt: attrs_vt(),
            count: std::cell::Cell::new(1),
            map: std::collections::HashMap::default(),
        }),
    })
}


const RESTART_PARAM_TITLES: i32 = 1 << 4;

#[repr(C)]
struct HandlerVt {
    query: TQuery,
    add_ref: TRef,
    release: TRef,
    begin_edit: unsafe extern "system" fn(*mut c_void, u32) -> i32,
    perform_edit: unsafe extern "system" fn(*mut c_void, u32, f64) -> i32,
    end_edit: unsafe extern "system" fn(*mut c_void, u32) -> i32,
    restart_component: unsafe extern "system" fn(*mut c_void, i32) -> i32,
}

#[repr(C)]
struct HandlerObj {
    vt: &'static HandlerVt,
    /// Flags the controller asked to be restarted with (it may run on
    /// its own thread, and `idle` reads this one at a time).
    restart: *mut AtomicU32,
    /// Values changed from the plugin's own window.
    edited: *mut Mutex<Vec<(u32, f64)>>,
}

unsafe extern "system" fn handler_query(this: *mut c_void, iid: *const c_void, out: *mut *mut c_void) -> i32 {
    unsafe { hand_out(this, iid, out, &[U_UNKNOWN, U_ICOMPONENT_HANDLER]) }
}

unsafe extern "system" fn handler_begin_edit(_: *mut c_void, _: u32) -> i32 {
    K_RESULT_OK
}

/// The plugin's own window moved a knob: remembered so the slider in the
/// Mixer follows, and the next block carries the value to the component.
unsafe extern "system" fn handler_perform_edit(this: *mut c_void, id: u32, value: f64) -> i32 {
    let obj = &*(this as *const HandlerObj);
    if !obj.edited.is_null() {
        if let Ok(mut list) = (*obj.edited).lock() {
            list.retain(|(i, _)| *i != id);
            if list.len() < 256 {
                list.push((id, value));
            }
        }
    }
    K_RESULT_OK
}

unsafe extern "system" fn handler_end_edit(_: *mut c_void, _: u32) -> i32 {
    K_RESULT_OK
}

unsafe extern "system" fn handler_restart(this: *mut c_void, flags: i32) -> i32 {
    let obj = &*(this as *const HandlerObj);
    if !obj.restart.is_null() {
        (*obj.restart).fetch_or(flags as u32, Ordering::Relaxed);
    }
    K_RESULT_OK
}

struct HandlerVtKnown(HandlerVt);
unsafe impl Sync for HandlerVtKnown {}

unsafe impl Send for HandlerObj {}

// The two fields point into the Instance, which owns the boxed atom; the
// Instance lives with the project (one thread at a time), so this is sound.
// The atomics keep a plugin thread from racing against `idle`.
fn new_handler(restart: *mut AtomicU32, edited: *mut Mutex<Vec<(u32, f64)>>) -> Box<HandlerObj> {
    static VT: OnceLock<HandlerVtKnown> = OnceLock::new();
    let vt = &VT.get_or_init(|| HandlerVtKnown(HandlerVt {
        query: handler_query,
        add_ref: host_ref,
        release: host_ref,
        begin_edit: handler_begin_edit,
        perform_edit: handler_perform_edit,
        end_edit: handler_end_edit,
        restart_component: handler_restart,
    })).0;
    Box::new(HandlerObj { vt, restart, edited })
}

// ---- IBStream, both directions (plugin state saving / loading) --------------

const SEEK_CUR: i32 = 1;
const SEEK_END: i32 = 2;

struct StreamVtKnown(StreamVt);
unsafe impl Sync for StreamVtKnown {}

#[repr(C)]
struct StreamVt {
    query: TQuery,
    add_ref: TRef,
    release: TRef,
    read: unsafe extern "system" fn(*mut c_void, *mut c_void, i32, *mut i32) -> i32,
    write: unsafe extern "system" fn(*mut c_void, *mut c_void, i32, *mut i32) -> i32,
    seek: unsafe extern "system" fn(*mut c_void, i64, i32, *mut i64) -> i32,
    tell: unsafe extern "system" fn(*mut c_void, *mut i64) -> i32,
}

#[repr(C)]
struct OutStreamObj {
    vt: &'static StreamVt,
    data: *mut Vec<u8>,
}

unsafe impl Send for OutStreamObj {}

unsafe extern "system" fn outstream_query(this: *mut c_void, iid: *const c_void, out: *mut *mut c_void) -> i32 {
    unsafe { hand_out(this, iid, out, &[U_UNKNOWN, U_IBSTREAM]) }
}

unsafe extern "system" fn outstream_read(_: *mut c_void, _: *mut c_void, _: i32, read: *mut i32) -> i32 {
    if !read.is_null() {
        *read = 0;
    }
    K_RESULT_FALSE
}

unsafe extern "system" fn outstream_write(this: *mut c_void, buf: *mut c_void, num: i32, written: *mut i32) -> i32 {
    let obj = &*(this as *const OutStreamObj);
    if !obj.data.is_null() {
        let src = std::slice::from_raw_parts(buf as *const u8, num.max(0) as usize);
        (*obj.data).extend_from_slice(src);
    }
    if !written.is_null() {
        *written = num;
    }
    K_RESULT_OK
}

unsafe extern "system" fn outstream_seek(this: *mut c_void, pos: i64, mode: i32, result: *mut i64) -> i32 {
    let obj = &*(this as *const OutStreamObj);
    if !obj.data.is_null() {
        let data = &mut *obj.data;
        let at = match mode {
            SEEK_CUR => data.len() as i64 + pos,
            SEEK_END => data.len() as i64 + pos,
            _ => pos, // SEEK_SET
        } as usize;
        data.resize(at, 0);
        if !result.is_null() {
            *result = data.len() as i64;
        }
    } else if !result.is_null() {
        *result = 0;
    }
    K_RESULT_OK
}

unsafe extern "system" fn outstream_tell(this: *mut c_void, pos: *mut i64) -> i32 {
    let obj = &*(this as *const OutStreamObj);
    if !pos.is_null() {
        *pos = if obj.data.is_null() { 0 } else { (*obj.data).len() as i64 };
    }
    K_RESULT_OK
}

/// A growing byte stream handed to `getState`.
fn outstream(data: *mut Vec<u8>) -> Box<OutStreamObj> {
    static VT: OnceLock<StreamVtKnown> = OnceLock::new();
    let vt = &VT.get_or_init(|| StreamVtKnown(StreamVt {
        query: outstream_query,
        add_ref: host_ref,
        release: host_ref,
        read: outstream_read,
        write: outstream_write,
        seek: outstream_seek,
        tell: outstream_tell,
    })).0;
    Box::new(OutStreamObj { vt, data })
}

#[repr(C)]
struct InStreamObj {
    vt: &'static StreamVt,
    data: *const u8,
    len: usize,
    at: usize,
}

unsafe impl Send for InStreamObj {}

unsafe extern "system" fn instream_query(this: *mut c_void, iid: *const c_void, out: *mut *mut c_void) -> i32 {
    unsafe { hand_out(this, iid, out, &[U_UNKNOWN, U_IBSTREAM]) }
}

unsafe extern "system" fn instream_read(this: *mut c_void, buf: *mut c_void, num: i32, read: *mut i32) -> i32 {
    let obj = &mut *(this as *mut InStreamObj);
    let n = (num.max(0) as usize).min(obj.len.saturating_sub(obj.at));
    std::ptr::copy_nonoverlapping(obj.data.add(obj.at), buf as *mut u8, n);
    obj.at += n;
    if !read.is_null() {
        *read = n as i32;
    }
    if n < num as usize {
        K_RESULT_FALSE
    } else {
        K_RESULT_OK
    }
}

unsafe extern "system" fn instream_write(_: *mut c_void, _: *mut c_void, _: i32, written: *mut i32) -> i32 {
    if !written.is_null() {
        *written = 0;
    }
    K_RESULT_FALSE
}

unsafe extern "system" fn instream_seek(this: *mut c_void, pos: i64, mode: i32, result: *mut i64) -> i32 {
    let obj = &mut *(this as *mut InStreamObj);
    let base = match mode {
        SEEK_CUR => obj.at as i64,
        SEEK_END => obj.len as i64,
        _ => 0, // SEEK_SET
    };
    obj.at = (base + pos).clamp(0, obj.len as i64) as usize;
    if !result.is_null() {
        *result = obj.at as i64;
    }
    K_RESULT_OK
}

unsafe extern "system" fn instream_tell(this: *mut c_void, pos: *mut i64) -> i32 {
    let obj = &*(this as *const InStreamObj);
    if !pos.is_null() {
        *pos = obj.at as i64;
    }
    K_RESULT_OK
}

/// A reading stream for `setState` / `setComponentState`.
fn instream(data: &[u8]) -> Box<InStreamObj> {
    static VT: OnceLock<StreamVtKnown> = OnceLock::new();
    let vt = &VT.get_or_init(|| StreamVtKnown(StreamVt {
        query: instream_query,
        add_ref: host_ref,
        release: host_ref,
        read: instream_read,
        write: instream_write,
        seek: instream_seek,
        tell: instream_tell,
    })).0;
    Box::new(InStreamObj { vt, data: data.as_ptr(), len: data.len(), at: 0 })
}

// ---------------------------------------------------------------- process ABI

const PROCESS_MODE_REALTIME: i32 = 0;
const SYMBOLIC_SAMPLE_32: i32 = 0;

#[repr(C)]
struct ProcessSetup {
    process_mode: i32,
    symbolic_sample_size: i32,
    max_samples_per_block: i32,
    sample_rate: f64,
}

#[repr(C)]
struct AudioBusBuffers {
    num_channels: i32,
    silence_flags: u64,
    channel_buffers32: *mut *mut f32,
}

#[repr(C)]
struct ProcessData {
    process_mode: i32,
    symbolic_sample_size: i32,
    num_samples: i32,
    num_inputs: i32,
    num_outputs: i32,
    inputs: *mut AudioBusBuffers,
    outputs: *mut AudioBusBuffers,
    input_parameter_changes: *mut c_void,
    output_parameter_changes: *mut c_void,
    input_events: *mut c_void,
    output_events: *mut c_void,
    process_context: *mut ProcessContext,
}

// Probe numbers: AudioBusBuffers 24 (buffers at 16), ProcessData 80
// (inputs at 24, the rest each 8), ProcessContext 112 (tempo at 72,
// time signature at 80, chord at 88, samples_to_next_clock at 104).
#[repr(C)]
struct ProcessContext {
    state: u32,
    sample_rate: f64,
    project_time_samples: i64,
    system_time: i64,
    continuous_time_samples: i64,
    project_time_music: f64,
    bar_position_music: f64,
    cycle_start_music: f64,
    cycle_end_music: f64,
    tempo: f64,
    time_sig_numerator: i32,
    time_sig_denominator: i32,
    chord_key: u8,
    chord_root: u8,
    chord_mask: i16,
    smpte_offset_subframes: i32,
    frame_rate_fps: u32,
    frame_rate_flags: u32,
    samples_to_next_clock: i32,
}

#[repr(C)]
union EventPayload {
    note_on: NoteOnEvent,
    note_off: NoteOffEvent,
    poly_pressure: PolyPressureEvent,
    // 24 bytes wide and 8-byte aligned on purpose: the real union also holds
    // DataEvent whose bytes pointer makes the whole union 8-byte aligned, so
    // plugins expect it at this exact offset inside Event.
    filler: [u64; 3],
}

impl Clone for EventPayload {
    fn clone(&self) -> Self {
        // Moving a union copies its bytes; only reading a member needs unsafe.
        *self
    }
}
impl Copy for EventPayload {}

#[repr(C)]
#[derive(Clone, Copy)]
struct NoteOnEvent {
    channel: i16,
    pitch: i16,
    tuning: f32,
    velocity: f32,
    length: i32,
    note_id: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NoteOffEvent {
    channel: i16,
    pitch: i16,
    velocity: f32,
    note_id: i32,
    tuning: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PolyPressureEvent {
    channel: i16,
    pitch: i16,
    pressure: f32,
    note_id: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Event {
    bus_index: i32,
    sample_offset: i32,
    ppq_position: f64,
    flags: u16,
    kind: u16,
    payload: EventPayload,
}

impl Event {
    fn note(kind: u16, at: u32, key: i16, velocity: f32) -> Event {
        let mut e = Event {
            bus_index: 0,
            sample_offset: at as i32,
            ppq_position: 0.0,
            flags: 0,
            kind,
            payload: EventPayload { filler: [0; 3] },
        };
        // Writing a union field is safe in Rust; only reading one is unsafe.
        if kind == EVENT_NOTE_ON {
            e.payload.note_on = NoteOnEvent { channel: 0, pitch: key, tuning: 0.0, velocity, length: 0, note_id: -1 };
        } else if kind == EVENT_NOTE_OFF {
            e.payload.note_off = NoteOffEvent { channel: 0, pitch: key, velocity: 0.0, note_id: -1, tuning: 0.0 };
        }
        e
    }
}

#[repr(C)]
struct ParameterInfo {
    id: u32,
    title: [u16; 128],
    short_title: [u16; 128],
    units: [u16; 128],
    step_count: i32,
    default_normalized_value: f64,
    unit_id: i32,
    flags: i32,
}

#[repr(C)]
struct BusInfo {
    media_type: i32,
    direction: i32,
    channel_count: i32,
    name: [u16; 128],
    bus_type: i32,
    flags: u32,
}

#[repr(C)]
struct PFactoryInfo {
    vendor: [u8; 64],
    url: [u8; 256],
    email: [u8; 128],
    flags: i32,
}

#[repr(C)]
struct PClassInfo {
    cid: [u8; 16],
    cardinality: i32,
    category: [u8; 32],
    name: [u8; 64],
}

#[repr(C)]
struct PClassInfo2 {
    cid: [u8; 16],
    cardinality: i32,
    category: [u8; 32],
    name: [u8; 64],
    class_flags: u32,
    sub_categories: [u8; 128],
    vendor: [u8; 64],
    version: [u8; 64],
    sdk_version: [u8; 64],
}

const CATEGORY_AUDIO_MODULE: &str = "Audio Module Class";

fn cbuf(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

fn u16str(buf: &[u16; 128]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

// host objects: parameter change lists and queues ----------------------------

#[repr(C)]
struct QueueObj {
    vt: &'static QueueVt,
    id: u32,
    points: *mut Vec<(i32, f64)>,
}

#[repr(C)]
struct QueueVt {
    query: TQuery,
    add_ref: TRef,
    release: TRef,
    get_parameter_id: unsafe extern "system" fn(*mut c_void) -> u32,
    get_point_count: unsafe extern "system" fn(*mut c_void) -> i32,
    get_point: unsafe extern "system" fn(*mut c_void, i32, *mut i32, *mut f64) -> i32,
    add_point: unsafe extern "system" fn(*mut c_void, i32, f64, *mut i32) -> i32,
}

unsafe impl Send for QueueObj {}

unsafe extern "system" fn queue_query(this: *mut c_void, iid: *const c_void, out: *mut *mut c_void) -> i32 {
    unsafe { hand_out(this, iid, out, &[U_UNKNOWN, U_IPARAM_VALUE_QUEUE]) }
}

unsafe extern "system" fn queue_id(this: *mut c_void) -> u32 {
    let obj = &*(this as *const QueueObj);
    obj.id
}

unsafe extern "system" fn queue_count(this: *mut c_void) -> i32 {
    let obj = &*(this as *const QueueObj);
    if obj.points.is_null() { 0 } else { (&*obj.points).len() as i32 }
}

unsafe extern "system" fn queue_point(this: *mut c_void, index: i32, offset: *mut i32, value: *mut f64) -> i32 {
    let obj = &*(this as *const QueueObj);
    if obj.points.is_null() {
        return K_RESULT_INVALID_ARGUMENT;
    }
    let points = &*obj.points;
    match points.get(index.max(0) as usize) {
        Some((at, val)) => {
            if !offset.is_null() {
                *offset = *at;
            }
            if !value.is_null() {
                *value = *val;
            }
            K_RESULT_OK
        }
        None => K_RESULT_INVALID_ARGUMENT,
    }
}

unsafe extern "system" fn queue_add(this: *mut c_void, offset: i32, value: f64, index: *mut i32) -> i32 {
    let obj = &*(this as *const QueueObj);
    if !obj.points.is_null() {
        let points = &mut *obj.points;
        if let Some(at) = points.iter().position(|p| p.0 == offset) {
            points[at] = (offset, value);
            if !index.is_null() {
                *index = at as i32;
            }
        } else {
            points.retain(|p| p.0 <= offset);
            points.push((offset, value));
            if !index.is_null() {
                *index = points.len() as i32 - 1;
            }
        }
    }
    K_RESULT_OK
}

struct QueueVtKnown(QueueVt);
unsafe impl Sync for QueueVtKnown {}

fn queue_vt() -> &'static QueueVt {
    static VT: OnceLock<QueueVtKnown> = OnceLock::new();
    &VT.get_or_init(|| QueueVtKnown(QueueVt {
        query: queue_query,
        add_ref: host_ref,
        release: host_ref,
        get_parameter_id: queue_id,
        get_point_count: queue_count,
        get_point: queue_point,
        add_point: queue_add,
    })).0
}

/// IParameterChanges: the queues of one block. New queues are handed out as
/// bare pointers and live exactly until the block is over.
#[repr(C)]
struct ParamChangesObj {
    vt: &'static ParamChangesVt,
    queues: *mut Vec<QueueObj>,
}

#[repr(C)]
struct ParamChangesVt {
    query: TQuery,
    add_ref: TRef,
    release: TRef,
    get_parameter_count: unsafe extern "system" fn(*mut c_void) -> i32,
    get_parameter_data: unsafe extern "system" fn(*mut c_void, i32) -> *mut c_void,
    add_parameter_data: unsafe extern "system" fn(*mut c_void, *const u32, *mut i32) -> *mut c_void,
}

unsafe impl Send for ParamChangesObj {}

unsafe extern "system" fn changes_query(this: *mut c_void, iid: *const c_void, out: *mut *mut c_void) -> i32 {
    unsafe { hand_out(this, iid, out, &[U_UNKNOWN, U_IPARAMETER_CHANGES]) }
}

unsafe extern "system" fn changes_count(this: *mut c_void) -> i32 {
    let obj = &*(this as *const ParamChangesObj);
    if obj.queues.is_null() { 0 } else { (&*obj.queues).len() as i32 }
}

unsafe extern "system" fn changes_get(this: *mut c_void, index: i32) -> *mut c_void {
    let obj = &*(this as *const ParamChangesObj);
    if obj.queues.is_null() {
        return ptr::null_mut();
    }
    let queues = &*obj.queues;
    match queues.get(index.max(0) as usize) {
        Some(q) => q as *const QueueObj as *mut c_void,
        None => ptr::null_mut(),
    }
}

unsafe extern "system" fn changes_add(this: *mut c_void, id: *const u32, index: *mut i32) -> *mut c_void {
    let obj = &*(this as *const ParamChangesObj);
    if obj.queues.is_null() {
        return ptr::null_mut();
    }
    let queues = &mut *obj.queues;
    let slot = match queues.iter().position(|q| q.id == *id) {
        Some(at) => {
            if !index.is_null() {
                *index = at as i32;
            }
            &mut queues[at]
        }
        None => {
            // One queue per changed parameter; ways over 192 in a block would
            // be a very chatty plugin, so the list is bounded.
            if queues.len() >= 192 {
                if !index.is_null() {
                    *index = -1;
                }
                return ptr::null_mut();
            }
            queues.push(QueueObj { vt: queue_vt(), id: *id, points: ptr::null_mut() });
            if !index.is_null() {
                *index = (queues.len() - 1) as i32;
            }
            queues.last_mut().unwrap()
        }
    };
    if slot.points.is_null() {
        slot.points = Box::into_raw(Box::new(Vec::new()));
    }
    slot as *mut QueueObj as *mut c_void
}

struct ParamChangesVtKnown(ParamChangesVt);
unsafe impl Sync for ParamChangesVtKnown {}

fn param_changes_vt() -> &'static ParamChangesVt {
    static VT: OnceLock<ParamChangesVtKnown> = OnceLock::new();
    &VT.get_or_init(|| ParamChangesVtKnown(ParamChangesVt {
        query: changes_query,
        add_ref: host_ref,
        release: host_ref,
        get_parameter_count: changes_count,
        get_parameter_data: changes_get,
        add_parameter_data: changes_add,
    })).0
}

// host object: the event list ------------------------------------------------

#[repr(C)]
struct EventListVt {
    query: TQuery,
    add_ref: TRef,
    release: TRef,
    get_event_count: unsafe extern "system" fn(*mut c_void) -> i32,
    get_event: unsafe extern "system" fn(*mut c_void, i32, *mut Event) -> i32,
    add_event: unsafe extern "system" fn(*mut c_void, *mut Event) -> i32,
}

#[repr(C)]
struct EventListObj {
    vt: &'static EventListVt,
    events: *mut Vec<Event>,
}

unsafe impl Send for EventListObj {}

unsafe extern "system" fn elist_query(this: *mut c_void, iid: *const c_void, out: *mut *mut c_void) -> i32 {
    unsafe { hand_out(this, iid, out, &[U_UNKNOWN, U_IEVENT_LIST]) }
}

unsafe extern "system" fn elist_count(this: *mut c_void) -> i32 {
    let obj = &*(this as *const EventListObj);
    if obj.events.is_null() { 0 } else { (&*obj.events).len() as i32 }
}

unsafe extern "system" fn elist_get(this: *mut c_void, index: i32, out: *mut Event) -> i32 {
    let obj = &*(this as *const EventListObj);
    if obj.events.is_null() {
        return K_RESULT_INVALID_ARGUMENT;
    }
    let events = &*obj.events;
    match events.get(index.max(0) as usize) {
        Some(e) => {
            *out = *e;
            K_RESULT_OK
        }
        None => K_RESULT_INVALID_ARGUMENT,
    }
}

unsafe extern "system" fn elist_add(this: *mut c_void, e: *mut Event) -> i32 {
    let obj = &mut *(this as *mut EventListObj);
    if !obj.events.is_null() {
        let list = &mut *obj.events;
        if list.len() < 1024 {
            list.push(*e);
        }
    }
    K_RESULT_OK
}

struct EventListVtKnown(EventListVt);
unsafe impl Sync for EventListVtKnown {}

fn event_list_vt() -> &'static EventListVt {
    static VT: OnceLock<EventListVtKnown> = OnceLock::new();
    &VT.get_or_init(|| EventListVtKnown(EventListVt {
        query: elist_query,
        add_ref: host_ref,
        release: host_ref,
        get_event_count: elist_count,
        get_event: elist_get,
        add_event: elist_add,
    })).0
}

// ---------------------------------------------------------------- libraries

/// A loaded plugin file. Libraries are never unloaded: some plugins keep
/// threads running after their last instance is gone.
#[derive(Clone, Copy)]
struct Lib {
    _handle: *mut c_void,
    _module_exit: Option<unsafe extern "system" fn()>,
    factory: *mut c_void,
}

unsafe impl Send for Lib {}

impl Lib {
    fn open(path: &Path) -> Result<Lib, String> {
        let handle = crate::dynlib::Lib::open(path)?;
        let fp = handle.sym(c"ModuleEntry");
        if fp.is_null() {
            // New-format bundles export ModuleEntry/ModuleExit; without it
            // the file is not a loadable VST3.
            return Err("not a VST3 plugin".into());
        }
        let module_entry: unsafe extern "system" fn(*mut c_void) -> bool = unsafe { std::mem::transmute_copy(&fp) };
        let exit_fp = handle.sym(c"ModuleExit");
        let module_exit: Option<unsafe extern "system" fn()> = (!exit_fp.is_null())
            .then(|| unsafe { std::mem::transmute_copy(&exit_fp) });
        let fp = handle.sym(c"GetPluginFactory");
        if fp.is_null() {
            return Err("not a VST3 plugin".into());
        }
        let get_factory: unsafe extern "system" fn() -> *mut c_void = unsafe { std::mem::transmute_copy(&fp) };
        // The module must be armed before the factory comes out.
        if !unsafe { module_entry(handle.raw()) } {
            return Err("the plugin refused to start".into());
        }
        let factory = unsafe { get_factory() };
        if factory.is_null() {
            return Err("the plugin has no factory".into());
        }
        Ok(Lib { _handle: handle.raw(), _module_exit: module_exit, factory })
    }

    fn factory_info(&self) -> PFactoryInfo {
        let Some(factory) = Obj::of(self.factory) else { return unsafe { std::mem::zeroed() } };
        let mut info: PFactoryInfo = unsafe { std::mem::zeroed() };
        let f: unsafe extern "system" fn(*mut c_void, *mut PFactoryInfo) -> i32 = unsafe { factory.slot(3) };
        unsafe { f(factory.0, &mut info) };
        info
    }

    fn class_count(&self) -> i32 {
        let Some(factory) = Obj::of(self.factory) else { return 0 };
        let f: unsafe extern "system" fn(*mut c_void) -> i32 = unsafe { factory.slot(4) };
        unsafe { f(factory.0) }
    }

    fn class_info2(&self, index: i32) -> Option<PClassInfo2> {
        let factory = Obj::of(self.factory)?;
        let f2 = factory.query(U_IPLUGIN_FACTORY_2)?;
        let mut info: PClassInfo2 = unsafe { std::mem::zeroed() };
        let f: unsafe extern "system" fn(*mut c_void, i32, *mut PClassInfo2) -> i32 = unsafe { f2.slot(7) };
        (unsafe { f(f2.0, index, &mut info) } == K_RESULT_OK).then_some(info)
    }

    fn class_info(&self, index: i32) -> Option<PClassInfo> {
        let factory = Obj::of(self.factory)?;
        let mut info: PClassInfo = unsafe { std::mem::zeroed() };
        let f: unsafe extern "system" fn(*mut c_void, i32, *mut PClassInfo) -> i32 = unsafe { factory.slot(5) };
        (unsafe { f(factory.0, index, &mut info) } == K_RESULT_OK).then_some(info)
    }

    /// Creates an object of a class. `interface` names a GUID by hex text.
    fn create(&self, cid: &Tuid, interface: &[u8; 32]) -> Option<Obj> {
        let factory = Obj::of(self.factory)?;
        let f: unsafe extern "system" fn(*mut c_void, *const c_void, *const c_void, *mut *mut c_void) -> i32 = unsafe { factory.slot(6) };
        let mut out: *mut c_void = ptr::null_mut();
        let r = unsafe { f(factory.0, cid.as_ptr(), Tuid::hex(interface).as_ptr(), &mut out) };
        if r == K_RESULT_OK {
            Obj::of(out)
        } else {
            None
        }
    }
}

fn libraries() -> &'static Mutex<HashMap<PathBuf, Lib>> {
    static LIBS: OnceLock<Mutex<HashMap<PathBuf, Lib>>> = OnceLock::new();
    LIBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn with_lib<T>(path: &Path, f: impl FnOnce(&Lib) -> Result<T, String>) -> Result<T, String> {
    let mut libs = libraries().lock().map_err(|_| "plugin list is broken")?;
    let lib = match libs.get(path) {
        Some(lib) => lib,
        None => {
            let file = bundle_library(path).ok_or_else(|| format!("no library inside {}", path.display()))?;
            let lib = Lib::open(&file)?;
            libs.insert(path.to_path_buf(), lib);
            &libs[path]
        }
    };
    f(lib)
}

/// The inner library of a plugin: `Name.vst3/Contents/x86_64-linux/*.so` on
/// Linux, a `*.dll` or `*.vst3` file on Windows, `Contents/MacOS/*` on macOS.
/// A single flat file also counts (some hosts accept that shape).
pub fn bundle_library(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    if !path.is_dir() {
        return None;
    }
    #[cfg(target_os = "macos")]
    let arch_dir = "MacOS".to_string();
    #[cfg(not(target_os = "macos"))]
    let arch_dir = {
        let arch = if cfg!(target_arch = "x86_64") {
            "x86_64"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64"
        } else if cfg!(target_arch = "x86") {
            "i386"
        } else if cfg!(target_arch = "arm") {
            "arm"
        } else {
            "i686"
        };
        if cfg!(target_os = "windows") {
            format!("{arch}-win")
        } else {
            format!("{arch}-linux")
        }
    };

    let contents = path.join("Contents");
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut best: Option<(u8, u8, PathBuf)> = None; // name score, extension score, file
    for dir in [contents.join(&arch_dir), contents.join("MacOS")] {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = p.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let good = if cfg!(target_os = "windows") {
                ext.eq_ignore_ascii_case("vst3") || ext.eq_ignore_ascii_case("dll")
            } else if cfg!(target_os = "macos") {
                ext == "so" || ext.is_empty() || ext.eq_ignore_ascii_case("vst3")
            } else {
                ext == "so"
            };
            if !good {
                continue;
            }
            let name_score = if name == stem {
                3
            } else if name.starts_with(&format!("{stem}.")) {
                2
            } else if name.starts_with("lib") {
                1
            } else {
                0
            };
            let ext_score = if ext.eq_ignore_ascii_case("so") {
                2
            } else if ext.is_empty() {
                1
            } else {
                0
            };
            let better = match &best {
                None => true,
                Some((ns, es, bp)) => {
                    let bn = bp.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    (name_score, ext_score) > (*ns, *es) || ((name_score, ext_score) == (*ns, *es) && name < bn)
                }
            };
            if better {
                best = Some((name_score, ext_score, p));
            }
        }
        if best.is_some() {
            break;
        }
    }
    best.map(|(_, _, p)| p)
}

// ---------------------------------------------------------------- scanning

/// Folders VST3 plugins are installed in, per platform.
pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(extra) = std::env::var_os("VST3_PATH") {
        dirs.extend(std::env::split_paths(&extra));
    }
    let home = crate::settings::home_dir();
    if cfg!(windows) {
        if let Some(common) = std::env::var_os("COMMONPROGRAMFILES") {
            dirs.push(PathBuf::from(common).join("VST3"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Programs").join("Common").join("VST3"));
        }
    } else if cfg!(target_os = "macos") {
        dirs.push(home.join("Library/Audio/Plug-Ins/VST3"));
        dirs.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
    } else {
        dirs.push(home.join(".vst3"));
        dirs.push(PathBuf::from("/usr/lib/vst3"));
        dirs.push(PathBuf::from("/usr/local/lib/vst3"));
    }
    dirs
}

/// True for a `.vst3` file or bundle folder that holds a library.
pub fn is_plugin_file(path: &Path) -> bool {
    let is = path.extension().is_some_and(|s| s.eq_ignore_ascii_case("vst3"));
    is && (path.is_file() || bundle_library(path).is_some())
}

/// Collects plugin files under `dir` (called for each folder of dirs).
pub fn walk_dir(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if is_plugin_file(&p) {
            out.push(p);
        } else if p.is_dir() && depth < 3 {
            walk_dir(&p, depth + 1, out);
        }
    }
}

/// Runs in the child process: prints one line per plugin in the file.
pub fn scan_here(path: &Path) -> Result<(), String> {
    let lines = with_lib(path, |lib| {
        let mut out = Vec::new();
        let factory_vendor = cbuf(&lib.factory_info().vendor);
        for i in 0..lib.class_count() {
            let (cid, category, name, sub, vendor) = match lib.class_info2(i) {
                Some(info) => {
                    let vendor = cbuf(&info.vendor);
                    (
                        info.cid,
                        cbuf(&info.category),
                        cbuf(&info.name),
                        cbuf(&info.sub_categories),
                        if vendor.is_empty() { factory_vendor.clone() } else { vendor },
                    )
                }
                None => {
                    let Some(info) = lib.class_info(i) else { continue };
                    (info.cid, cbuf(&info.category), cbuf(&info.name), String::new(), factory_vendor.clone())
                }
            };
            if category != CATEGORY_AUDIO_MODULE {
                continue;
            }
            let instrument = sub.split(['|', ' ', ',']).any(|s| s.eq_ignore_ascii_case("Instrument"))
                // Old class info has no categories: an instrument is a class
                // without audio inputs.
                || (sub.is_empty() && is_instrument_by_buses(lib, &Tuid(cid)));
            out.push(format!(
                "{}\t{}\t{}\t{}",
                Tuid(cid).text(),
                name,
                vendor,
                if instrument { "instrument" } else { "effect" }
            ));
        }
        Ok(out)
    })?;
    for line in lines {
        println!("VST3\t{line}");
    }
    Ok(())
}

/// Creates the object just to count its buses; only used when the class info
/// (which knows "Instrument" already) is not there.
fn is_instrument_by_buses(lib: &Lib, cid: &Tuid) -> bool {
    let Some(comp) = lib.create(cid, U_ICOMPONENT) else { return false };
    let f: unsafe extern "system" fn(*mut c_void, i32, i32) -> i32 = unsafe { comp.slot(7) };
    let ins = unsafe { f(comp.0, BUS_MEDIA_AUDIO, BUS_DIRECTION_INPUT) };
    let outs = unsafe { f(comp.0, BUS_MEDIA_AUDIO, BUS_DIRECTION_OUTPUT) };
    ins == 0 && outs > 0
}

// ---------------------------------------------------------------- instance

/// One running VST3 plugin: the component (sound), usually a separate
/// controller object (parameters), and our host objects for both. Created,
/// activated and destroyed on the interface thread; `run` runs on the sound
/// thread (never both at once: the project lock is held for both), same
/// rules as the CLAP layer.
pub struct Instance {
    comp: Obj,
    ctrl: Option<Obj>,
    processor: Option<Obj>,
    to_release: Vec<Obj>,
    // Kept alive only: the plugin holds a pointer into this host object.
    _host: Host,
    handler: Box<HandlerObj>,
    restart: Box<AtomicU32>,
    edited: Box<Mutex<Vec<(u32, f64)>>>,
    info: crate::plugin::PluginInfo,
    params: Vec<crate::plugin::Param>,
    in_buses: i32,
    out_buses: i32,
    event_in_buses: i32,
    in_channels: Vec<u32>,
    out_channels: Vec<u32>,
    rate: f64,
    processing: bool,
    steady: i64,
    /// Notes waiting for the next block, with their offset in it.
    events: Vec<Event>,
    /// Parameter values waiting for the next block (normalized 0..1).
    pending: Vec<(u32, f64)>,
    started: Instant,
    /// Inputs then outputs, one line per channel, MAX_FRAMES long.
    scratch: Vec<Vec<f32>>,
}

unsafe impl Send for Instance {}

impl Instance {
    pub fn new(info: &crate::plugin::PluginInfo, rate: f64, state: Option<&[u8]>) -> Result<Instance, String> {
        let host = new_host();
        let mut restart = Box::new(AtomicU32::new(0u32));
        let mut edited = Box::<Mutex<Vec<(u32, f64)>>>::default();
        let handler = new_handler(&mut *restart, &mut *edited);
        let cid = uid_from_hex(&info.id).ok_or_else(|| "bad plugin class id".to_string())?;
        let origin = info.clone();

        // Component and controller (see plugprovider.cpp): create, initialize,
        // then the controller either hidden inside the component or as another
        // class of its own, also initialized.
        let (comp, ctrl, processor, cx_comp) = with_lib(&info.path, |lib| {
            let comp = lib.create(&cid, U_ICOMPONENT).ok_or_else(|| format!("{} could not be created", origin.name))?;
            let initialize: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32 = unsafe { comp.slot(3) };
            if unsafe { initialize(comp.0, host.ptr()) } != K_RESULT_OK {
                comp.release();
                return Err(format!("{} failed to start", origin.name));
            }
            // The class id of the controller part (some builds only answer
            // the parameters along the older JUCE iid, so keep both).
            let mut class_id = Tuid([0; 16]);
            let gid: unsafe extern "system" fn(*mut c_void, *mut u8) -> i32 = unsafe { comp.slot(5) };
            let gr = unsafe { gid(comp.0, class_id.0.as_mut_ptr()) };
            let class_id = (gr == K_RESULT_OK).then_some(class_id);
            let ctrl = match comp.query(U_IEDIT_CONTROLLER).or_else(|| comp.query(U_IEDIT_CONTROLLER_ALT)) {
                // One class doing both jobs: no second object.
                Some(c) => Some(c),
                None => class_id
                    .as_ref()
                    .and_then(|cid| lib.create(cid, U_IEDIT_CONTROLLER).or_else(|| lib.create(cid, U_IEDIT_CONTROLLER_ALT))),
            };
            if let Some(c) = &ctrl {
                let initialize: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32 = unsafe { c.slot(3) };
                unsafe { initialize(c.0, host.ptr()) };
            }
            // Split wrappers that listen for each other over messages (the
            // JUCE built ones) introduce the controller to the component's
            // sound part with a pointer message instead of plain connect
            // queries; hand the pointer over the way the controller itself
            // would (other plugins ignore the message id).
            let cx_comp = comp.query(U_ICONNECTION_POINT);
            if let (Some(cxp), Some(c)) = (&cx_comp, &ctrl) {
                let full = class_id.as_ref().and_then(|cid| c.query_bytes(&cid.0));
                if let Some(full) = full {
                    let m = new_message();
                    let tag = std::ffi::CString::new("JuceVST3EditController").unwrap();
                    unsafe {
                        (m.vt.set_id)(m.addr(), tag.as_ptr());
                        (m.attrs.vt.set_int)(m.attr_addr(), tag.as_ptr(), full.0 as i64);
                        let notify: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32 = cxp.slot(5);
                        let _ = notify(cxp.0, m.addr());
                        full.release();
                        // The message's one reference is the box `m` itself:
                        // dropping it at the end of the block lets it go once.
                    }
                }
            }
            let processor = comp.query(U_IAUDIO_PROCESSOR);
            Ok((comp, ctrl, processor, cx_comp))
        })?;

        // The two parts of big plugins sometimes talk through connection
        // points; optional everywhere, so failures are fine.
        let cx_ctrl = ctrl.as_ref().and_then(|c| c.query(U_ICONNECTION_POINT));
        if let (Some(a), Some(b)) = (&cx_comp, &cx_ctrl) {
            let connect: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32 = unsafe { a.slot(3) };
            unsafe { connect(a.0, b.0) };
            unsafe { connect(b.0, a.0) };
        }
        // Every queryInterface answer carries an extra reference; it all goes
        // back at the end (Drop).
        let mut to_release: Vec<Obj> = Vec::new();
        for extra in [processor, cx_comp, cx_ctrl].into_iter().flatten() {
            to_release.push(extra);
        }

        let mut inst = Instance {
            comp,
            ctrl,
            processor,
            to_release,
            _host: host,
            handler,
            restart,
            edited,
            info: origin,
            params: Vec::new(),
            in_buses: 0,
            out_buses: 0,
            event_in_buses: 0,
            in_channels: Vec::new(),
            out_channels: Vec::new(),
            rate: 0.0,
            processing: false,
            steady: 0,
            events: Vec::new(),
            pending: Vec::new(),
            started: Instant::now(),
            scratch: Vec::new(),
        };
        // --------------------------------------------------------------
        // The controller wants its callback object to report edits and
        // restarts (setComponentHandler).
        if let Some(c) = &inst.ctrl {
            let set_handler: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32 = unsafe { c.slot(16) };
            unsafe { set_handler(c.0, &*inst.handler as *const HandlerObj as *mut c_void) };
        }
        inst.read_ports();
        inst.read_params();
        // Saved state goes in before anything sounds.
        if let Some(s) = state {
            inst.load_state(s);
        }
        inst.set_rate(rate)?;
        Ok(inst)
    }

    /// The interface that answers the sound functions: the dedicated query
    /// result when the plugin answered it, else the component (which carries
    /// IAudioProcessor in most plugins even when the query refuses).
    fn processor_obj(&self) -> Obj {
        self.processor.unwrap_or(self.comp)
    }

    fn set_processing(&mut self, on: bool) {
        let p = self.processor_obj();
        let f: unsafe extern "system" fn(*mut c_void, u8) -> i32 = unsafe { p.slot(8) };
        unsafe { f(p.0, on as u8) };
        self.processing = on;
    }

    /// New sound settings: setupProcessing, wake the buses, setActive.
    fn set_rate(&mut self, rate: f64) -> Result<(), String> {
        if self.rate != 0.0 {
            if self.processing {
                self.set_processing(false);
            }
            let off: unsafe extern "system" fn(*mut c_void, u8) -> i32 = unsafe { self.comp.slot(11) };
            unsafe { off(self.comp.0, 0) };
            self.rate = 0.0;
        }
        let p = self.processor_obj();
        let mut setup = ProcessSetup {
            process_mode: PROCESS_MODE_REALTIME,
            symbolic_sample_size: SYMBOLIC_SAMPLE_32,
            max_samples_per_block: MAX_FRAMES as i32,
            sample_rate: rate,
        };
        let setup_f: unsafe extern "system" fn(*mut c_void, *mut ProcessSetup) -> i32 = unsafe { p.slot(7) };
        if unsafe { setup_f(p.0, &mut setup) } != K_RESULT_OK {
            return Err(format!("{} refused the sound settings", self.info.name));
        }
        let activate: unsafe extern "system" fn(*mut c_void, i32, i32, i32, u8) -> i32 = unsafe { self.comp.slot(10) };
        for (media, dir, buses) in [
            (BUS_MEDIA_AUDIO, BUS_DIRECTION_INPUT, self.in_buses),
            (BUS_MEDIA_AUDIO, BUS_DIRECTION_OUTPUT, self.out_buses),
            (BUS_MEDIA_EVENT, BUS_DIRECTION_INPUT, self.event_in_buses),
        ] {
            for i in 0..buses {
                unsafe { activate(self.comp.0, media, dir, i, 1) };
            }
        }
        let on: unsafe extern "system" fn(*mut c_void, u8) -> i32 = unsafe { self.comp.slot(11) };
        if unsafe { on(self.comp.0, 1) } != K_RESULT_OK {
            return Err(format!("{} refused to wake up", self.info.name));
        }
        self.rate = rate;
        self.steady = 0;
        Ok(())
    }

    /// Buses count and main channels; it also resizes the sound scratch.
    fn read_ports(&mut self) {
        let count: unsafe extern "system" fn(*mut c_void, i32, i32) -> i32 = unsafe { self.comp.slot(7) };
        let info_f: unsafe extern "system" fn(*mut c_void, i32, i32, i32, *mut BusInfo) -> i32 = unsafe { self.comp.slot(8) };
        self.in_buses = unsafe { count(self.comp.0, BUS_MEDIA_AUDIO, BUS_DIRECTION_INPUT) };
        self.out_buses = unsafe { count(self.comp.0, BUS_MEDIA_AUDIO, BUS_DIRECTION_OUTPUT) };
        self.event_in_buses = unsafe { count(self.comp.0, BUS_MEDIA_EVENT, BUS_DIRECTION_INPUT) };
        self.in_channels.clear();
        self.out_channels.clear();
        for (media, dir, buses, out) in [
            (BUS_MEDIA_AUDIO, BUS_DIRECTION_INPUT, self.in_buses, &mut self.in_channels),
            (BUS_MEDIA_AUDIO, BUS_DIRECTION_OUTPUT, self.out_buses, &mut self.out_channels),
        ] {
            for i in 0..buses {
                let mut info: BusInfo = unsafe { std::mem::zeroed() };
                if unsafe { info_f(self.comp.0, media, dir, i, &mut info) } != K_RESULT_OK {
                    continue;
                }
                // Main buses only: aux outputs would be extra stems to mix.
                if info.bus_type != BUS_TYPE_MAIN {
                    continue;
                }
                out.push(info.channel_count.clamp(1, 8) as u32);
            }
        }
        let total = self.in_channels.iter().chain(&self.out_channels).sum::<u32>() as usize;
        self.scratch = vec![vec![0.0; MAX_FRAMES as usize]; total];
    }

    fn read_params(&mut self) {
        let Some(ctrl) = self.ctrl else { return };
        let count_f: unsafe extern "system" fn(*mut c_void) -> i32 = unsafe { ctrl.slot(8) };
        let info_f: unsafe extern "system" fn(*mut c_void, i32, *mut ParameterInfo) -> i32 = unsafe { ctrl.slot(9) };
        let count = unsafe { count_f(ctrl.0) }.clamp(0, 4096);
        let mut out = Vec::new();
        for i in 0..count {
            let mut info: ParameterInfo = unsafe { std::mem::zeroed() };
            if unsafe { info_f(ctrl.0, i, &mut info) } != K_RESULT_OK {
                continue;
            }
            if info.flags & (PARAM_IS_HIDDEN | PARAM_IS_READONLY) != 0 {
                continue;
            }
            let mut name = u16str(&info.title);
            if name.is_empty() {
                name = u16str(&info.short_title);
            }
            // The JUCE wrappers list 2048 emulated MIDI CC knobs ("MIDI CC
            // 15|123"): their mechanical way of accepting midi messages, not
            // knobs worth a channel strip, so hide them.
            if name.starts_with("MIDI CC ") {
                continue;
            }
            // Values here are normalized 0..1; the sliders show it that way.
            out.push(crate::plugin::Param {
                id: info.id,
                name,
                min: 0.0,
                max: 1.0,
                default: info.default_normalized_value,
                stepped: info.step_count > 0,
            });
        }
        self.params = out;
    }

    pub fn params(&self) -> &[crate::plugin::Param] {
        &self.params
    }

    pub fn param_value(&self, id: u32) -> Option<f64> {
        if let Some((_, v)) = self.pending.iter().rev().find(|(i, _)| *i == id) {
            return Some(*v);
        }
        let ctrl = self.ctrl?;
        let f: unsafe extern "system" fn(*mut c_void, u32) -> f64 = unsafe { ctrl.slot(14) };
        Some(unsafe { f(ctrl.0, id) })
    }

    /// The value as the plugin spells it ("3.2 kHz").
    pub fn param_text(&self, id: u32, value: f64) -> Option<String> {
        let ctrl = self.ctrl?;
        let f: unsafe extern "system" fn(*mut c_void, u32, f64, *mut u16) -> i32 = unsafe { ctrl.slot(10) };
        let mut buf = [0u16; 128];
        if unsafe { f(ctrl.0, id, value, buf.as_mut_ptr()) } == K_RESULT_OK {
            let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..end]))
        } else {
            None
        }
    }

    pub fn set_param(&mut self, id: u32, value: f64) {
        // The controller keeps its copy (it is what the plugin's own window
        // would do); the sound part gets it in the next block's queue.
        if let Some(ctrl) = self.ctrl {
            let set: unsafe extern "system" fn(*mut c_void, u32, f64) -> i32 = unsafe { ctrl.slot(15) };
            unsafe { set(ctrl.0, id, value) };
        }
        self.pending.retain(|(i, _)| *i != id);
        if self.pending.len() < 1024 {
            self.pending.push((id, value));
        }
        if !self.processing {
            self.flush();
        }
    }

    /// Applies queued parameters without sound (a block of zero samples),
    /// what the plugin would hear from us while it is quiet.
    fn flush(&mut self) {
        if self.rate == 0.0 || self.pending.is_empty() {
            return;
        }
        let was_processing = self.processing;
        if !was_processing {
            self.set_processing(true);
        }
        let tr = crate::plugin::Transport { tempo: 120.0, playing: false, beats: 0.0 };
        let mut context = self.context(&tr);
        let mut events: Vec<Event> = Vec::new();
        let mut queues = self.build_queues();
        let mut list = EventListObj { vt: event_list_vt(), events: &mut events };
        let mut changes = ParamChangesObj { vt: param_changes_vt(), queues: &mut queues };
        let mut data = ProcessData {
            process_mode: PROCESS_MODE_REALTIME,
            symbolic_sample_size: SYMBOLIC_SAMPLE_32,
            num_samples: 0,
            num_inputs: 0,
            num_outputs: 0,
            inputs: ptr::null_mut(),
            outputs: ptr::null_mut(),
            input_parameter_changes: &mut changes as *mut ParamChangesObj as *mut c_void,
            output_parameter_changes: ptr::null_mut(),
            input_events: &mut list as *mut EventListObj as *mut c_void,
            output_events: ptr::null_mut(),
            process_context: &mut context,
        };
        let p = self.processor_obj();
        let f: unsafe extern "system" fn(*mut c_void, *mut ProcessData) -> i32 = unsafe { p.slot(9) };
        let _ = unsafe { f(p.0, &mut data) };
        self.pending.clear();
        free_queues(&mut queues);
        if !was_processing {
            self.set_processing(false);
        }
    }

    /// One queue per changed parameter, with the value at offset 0.
    fn build_queues(&self) -> Vec<QueueObj> {
        self.pending
            .iter()
            .map(|&(id, v)| QueueObj {
                vt: queue_vt(),
                id,
                points: Box::into_raw(Box::new(vec![(0, v)])),
            })
            .collect()
    }

    fn context(&self, tr: &crate::plugin::Transport) -> ProcessContext {
        ProcessContext {
            state: CONTEXT_SYSTEM_TIME_VALID
                | CONTEXT_CONTINUOUS_TIME_VALID
                | CONTEXT_PROJECT_TIME_MUSIC_VALID
                | CONTEXT_BAR_POSITION_VALID
                | CONTEXT_TEMPO_VALID
                | CONTEXT_TIME_SIG_VALID
                | if tr.playing { CONTEXT_PLAYING } else { 0 },
            sample_rate: self.rate,
            project_time_samples: self.steady,
            system_time: self.started.elapsed().as_nanos() as i64,
            continuous_time_samples: self.steady,
            project_time_music: tr.beats,
            bar_position_music: (tr.beats / 4.0).floor() * 4.0,
            cycle_start_music: 0.0,
            cycle_end_music: 0.0,
            tempo: tr.tempo,
            time_sig_numerator: 4,
            time_sig_denominator: 4,
            chord_key: 0,
            chord_root: 0,
            chord_mask: 0,
            smpte_offset_subframes: 0,
            frame_rate_fps: 0,
            frame_rate_flags: 0,
            samples_to_next_clock: 0,
        }
    }

    pub fn note(&mut self, on: bool, key: u8, vel: f32, time: u32) {
        if self.events.len() > 1024 {
            return;
        }
        self.events
            .push(Event::note(if on { EVENT_NOTE_ON } else { EVENT_NOTE_OFF }, time, key as i16, vel));
    }

    /// The plugin's own window (IPlugView) is not opened yet: sliders instead.
    pub fn has_gui(&self) -> bool {
        false
    }

    pub fn gui_open(&self) -> bool {
        false
    }

    pub fn open_gui(&mut self, _title: &str) -> Result<(), String> {
        Err("the own window of a VST3 comes later; sliders for now".into())
    }

    pub fn close_gui(&mut self) {}

    pub fn save_state(&self) -> Option<Vec<u8>> {
        let comp = get_state_of(self.comp, 13)?;
        let ctrl = self.ctrl.and_then(|c| get_state_of(c, 7));
        let mut out = STATE_MAGIC.to_vec();
        out.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        out.extend_from_slice(&(ctrl.as_ref().map_or(0, |v| v.len()) as u32).to_le_bytes());
        out.extend_from_slice(&comp);
        out.extend(&ctrl.unwrap_or_default());
        Some(out)
    }

    fn load_state(&mut self, data: &[u8]) -> bool {
        let (comp_part, ctrl_part) = split_state(data);
        let mut ok = false;
        if let Some(part) = comp_part {
            ok |= put_state_into(self.comp, 12, part);
        }
        if let (Some(c), Some(part)) = (self.ctrl, ctrl_part) {
            ok |= put_state_into(c, 5, part);
        }
        ok
    }

    /// Runs one block. `input` (stereo) feeds the first input bus; the first
    /// output bus is mixed back into `output`.
    pub fn run(&mut self, input: Option<&[[f32; 2]]>, output: &mut [[f32; 2]], tr: &crate::plugin::Transport) {
        if self.rate == 0.0 {
            return;
        }
        if !self.processing {
            self.set_processing(true);
        }
        let total_in = self.in_channels.iter().sum::<u32>() as usize;
        let main_in = self.in_channels.first().copied().unwrap_or(0) as usize;
        let first_out = self.out_channels.first().copied().unwrap_or(0) as usize;
        for chunk_start in (0..output.len()).step_by(MAX_FRAMES as usize) {
            let n = (output.len() - chunk_start).min(MAX_FRAMES as usize);
            for (i, ch) in self.scratch.iter_mut().enumerate() {
                ch[..n].fill(0.0);
                if let (Some(src), true) = (input, i < main_in.min(2)) {
                    for (k, f) in src[chunk_start..chunk_start + n].iter().enumerate() {
                        ch[k] = if main_in == 1 { (f[0] + f[1]) * 0.5 } else { f[i] };
                    }
                }
            }
            let mut ptrs: Vec<*mut f32> = self.scratch.iter_mut().map(|c| c.as_mut_ptr()).collect();
            let mut bufs: Vec<AudioBusBuffers> = Vec::new();
            let mut at = 0usize;
            for &c in self.in_channels.iter().chain(&self.out_channels) {
                bufs.push(AudioBusBuffers {
                    num_channels: c as i32,
                    silence_flags: 0,
                    channel_buffers32: unsafe { ptrs.as_mut_ptr().add(at) },
                });
                at += c as usize;
            }
            let (ins, outs) = bufs.split_at_mut(self.in_channels.len());
            let mut context = self.context(tr);
            // Notes and parameters go with the first block of each run.
            let mut events = if chunk_start == 0 { std::mem::take(&mut self.events) } else { Vec::new() };
            events.sort_by_key(|e| e.sample_offset);
            let mut list = EventListObj { vt: event_list_vt(), events: &mut events };
            let mut queues = if chunk_start == 0 { self.build_queues() } else { Vec::new() };
            let mut changes = ParamChangesObj { vt: param_changes_vt(), queues: &mut queues };
            let mut data = ProcessData {
                process_mode: PROCESS_MODE_REALTIME,
                symbolic_sample_size: SYMBOLIC_SAMPLE_32,
                num_samples: n as i32,
                num_inputs: ins.len() as i32,
                num_outputs: outs.len() as i32,
                inputs: if ins.is_empty() { ptr::null_mut() } else { ins.as_mut_ptr() },
                outputs: outs.as_mut_ptr(),
                input_parameter_changes: &mut changes as *mut ParamChangesObj as *mut c_void,
                output_parameter_changes: ptr::null_mut(),
                input_events: &mut list as *mut EventListObj as *mut c_void,
                output_events: ptr::null_mut(),
                process_context: &mut context,
            };
            let p = self.processor_obj();
            let f: unsafe extern "system" fn(*mut c_void, *mut ProcessData) -> i32 = unsafe { p.slot(9) };
            let _ = unsafe { f(p.0, &mut data) };
            self.steady += n as i64;
            if chunk_start == 0 {
                self.pending.clear();
            }
            free_queues(&mut queues);
            if first_out > 0 && total_in < self.scratch.len() {
                for (k, f) in output[chunk_start..chunk_start + n].iter_mut().enumerate() {
                    let l = self.scratch[total_in][k];
                    let r = if first_out > 1 { self.scratch[total_in + 1][k] } else { l };
                    *f = [if l.is_finite() { l } else { 0.0 }, if r.is_finite() { r } else { 0.0 }];
                }
            }
        }
    }

    /// Main-thread housekeeping: edits coming back and restart requests; no
    /// windows or timers in a VST3 yet (see plugin.rs for the CLAP ones).
    pub fn idle(&mut self, rate: f64) {
        let edits = self.edited.lock().ok().map(|mut m| std::mem::take(&mut *m)).unwrap_or_default();
        for (id, v) in edits {
            self.set_param(id, v);
        }
        let flags = self.restart.swap(0, Ordering::Relaxed);
        if flags & (RESTART_RELOAD as u32 | RESTART_IO_CHANGED as u32) != 0 {
            self.read_ports();
            self.read_params();
            if self.rate > 0.0 {
                let _ = self.set_rate(self.rate);
            }
        } else if flags & RESTART_PARAM_TITLES as u32 != 0 {
            self.read_params();
        }
        if rate > 0.0 && rate != self.rate {
            let _ = self.set_rate(rate);
        }
    }
}

/// The extra reference of a queryInterface result has to go back.
fn free_queues(queues: &mut Vec<QueueObj>) {
    for q in std::mem::take(queues) {
        if !q.points.is_null() {
            drop(unsafe { Box::from_raw(q.points) });
        }
    }
}

fn get_state_of(obj: Obj, slot: usize) -> Option<Vec<u8>> {
    let mut data: Vec<u8> = Vec::new();
    let stream = outstream(&mut data);
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32 = unsafe { obj.slot(slot) };
    (unsafe { f(obj.0, &*stream as *const OutStreamObj as *mut c_void) } == K_RESULT_OK).then_some(data)
}

fn put_state_into(obj: Obj, slot: usize, part: &[u8]) -> bool {
    if part.is_empty() {
        return false;
    }
    let stream = instream(part);
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32 = unsafe { obj.slot(slot) };
    (unsafe { f(obj.0, &*stream as *const InStreamObj as *mut c_void) } == K_RESULT_OK)
}

/// Our envelope: magic "V3S1", then the component bytes and the controller
/// bytes with their lengths, so both parts can be given back to the right
/// object (when the plugin has one object for both, they are the same bytes).
const STATE_MAGIC: &[u8; 4] = b"V3S1";

fn split_state(data: &[u8]) -> (Option<&[u8]>, Option<&[u8]>) {
    if !data.starts_with(STATE_MAGIC) {
        return (Some(data), None);
    }
    let comp_len = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    let ctrl_len = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
    if data.len() < 12 + comp_len + ctrl_len {
        return (Some(data), None);
    }
    (Some(&data[12..12 + comp_len]), (ctrl_len > 0).then_some(&data[12 + comp_len..12 + comp_len + ctrl_len]))
}

impl Drop for Instance {
    fn drop(&mut self) {
        if self.rate != 0.0 {
            if self.processing {
                self.set_processing(false);
                self.processing = false;
            }
            let off: unsafe extern "system" fn(*mut c_void, u8) -> i32 = unsafe { self.comp.slot(11) };
            unsafe { off(self.comp.0, 0) };
        }
        // The plugin object frees what it created (terminate), then the
        // references go back. Same object never terminated twice.
        let term: unsafe extern "system" fn(*mut c_void) -> i32 = unsafe { self.comp.slot(4) };
        unsafe { term(self.comp.0) };
        if let Some(c) = self.ctrl.filter(|c| c.0 != self.comp.0) {
            let term: unsafe extern "system" fn(*mut c_void) -> i32 = unsafe { c.slot(4) };
            unsafe { term(c.0) };
        }
        for o in self.to_release.drain(..) {
            o.release();
        }
        if let Some(c) = self.ctrl {
            c.release();
        }
        self.comp.release();
    }
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn abi_sizes_match_vst3() {
        // Measured with g++ against the official headers (probe program).
        assert_eq!(size_of::<ProcessSetup>(), 24);
        assert_eq!(size_of::<AudioBusBuffers>(), 24);
        assert_eq!(size_of::<ProcessData>(), 80);
        assert_eq!(size_of::<ProcessContext>(), 112);
        assert_eq!(size_of::<Event>(), 48);
        assert_eq!(size_of::<NoteOnEvent>(), 20);
        assert_eq!(size_of::<NoteOffEvent>(), 16);
        assert_eq!(size_of::<ParameterInfo>(), 792);
        assert_eq!(size_of::<BusInfo>(), 276);
        assert_eq!(size_of::<PClassInfo>(), 116);
        assert_eq!(size_of::<PClassInfo2>(), 440);
        assert_eq!(size_of::<PFactoryInfo>(), 452);
        // The event union must sit where the plugin reads it (its DataEvent
        // member makes it 8-byte aligned, so after 6 bytes of padding).
        assert_eq!(offset_of!(Event, payload), 24);
        assert_eq!(offset_of!(Event, ppq_position), 8);
        assert_eq!(offset_of!(Event, kind), 18);
    }

    #[test]
    fn guids_are_header_word_order() {
        // IComponent's INLINE_UID (0xE831FF31, 0xF2D54301, 0x928EBBEE, 0x25697802):
        // every 32-bit word is written big-endian, one byte at a time.
        let c = Tuid::hex(U_ICOMPONENT);
        assert_eq!(c.0, [0xE8, 0x31, 0xFF, 0x31, 0xF2, 0xD5, 0x43, 0x01, 0x92, 0x8E, 0xBB, 0xEE, 0x25, 0x69, 0x78, 0x02]);
        let from_text = uid_from_hex("E831FF31F2D54301928EBBEE25697802").unwrap();
        assert_eq!(from_text, c);
        assert_eq!(uid_from_hex("E831FF31"), None);
        assert_eq!(uid_from_hex("Z831FF31F2D54301928EBBEE25697802"), None);
    }

    #[test]
    fn host_vtables_answer_queries() {
        // A host object used the plugin way: slots through the vtable.
        let host = new_host();
        let raw = host.ptr();
        let as_plugin = Obj(raw);
        let got = as_plugin.query(U_UNKNOWN).expect("FUnknown");
        assert_eq!(got.0, raw);
        assert!(as_plugin.query(U_IHOST_APPLICATION).is_some());
        assert!(as_plugin.query(U_ICOMPONENT).is_none(), "unknown interface refused");
        as_plugin.release(); // our host_ref, a no-op
        // get_name through slot 3 answers "Beat Studio".
        let f: unsafe extern "system" fn(*mut c_void, *mut u16) -> i32 = unsafe { as_plugin.slot(3) };
        let mut buf = [0u16; 128];
        assert_eq!(unsafe { f(raw, buf.as_mut_ptr()) }, K_RESULT_OK);
        assert_eq!(String::from_utf16_lossy(&buf[..11]), "Beat Studio");
        // The BStream: writing lands in the Vec, tell follows, reading refuses.
        let mut sink = Vec::new();
        let box_out = outstream(&mut sink);
        let so = Obj(&*box_out as *const OutStreamObj as *mut c_void);
        let write: unsafe extern "system" fn(*mut c_void, *mut c_void, i32, *mut i32) -> i32 = unsafe { so.slot(4) };
        let mut wrote = 0;
        assert_eq!(unsafe { write(so.0, [9u8; 5].as_mut_ptr().cast(), 5, &mut wrote) }, K_RESULT_OK);
        assert_eq!(wrote, 5);
        assert_eq!(sink, vec![9; 5]);
        let tell: unsafe extern "system" fn(*mut c_void, *mut i64) -> i32 = unsafe { so.slot(6) };
        let mut pos = 0i64;
        unsafe { tell(so.0, &mut pos) };
        assert_eq!(pos, 5);
        let read: unsafe extern "system" fn(*mut c_void, *mut c_void, i32, *mut i32) -> i32 = unsafe { so.slot(3) };
        assert_ne!(unsafe { read(so.0, buf.as_mut_ptr().cast(), 1, &mut wrote) }, K_RESULT_OK);
        // The reading stream answers read and seek.
        let src = [1u8, 2, 3, 4];
        let box_in = instream(&src);
        let si = Obj(&*box_in as *const InStreamObj as *mut c_void);
        let read2: unsafe extern "system" fn(*mut c_void, *mut c_void, i32, *mut i32) -> i32 = unsafe { si.slot(3) };
        let mut got = 0;
        let mut back = [0u8; 4];
        assert_eq!(unsafe { read2(si.0, back.as_mut_ptr().cast(), 3, &mut got) }, K_RESULT_OK);
        assert_eq!((got, back[..3].to_vec()), (3, vec![1, 2, 3]));
        assert_eq!(unsafe { read2(si.0, back.as_mut_ptr().add(3).cast(), 5, &mut got) }, K_RESULT_FALSE);
        assert_eq!(got, 1);
        // The handler: performEdit lands in the shared list, restart in flags.
        let mut restart = Box::new(AtomicU32::new(0));
        let mut edited: Box<Mutex<Vec<(u32, f64)>>> = Box::default();
        let handler = new_handler(&mut *restart, &mut *edited);
        let ho = Obj(&*handler as *const HandlerObj as *mut c_void);
        let perform: unsafe extern "system" fn(*mut c_void, u32, f64) -> i32 = unsafe { ho.slot(4) };
        assert_eq!(unsafe { perform(ho.0, 42, 0.5) }, K_RESULT_OK);
        assert_eq!(&*edited.lock().unwrap(), &vec![(42, 0.5)]);
        let restarter: unsafe extern "system" fn(*mut c_void, i32) -> i32 = unsafe { ho.slot(6) };
        assert_eq!(unsafe { restarter(ho.0, RESTART_IO_CHANGED) }, K_RESULT_OK);
        assert_eq!(restart.load(Ordering::Relaxed), 2);
    }
}
