// CLAP plugin hosting: scanning, loading, sound processing, parameters and
// saved state. The plugin's own window is not shown; its parameters get
// generic sliders instead. Scanning runs each file in a child process
// (`beatstudio --clap-scan <file>`) so a plugin that crashes cannot take the
// studio down with it.

use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------- CLAP ABI

#[repr(C)]
#[derive(Clone, Copy)]
struct ClapVersion {
    major: u32,
    minor: u32,
    revision: u32,
}

const CLAP_VERSION: ClapVersion = ClapVersion { major: 1, minor: 2, revision: 0 };

#[repr(C)]
struct ClapEntry {
    clap_version: ClapVersion,
    init: unsafe extern "C" fn(*const c_char) -> bool,
    deinit: unsafe extern "C" fn(),
    get_factory: unsafe extern "C" fn(*const c_char) -> *const c_void,
}

#[repr(C)]
struct ClapPluginFactory {
    get_plugin_count: unsafe extern "C" fn(*const ClapPluginFactory) -> u32,
    get_plugin_descriptor: unsafe extern "C" fn(*const ClapPluginFactory, u32) -> *const ClapDescriptor,
    create_plugin: unsafe extern "C" fn(*const ClapPluginFactory, *const ClapHost, *const c_char) -> *const ClapPlugin,
}

#[repr(C)]
struct ClapDescriptor {
    clap_version: ClapVersion,
    id: *const c_char,
    name: *const c_char,
    vendor: *const c_char,
    url: *const c_char,
    manual_url: *const c_char,
    support_url: *const c_char,
    version: *const c_char,
    description: *const c_char,
    features: *const *const c_char,
}

#[repr(C)]
struct ClapHost {
    clap_version: ClapVersion,
    host_data: *mut c_void,
    name: *const c_char,
    vendor: *const c_char,
    url: *const c_char,
    version: *const c_char,
    get_extension: unsafe extern "C" fn(*const ClapHost, *const c_char) -> *const c_void,
    request_restart: unsafe extern "C" fn(*const ClapHost),
    request_process: unsafe extern "C" fn(*const ClapHost),
    request_callback: unsafe extern "C" fn(*const ClapHost),
}

#[repr(C)]
struct ClapPlugin {
    desc: *const ClapDescriptor,
    plugin_data: *mut c_void,
    init: unsafe extern "C" fn(*const ClapPlugin) -> bool,
    destroy: unsafe extern "C" fn(*const ClapPlugin),
    activate: unsafe extern "C" fn(*const ClapPlugin, f64, u32, u32) -> bool,
    deactivate: unsafe extern "C" fn(*const ClapPlugin),
    start_processing: unsafe extern "C" fn(*const ClapPlugin) -> bool,
    stop_processing: unsafe extern "C" fn(*const ClapPlugin),
    reset: unsafe extern "C" fn(*const ClapPlugin),
    process: unsafe extern "C" fn(*const ClapPlugin, *const ClapProcess) -> i32,
    get_extension: unsafe extern "C" fn(*const ClapPlugin, *const c_char) -> *const c_void,
    on_main_thread: unsafe extern "C" fn(*const ClapPlugin),
}

#[repr(C)]
struct ClapAudioBuffer {
    data32: *mut *mut f32,
    data64: *mut *mut f64,
    channel_count: u32,
    latency: u32,
    constant_mask: u64,
}

#[repr(C)]
struct ClapInputEvents {
    ctx: *mut c_void,
    size: unsafe extern "C" fn(*const ClapInputEvents) -> u32,
    get: unsafe extern "C" fn(*const ClapInputEvents, u32) -> *const EventHeader,
}

#[repr(C)]
struct ClapOutputEvents {
    ctx: *mut c_void,
    try_push: unsafe extern "C" fn(*const ClapOutputEvents, *const EventHeader) -> bool,
}

#[repr(C)]
struct ClapProcess {
    steady_time: i64,
    frames_count: u32,
    transport: *const EventTransport,
    audio_inputs: *const ClapAudioBuffer,
    audio_outputs: *mut ClapAudioBuffer,
    audio_inputs_count: u32,
    audio_outputs_count: u32,
    in_events: *const ClapInputEvents,
    out_events: *const ClapOutputEvents,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EventHeader {
    size: u32,
    time: u32,
    space_id: u16,
    kind: u16,
    flags: u32,
}

const EVENT_NOTE_ON: u16 = 0;
const EVENT_NOTE_OFF: u16 = 1;
const EVENT_PARAM_VALUE: u16 = 5;
const EVENT_TRANSPORT: u16 = 9;

#[repr(C)]
#[derive(Clone, Copy)]
struct EventNote {
    header: EventHeader,
    note_id: i32,
    port_index: i16,
    channel: i16,
    key: i16,
    velocity: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EventParamValue {
    header: EventHeader,
    param_id: u32,
    cookie: *mut c_void,
    note_id: i32,
    port_index: i16,
    channel: i16,
    key: i16,
    value: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
union RawEvent {
    header: EventHeader,
    note: EventNote,
    param: EventParamValue,
}

#[repr(C)]
struct EventTransport {
    header: EventHeader,
    flags: u32,
    song_pos_beats: i64,
    song_pos_seconds: i64,
    tempo: f64,
    tempo_inc: f64,
    loop_start_beats: i64,
    loop_end_beats: i64,
    loop_start_seconds: i64,
    loop_end_seconds: i64,
    bar_start: i64,
    bar_number: i32,
    tsig_num: u16,
    tsig_denom: u16,
}

const TRANSPORT_HAS_TEMPO: u32 = 1 << 0;
const TRANSPORT_HAS_BEATS_TIMELINE: u32 = 1 << 1;
const TRANSPORT_HAS_TIME_SIGNATURE: u32 = 1 << 3;
const TRANSPORT_IS_PLAYING: u32 = 1 << 4;
const BEATTIME_FACTOR: f64 = (1u64 << 31) as f64;

#[repr(C)]
struct ClapParamInfo {
    id: u32,
    flags: u32,
    cookie: *mut c_void,
    name: [c_char; 256],
    module: [c_char; 1024],
    min_value: f64,
    max_value: f64,
    default_value: f64,
}

const PARAM_IS_STEPPED: u32 = 1 << 0;
const PARAM_IS_HIDDEN: u32 = 1 << 2;
const PARAM_IS_READONLY: u32 = 1 << 3;

#[repr(C)]
struct ClapPluginParams {
    count: unsafe extern "C" fn(*const ClapPlugin) -> u32,
    get_info: unsafe extern "C" fn(*const ClapPlugin, u32, *mut ClapParamInfo) -> bool,
    get_value: unsafe extern "C" fn(*const ClapPlugin, u32, *mut f64) -> bool,
    value_to_text: unsafe extern "C" fn(*const ClapPlugin, u32, f64, *mut c_char, u32) -> bool,
    text_to_value: unsafe extern "C" fn(*const ClapPlugin, u32, *const c_char, *mut f64) -> bool,
    flush: unsafe extern "C" fn(*const ClapPlugin, *const ClapInputEvents, *const ClapOutputEvents),
}

#[repr(C)]
struct ClapAudioPortInfo {
    id: u32,
    name: [c_char; 256],
    flags: u32,
    channel_count: u32,
    port_type: *const c_char,
    in_place_pair: u32,
}

#[repr(C)]
struct ClapPluginAudioPorts {
    count: unsafe extern "C" fn(*const ClapPlugin, bool) -> u32,
    get: unsafe extern "C" fn(*const ClapPlugin, u32, bool, *mut ClapAudioPortInfo) -> bool,
}

#[repr(C)]
struct ClapOStream {
    ctx: *mut c_void,
    write: unsafe extern "C" fn(*const ClapOStream, *const c_void, u64) -> i64,
}

#[repr(C)]
struct ClapIStream {
    ctx: *mut c_void,
    read: unsafe extern "C" fn(*const ClapIStream, *mut c_void, u64) -> i64,
}

#[repr(C)]
struct ClapPluginState {
    save: unsafe extern "C" fn(*const ClapPlugin, *const ClapOStream) -> bool,
    load: unsafe extern "C" fn(*const ClapPlugin, *const ClapIStream) -> bool,
}

extern "C" {
    fn dlopen(filename: *const c_char, flag: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}
const RTLD_NOW: i32 = 2;

fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

// ---------------------------------------------------------------- libraries

/// A loaded plugin file. Libraries are never unloaded: some plugins keep
/// threads running after their last instance is gone.
struct Library {
    entry: *const ClapEntry,
    factory: *const ClapPluginFactory,
}

unsafe impl Send for Library {}

fn libraries() -> &'static Mutex<HashMap<PathBuf, Library>> {
    static LIBS: OnceLock<Mutex<HashMap<PathBuf, Library>>> = OnceLock::new();
    LIBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn with_factory<T>(path: &Path, f: impl FnOnce(*const ClapPluginFactory) -> T) -> Result<T, String> {
    let mut libs = libraries().lock().map_err(|_| "plugin list is broken")?;
    if !libs.contains_key(path) {
        let c_path = CString::new(path.as_os_str().as_encoded_bytes()).map_err(|_| "bad file name")?;
        let handle = unsafe { dlopen(c_path.as_ptr(), RTLD_NOW) };
        if handle.is_null() {
            return Err(format!("cannot load: {}", cstr(unsafe { dlerror() })));
        }
        let entry = unsafe { dlsym(handle, c"clap_entry".as_ptr()) } as *const ClapEntry;
        if entry.is_null() {
            return Err("not a CLAP plugin".into());
        }
        let e = unsafe { &*entry };
        if e.clap_version.major < 1 || !unsafe { (e.init)(c_path.as_ptr()) } {
            return Err("the plugin refused to start".into());
        }
        let factory = unsafe { (e.get_factory)(c"clap.plugin-factory".as_ptr()) } as *const ClapPluginFactory;
        if factory.is_null() {
            return Err("the plugin has no factory".into());
        }
        libs.insert(path.to_path_buf(), Library { entry, factory });
    }
    let lib = &libs[path];
    let _ = lib.entry;
    Ok(f(lib.factory))
}

// ---------------------------------------------------------------- scanning

#[derive(Clone, Debug, PartialEq)]
pub struct PluginInfo {
    pub path: PathBuf,
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub instrument: bool,
}

/// Folders CLAP plugins are installed in on Linux, plus $CLAP_PATH.
pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(extra) = std::env::var_os("CLAP_PATH") {
        dirs.extend(std::env::split_paths(&extra));
    }
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".clap"));
    }
    dirs.push(PathBuf::from("/usr/lib/clap"));
    dirs.push(PathBuf::from("/usr/local/lib/clap"));
    dirs
}

pub fn plugin_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "clap") && p.is_file() {
                out.push(p);
            } else if p.is_dir() && depth < 3 {
                walk(&p, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    for d in search_dirs() {
        walk(&d, 0, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

/// Runs inside the child process: prints one line per plugin in the file.
pub fn scan_file_here(path: &Path) -> Result<(), String> {
    let infos = with_factory(path, |factory| {
        let f = unsafe { &*factory };
        let mut out = Vec::new();
        for i in 0..unsafe { (f.get_plugin_count)(factory) } {
            let d = unsafe { (f.get_plugin_descriptor)(factory, i) };
            if d.is_null() {
                continue;
            }
            let d = unsafe { &*d };
            let mut instrument = false;
            let mut feat = d.features;
            while !feat.is_null() && !unsafe { *feat }.is_null() {
                if cstr(unsafe { *feat }) == "instrument" {
                    instrument = true;
                }
                feat = unsafe { feat.add(1) };
            }
            out.push(format!("{}\t{}\t{}\t{}", cstr(d.id), cstr(d.name), cstr(d.vendor), if instrument { "instrument" } else { "effect" }));
        }
        out
    })?;
    for line in infos {
        println!("CLAP\t{line}");
    }
    Ok(())
}

fn parse_scan(path: &Path, text: &str) -> Vec<PluginInfo> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.strip_prefix("CLAP\t")?.split('\t');
            let (id, name, vendor, kind) = (f.next()?, f.next()?, f.next()?, f.next()?);
            Some(PluginInfo { path: path.to_path_buf(), id: id.into(), name: name.into(), vendor: vendor.into(), instrument: kind == "instrument" })
        })
        .collect()
}

/// Scans every plugin file, each in its own child process with a time limit.
pub fn scan_all() -> Vec<PluginInfo> {
    match std::env::current_exe() {
        Ok(me) => scan_with(&me),
        Err(_) => Vec::new(),
    }
}

/// `me` is the Beat Studio program, which answers `--clap-scan`.
fn scan_with(me: &Path) -> Vec<PluginInfo> {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let mut found = Vec::new();
    for path in plugin_files() {
        let Ok(mut child) = Command::new(me).arg("--clap-scan").arg(&path).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else { continue };
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) if start.elapsed() < Duration::from_secs(15) => std::thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        if status.is_some() {
            let mut text = String::new();
            if let Some(mut out) = child.stdout.take() {
                use std::io::Read;
                let _ = out.read_to_string(&mut text);
            }
            found.extend(parse_scan(&path, &text));
        }
    }
    found.sort_by_key(|p| p.name.to_lowercase());
    found
}

pub fn cache_to_text(list: &[PluginInfo]) -> String {
    list.iter()
        .map(|p| format!("{}\t{}\t{}\t{}\t{}\n", p.path.display(), p.id, p.name, p.vendor, if p.instrument { "instrument" } else { "effect" }))
        .collect()
}

pub fn cache_from_text(text: &str) -> Vec<PluginInfo> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let path = PathBuf::from(f.next()?);
            let (id, name, vendor, kind) = (f.next()?, f.next()?, f.next()?, f.next()?);
            Some(PluginInfo { path, id: id.into(), name: name.into(), vendor: vendor.into(), instrument: kind == "instrument" })
        })
        .collect()
}

// ---------------------------------------------------------------- host side

/// What the plugin asked of the host from any thread.
struct HostFlags {
    callback: AtomicBool,
    restart: AtomicBool,
}

unsafe extern "C" fn host_get_extension(_: *const ClapHost, _: *const c_char) -> *const c_void {
    ptr::null()
}
unsafe extern "C" fn host_request_restart(h: *const ClapHost) {
    let flags = &*((*h).host_data as *const HostFlags);
    flags.restart.store(true, Ordering::Relaxed);
}
unsafe extern "C" fn host_request_process(_: *const ClapHost) {}
unsafe extern "C" fn host_request_callback(h: *const ClapHost) {
    let flags = &*((*h).host_data as *const HostFlags);
    flags.callback.store(true, Ordering::Relaxed);
}

unsafe extern "C" fn events_size(list: *const ClapInputEvents) -> u32 {
    (*((*list).ctx as *const Vec<RawEvent>)).len() as u32
}
unsafe extern "C" fn events_get(list: *const ClapInputEvents, i: u32) -> *const EventHeader {
    let v = &*((*list).ctx as *const Vec<RawEvent>);
    v.get(i as usize).map_or(ptr::null(), |e| &e.header as *const EventHeader)
}
unsafe extern "C" fn events_push(_: *const ClapOutputEvents, _: *const EventHeader) -> bool {
    true
}

unsafe extern "C" fn ostream_write(s: *const ClapOStream, buf: *const c_void, size: u64) -> i64 {
    let out = &mut *((*s).ctx as *mut Vec<u8>);
    out.extend_from_slice(std::slice::from_raw_parts(buf as *const u8, size as usize));
    size as i64
}

struct ReadCursor<'a> {
    data: &'a [u8],
    at: usize,
}

unsafe extern "C" fn istream_read(s: *const ClapIStream, buf: *mut c_void, size: u64) -> i64 {
    let c = &mut *((*s).ctx as *mut ReadCursor);
    let n = (size as usize).min(c.data.len() - c.at);
    ptr::copy_nonoverlapping(c.data[c.at..].as_ptr(), buf as *mut u8, n);
    c.at += n;
    n as i64
}

#[derive(Clone, Debug)]
pub struct Param {
    pub id: u32,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub stepped: bool,
}

/// The tempo and position given to plugins with each block.
#[derive(Clone, Copy)]
pub struct Transport {
    pub tempo: f64,
    pub playing: bool,
    pub beats: f64,
}

const MAX_FRAMES: u32 = 4096;

/// One running plugin. Created, activated and destroyed on the interface
/// thread; `process` runs on the sound thread (never both at once: the
/// project lock is held for both).
struct Instance {
    plugin: *const ClapPlugin,
    _host: Box<ClapHost>,
    flags: Box<HostFlags>,
    _strings: [CString; 4],
    info: PluginInfo,
    params: Vec<Param>,
    in_channels: Vec<u32>,
    out_channels: Vec<u32>,
    rate: f64,
    processing: bool,
    steady: i64,
    events: Vec<RawEvent>,
    /// Inputs then outputs, one Vec per channel, MAX_FRAMES long.
    scratch: Vec<Vec<f32>>,
}

unsafe impl Send for Instance {}

impl Instance {
    fn new(info: &PluginInfo, rate: f64) -> Result<Instance, String> {
        let strings = [c"Beat Studio".to_owned(), c"Beat Studio".to_owned(), c"".to_owned(), c"0.1".to_owned()];
        let mut flags = Box::new(HostFlags { callback: AtomicBool::new(false), restart: AtomicBool::new(false) });
        let host = Box::new(ClapHost {
            clap_version: CLAP_VERSION,
            host_data: &mut *flags as *mut HostFlags as *mut c_void,
            name: strings[0].as_ptr(),
            vendor: strings[1].as_ptr(),
            url: strings[2].as_ptr(),
            version: strings[3].as_ptr(),
            get_extension: host_get_extension,
            request_restart: host_request_restart,
            request_process: host_request_process,
            request_callback: host_request_callback,
        });
        let id = CString::new(info.id.clone()).map_err(|_| "bad plugin id")?;
        let plugin = with_factory(&info.path, |factory| unsafe { ((*factory).create_plugin)(factory, &*host, id.as_ptr()) })?;
        if plugin.is_null() {
            return Err(format!("{} could not be created", info.name));
        }
        let mut inst = Instance {
            plugin,
            _host: host,
            flags,
            _strings: strings,
            info: info.clone(),
            params: Vec::new(),
            in_channels: Vec::new(),
            out_channels: Vec::new(),
            rate: 0.0,
            processing: false,
            steady: 0,
            events: Vec::new(),
            scratch: Vec::new(),
        };
        if !unsafe { ((*plugin).init)(plugin) } {
            unsafe { ((*plugin).destroy)(plugin) };
            inst.plugin = ptr::null();
            return Err(format!("{} failed to start", info.name));
        }
        inst.read_ports();
        inst.read_params();
        inst.activate(rate)?;
        Ok(inst)
    }

    fn ext<T>(&self, id: &CStr) -> Option<&T> {
        let p = unsafe { ((*self.plugin).get_extension)(self.plugin, id.as_ptr()) } as *const T;
        unsafe { p.as_ref() }
    }

    fn read_ports(&mut self) {
        let mut ins = Vec::new();
        let mut outs = Vec::new();
        if let Some(ap) = self.ext::<ClapPluginAudioPorts>(c"clap.audio-ports") {
            for (is_input, list) in [(true, &mut ins), (false, &mut outs)] {
                for i in 0..unsafe { (ap.count)(self.plugin, is_input) } {
                    let mut info: ClapAudioPortInfo = unsafe { std::mem::zeroed() };
                    if unsafe { (ap.get)(self.plugin, i, is_input, &mut info) } {
                        list.push(info.channel_count.clamp(1, 8));
                    }
                }
            }
        } else {
            ins.push(2);
            outs.push(2);
        }
        self.in_channels = ins;
        self.out_channels = outs;
        let total = (self.in_channels.iter().sum::<u32>() + self.out_channels.iter().sum::<u32>()) as usize;
        self.scratch = vec![vec![0.0; MAX_FRAMES as usize]; total];
    }

    fn read_params(&mut self) {
        let mut out = Vec::new();
        if let Some(pp) = self.ext::<ClapPluginParams>(c"clap.params") {
            for i in 0..unsafe { (pp.count)(self.plugin) } {
                let mut info: ClapParamInfo = unsafe { std::mem::zeroed() };
                if !unsafe { (pp.get_info)(self.plugin, i, &mut info) } || info.flags & (PARAM_IS_HIDDEN | PARAM_IS_READONLY) != 0 {
                    continue;
                }
                out.push(Param {
                    id: info.id,
                    name: cstr(info.name.as_ptr()),
                    min: info.min_value,
                    max: info.max_value,
                    default: info.default_value,
                    stepped: info.flags & PARAM_IS_STEPPED != 0,
                });
            }
        }
        self.params = out;
    }

    fn activate(&mut self, rate: f64) -> Result<(), String> {
        if !unsafe { ((*self.plugin).activate)(self.plugin, rate, 1, MAX_FRAMES) } {
            return Err(format!("{} refused the sound settings", self.info.name));
        }
        self.rate = rate;
        Ok(())
    }

    fn deactivate(&mut self) {
        if self.rate == 0.0 {
            return;
        }
        if self.processing {
            unsafe { ((*self.plugin).stop_processing)(self.plugin) };
            self.processing = false;
        }
        unsafe { ((*self.plugin).deactivate)(self.plugin) };
        self.rate = 0.0;
    }

    /// Main-thread housekeeping: callbacks the plugin asked for, a restart, or
    /// a new sample rate.
    fn idle(&mut self, rate: f64) {
        if self.flags.callback.swap(false, Ordering::Relaxed) {
            unsafe { ((*self.plugin).on_main_thread)(self.plugin) };
        }
        let restart = self.flags.restart.swap(false, Ordering::Relaxed);
        if restart || (rate != self.rate && rate > 0.0) {
            self.deactivate();
            self.read_ports();
            let _ = self.activate(rate);
        }
    }

    fn param_value(&self, id: u32) -> Option<f64> {
        // A change still waiting for the next block: show it already.
        for e in &self.events {
            if unsafe { e.header.kind == EVENT_PARAM_VALUE && e.param.param_id == id } {
                return Some(unsafe { e.param.value });
            }
        }
        let pp = self.ext::<ClapPluginParams>(c"clap.params")?;
        let mut v = 0.0;
        unsafe { (pp.get_value)(self.plugin, id, &mut v) }.then_some(v)
    }

    fn param_text(&self, id: u32, value: f64) -> Option<String> {
        let pp = self.ext::<ClapPluginParams>(c"clap.params")?;
        let mut buf = [0 as c_char; 128];
        unsafe { (pp.value_to_text)(self.plugin, id, value, buf.as_mut_ptr(), buf.len() as u32) }.then(|| cstr(buf.as_ptr()))
    }

    /// Queues a parameter change for the next block. When the plugin is not
    /// processing it is applied at once through `flush`.
    fn set_param(&mut self, id: u32, value: f64) {
        let header = EventHeader { size: std::mem::size_of::<EventParamValue>() as u32, time: 0, space_id: 0, kind: EVENT_PARAM_VALUE, flags: 0 };
        let ev = EventParamValue { header, param_id: id, cookie: ptr::null_mut(), note_id: -1, port_index: -1, channel: -1, key: -1, value };
        self.events.retain(|e| unsafe { e.header.kind != EVENT_PARAM_VALUE || e.param.param_id != id });
        self.events.push(RawEvent { param: ev });
        if !self.processing {
            if let Some(pp) = self.ext::<ClapPluginParams>(c"clap.params") {
                let flush = pp.flush;
                let list = ClapInputEvents { ctx: &mut self.events as *mut Vec<RawEvent> as *mut c_void, size: events_size, get: events_get };
                let out = ClapOutputEvents { ctx: ptr::null_mut(), try_push: events_push };
                unsafe { flush(self.plugin, &list, &out) };
                self.events.clear();
            }
        }
    }

    fn note(&mut self, on: bool, key: u8, vel: f32, time: u32) {
        if self.events.len() > 1024 {
            return;
        }
        let header = EventHeader {
            size: std::mem::size_of::<EventNote>() as u32,
            time,
            space_id: 0,
            kind: if on { EVENT_NOTE_ON } else { EVENT_NOTE_OFF },
            flags: 0,
        };
        let ev = EventNote { header, note_id: -1, port_index: 0, channel: 0, key: key as i16, velocity: vel as f64 };
        self.events.push(RawEvent { note: ev });
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        let st = self.ext::<ClapPluginState>(c"clap.state")?;
        let mut data: Vec<u8> = Vec::new();
        let stream = ClapOStream { ctx: &mut data as *mut Vec<u8> as *mut c_void, write: ostream_write };
        unsafe { (st.save)(self.plugin, &stream) }.then_some(data)
    }

    fn load_state(&mut self, data: &[u8]) -> bool {
        let Some(st) = self.ext::<ClapPluginState>(c"clap.state") else { return false };
        let load = st.load;
        let mut cur = ReadCursor { data, at: 0 };
        let stream = ClapIStream { ctx: &mut cur as *mut ReadCursor as *mut c_void, read: istream_read };
        unsafe { load(self.plugin, &stream) }
    }

    /// Runs one block. `input` (stereo) goes to the first input port; the first
    /// output port is mixed back into `output`.
    fn run(&mut self, input: Option<&[[f32; 2]]>, output: &mut [[f32; 2]], tr: &Transport) {
        if self.rate == 0.0 {
            return;
        }
        if !self.processing {
            self.processing = unsafe { ((*self.plugin).start_processing)(self.plugin) };
            if !self.processing {
                return;
            }
        }
        for chunk_start in (0..output.len()).step_by(MAX_FRAMES as usize) {
            let n = (output.len() - chunk_start).min(MAX_FRAMES as usize);
            let total_in: usize = self.in_channels.iter().sum::<u32>() as usize;
            let main_in = self.in_channels.first().copied().unwrap_or(0) as usize;
            for (i, ch) in self.scratch.iter_mut().enumerate() {
                ch[..n].fill(0.0);
                if let (Some(inp), true) = (input, i < main_in.min(2)) {
                    for (k, f) in inp[chunk_start..chunk_start + n].iter().enumerate() {
                        ch[k] = if main_in == 1 { (f[0] + f[1]) * 0.5 } else { f[i] };
                    }
                }
            }
            let mut ptrs: Vec<*mut f32> = self.scratch.iter_mut().map(|c| c.as_mut_ptr()).collect();
            let mut bufs = Vec::new();
            let mut at = 0usize;
            for &c in self.in_channels.iter().chain(&self.out_channels) {
                bufs.push(ClapAudioBuffer { data32: unsafe { ptrs.as_mut_ptr().add(at) }, data64: ptr::null_mut(), channel_count: c, latency: 0, constant_mask: 0 });
                at += c as usize;
            }
            let (ins, outs) = bufs.split_at_mut(self.in_channels.len());
            let transport = EventTransport {
                header: EventHeader { size: std::mem::size_of::<EventTransport>() as u32, time: 0, space_id: 0, kind: EVENT_TRANSPORT, flags: 0 },
                flags: TRANSPORT_HAS_TEMPO | TRANSPORT_HAS_BEATS_TIMELINE | TRANSPORT_HAS_TIME_SIGNATURE | if tr.playing { TRANSPORT_IS_PLAYING } else { 0 },
                song_pos_beats: (tr.beats * BEATTIME_FACTOR) as i64,
                song_pos_seconds: 0,
                tempo: tr.tempo,
                tempo_inc: 0.0,
                loop_start_beats: 0,
                loop_end_beats: 0,
                loop_start_seconds: 0,
                loop_end_seconds: 0,
                bar_start: ((tr.beats / 4.0).floor() * 4.0 * BEATTIME_FACTOR) as i64,
                bar_number: (tr.beats / 4.0) as i32,
                tsig_num: 4,
                tsig_denom: 4,
            };
            let mut events = if chunk_start == 0 { std::mem::take(&mut self.events) } else { Vec::new() };
            events.sort_by_key(|e| unsafe { e.header.time });
            let in_events = ClapInputEvents { ctx: &mut events as *mut Vec<RawEvent> as *mut c_void, size: events_size, get: events_get };
            let out_events = ClapOutputEvents { ctx: ptr::null_mut(), try_push: events_push };
            let process = ClapProcess {
                steady_time: self.steady,
                frames_count: n as u32,
                transport: &transport,
                audio_inputs: ins.as_ptr(),
                audio_outputs: outs.as_mut_ptr(),
                audio_inputs_count: ins.len() as u32,
                audio_outputs_count: outs.len() as u32,
                in_events: &in_events,
                out_events: &out_events,
            };
            unsafe { ((*self.plugin).process)(self.plugin, &process) };
            self.steady += n as i64;
            if let Some(&c) = self.out_channels.first() {
                let base = total_in;
                for (k, f) in output[chunk_start..chunk_start + n].iter_mut().enumerate() {
                    let l = self.scratch[base][k];
                    let r = if c > 1 { self.scratch[base + 1][k] } else { l };
                    *f = [if l.is_finite() { l } else { 0.0 }, if r.is_finite() { r } else { 0.0 }];
                }
            }
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        if self.plugin.is_null() {
            return;
        }
        self.deactivate();
        unsafe { ((*self.plugin).destroy)(self.plugin) };
    }
}

// ---------------------------------------------------------------- public wrappers

/// A plugin in a project: which one it is, the values saved with the project
/// (applied once it loads), and the running instance.
pub struct PluginSlot<T> {
    pub info: PluginInfo,
    pub state: Option<Vec<u8>>,
    pub values: Vec<(u32, f64)>,
    pub live: Option<T>,
    pub error: Option<String>,
    /// Set by undo: `values` lists every changed parameter, the others go
    /// back to their defaults.
    pub restored: bool,
}

impl<T> PluginSlot<T> {
    pub fn new(info: PluginInfo) -> PluginSlot<T> {
        PluginSlot { info, state: None, values: Vec::new(), live: None, error: None, restored: false }
    }
}

/// Lets the project code reach the shared part of either kind of plugin.
pub trait HasCore {
    fn core(&self) -> &PluginCore;
}

/// Shared by effect and instrument plugins: what the interface needs.
pub struct PluginCore {
    inst: Instance,
}

impl PluginCore {
    pub fn load(info: &PluginInfo, rate: f32, state: Option<&[u8]>) -> Result<PluginCore, String> {
        let mut inst = Instance::new(info, rate as f64)?;
        if let Some(s) = state {
            inst.load_state(s);
        }
        Ok(PluginCore { inst })
    }
    pub fn params(&self) -> &[Param] {
        &self.inst.params
    }
    pub fn value(&self, id: u32) -> Option<f64> {
        self.inst.param_value(id)
    }
    pub fn value_text(&self, id: u32, v: f64) -> Option<String> {
        self.inst.param_text(id, v)
    }
    pub fn set(&mut self, id: u32, v: f64) {
        self.inst.set_param(id, v);
    }
    pub fn save_state(&self) -> Option<Vec<u8>> {
        self.inst.save_state()
    }
    pub fn idle(&mut self, rate: f32) {
        self.inst.idle(rate as f64);
    }
}

pub struct PluginEffect {
    pub core: PluginCore,
    dry: Vec<[f32; 2]>,
}

impl PluginEffect {
    pub fn new(core: PluginCore) -> PluginEffect {
        PluginEffect { core, dry: Vec::new() }
    }
    pub fn process(&mut self, buf: &mut [[f32; 2]], tr: &Transport) {
        self.dry.clear();
        self.dry.extend_from_slice(buf);
        self.core.inst.run(Some(&self.dry), buf, tr);
    }
}

/// An instrument plugin on a channel. Notes are queued with their offset in
/// the next block; sequenced notes get their note-off counted down in frames.
pub struct PluginInstrument {
    pub core: PluginCore,
    pending_offs: Vec<(u8, i64)>,
}

impl HasCore for PluginEffect {
    fn core(&self) -> &PluginCore {
        &self.core
    }
}

impl HasCore for PluginInstrument {
    fn core(&self) -> &PluginCore {
        &self.core
    }
}

impl PluginInstrument {
    pub fn new(core: PluginCore) -> PluginInstrument {
        PluginInstrument { core, pending_offs: Vec::new() }
    }
    /// `time` is the frame within the next block; `gate_frames` counts from it.
    pub fn note_on(&mut self, key: u8, vel: f32, time: u32, gate_frames: Option<i64>) {
        self.core.inst.note(true, key, vel, time);
        if let Some(g) = gate_frames {
            self.pending_offs.push((key, time as i64 + g.max(1)));
        }
    }
    pub fn note_off(&mut self, key: u8) {
        self.core.inst.note(false, key, 0.0, 0);
    }
    pub fn all_off(&mut self) {
        for k in 0..128 {
            self.core.inst.note(false, k, 0.0, 0);
        }
        self.pending_offs.clear();
    }
    /// Renders `out.len()` frames (overwriting `out`).
    pub fn render(&mut self, out: &mut [[f32; 2]], tr: &Transport) {
        let n = out.len() as i64;
        let mut offs = Vec::new();
        self.pending_offs.retain_mut(|(key, left)| {
            if *left <= n {
                offs.push((*key, (*left).clamp(0, n - 1) as u32));
                false
            } else {
                *left -= n;
                true
            }
        });
        for (key, t) in offs {
            self.core.inst.note(false, key, 0.0, t);
        }
        self.core.inst.run(None, out, tr);
    }
}

pub fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_sizes_match_clap() {
        use std::mem::size_of;
        assert_eq!(size_of::<EventHeader>(), 16);
        assert_eq!(size_of::<EventNote>(), 40);
        assert_eq!(size_of::<EventParamValue>(), 56);
        assert_eq!(size_of::<ClapParamInfo>(), 4 + 4 + 8 + 256 + 1024 + 24);
        assert_eq!(size_of::<ClapProcess>(), 8 + 8 + 8 * 3 + 8 + 8 * 2);
        assert_eq!(size_of::<ClapAudioBuffer>(), 32);
    }

    #[test]
    fn hex_round_trips() {
        let d = vec![0u8, 1, 0xab, 0xff];
        assert_eq!(unhex(&hex(&d)), Some(d));
        assert_eq!(unhex("abc"), None);
    }

    #[test]
    fn scan_lines_are_read() {
        let l = parse_scan(Path::new("/x.clap"), "noise\nCLAP\tcom.a.b\tReverb\tAcme\teffect\n");
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].name, "Reverb");
        assert!(!l[0].instrument);
        assert_eq!(cache_from_text(&cache_to_text(&l)), l);
    }
}

/// Loads every installed plugin and runs sound through it. Needs real
/// plugins, so it only runs on request: `cargo test real_plugins -- --ignored`.
#[cfg(test)]
mod real {
    use super::*;

    #[test]
    #[ignore]
    fn real_plugins_load_and_process() {
        // Tests run from target/debug/deps; the program is in target/debug.
        let me = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("beatstudio");
        let list = scan_with(&me);
        assert!(!list.is_empty(), "no CLAP plugins installed");
        let tr = Transport { tempo: 120.0, playing: true, beats: 0.0 };
        for info in &list {
            let core = match PluginCore::load(info, 48000.0, None) {
                Ok(c) => c,
                Err(e) => {
                    println!("SKIP {}: {e}", info.name);
                    continue;
                }
            };
            let params = core.params().len();
            let state = core.save_state().map_or(0, |s| s.len());
            let mut buf = vec![[0.0f32; 2]; 4800];
            if info.instrument {
                let mut inst = PluginInstrument::new(core);
                inst.note_on(60, 0.8, 0, Some(24000));
                let mut loud = 0.0f32;
                for _ in 0..10 {
                    inst.render(&mut buf, &tr);
                    loud = buf.iter().fold(loud, |m, f| m.max(f[0].abs()));
                }
                println!("OK   {:28} instrument params={params:3} state={state:6} peak={loud:.3}", info.name);
            } else {
                let mut fx = PluginEffect::new(core);
                for (i, f) in buf.iter_mut().enumerate() {
                    *f = [((i as f32) * 0.06).sin() * 0.5; 2];
                }
                fx.process(&mut buf, &tr);
                let loud = buf.iter().fold(0.0f32, |m, f| m.max(f[0].abs()));
                assert!(buf.iter().all(|f| f[0].is_finite()));
                println!("OK   {:28} effect     params={params:3} state={state:6} peak={loud:.3}", info.name);
            }
        }
    }
}
