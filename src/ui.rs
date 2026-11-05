// Main window: toolbar, menus, browser, playlist. The editor windows live in
// editors.rs; undo, sound files, plugins and recording in tools.rs.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Key, Pos2, Rect, Sense, Stroke, Ui};

use crate::arrange;
use crate::audiofile;
use crate::engine::{
    self, Automation, Clip, Instrument, Mode, Note, Project, SampleData, Shared, Target, BEATS_PER_BAR, PLAYLIST_TRACKS, STEPS_PER_BAR,
    TEMPO_MAX, TEMPO_MIN, TICKS_PER_BAR, TICKS_PER_STEP,
};
use crate::file;
use crate::midi::{self, MidiEvent, MidiIn};
use crate::plugin::PluginInfo;
use crate::record::{self, Recorder, SharedRecorder};
use crate::settings::{self, Settings};
use crate::smf;
use crate::widgets::*;

/// Typing keyboard to piano, laid out like FL Studio: the bottom row starts at
/// the normal pitch (C5 = key 60), the top row an octave higher. Physical key
/// positions are used, so the layout is the same on AZERTY and QWERTY.
const TYPING_PIANO: [(Key, u8); 30] = [
    (Key::Z, 60),
    (Key::S, 61),
    (Key::X, 62),
    (Key::D, 63),
    (Key::C, 64),
    (Key::V, 65),
    (Key::G, 66),
    (Key::B, 67),
    (Key::H, 68),
    (Key::N, 69),
    (Key::J, 70),
    (Key::M, 71),
    (Key::Comma, 72),
    (Key::Q, 72),
    (Key::Num2, 73),
    (Key::W, 74),
    (Key::Num3, 75),
    (Key::E, 76),
    (Key::R, 77),
    (Key::Num5, 78),
    (Key::T, 79),
    (Key::Num6, 80),
    (Key::Y, 81),
    (Key::Num7, 82),
    (Key::U, 83),
    (Key::I, 84),
    (Key::Num9, 85),
    (Key::O, 86),
    (Key::Num0, 87),
    (Key::P, 88),
];

/// A shortcut key press, ignoring keys taken by the typing keyboard piano (on
/// AZERTY the 6 key types "-", which would also change the pattern).
fn shortcut_pressed(i: &egui::InputState, k: Key, piano: bool) -> bool {
    i.events.iter().any(|e| match e {
        egui::Event::Key { key, physical_key, pressed: true, repeat: false, .. } if *key == k => {
            !(piano && physical_key.is_some_and(|pk| TYPING_PIANO.iter().any(|(m, _)| *m == pk)))
        }
        _ => false,
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Folder {
    Plugins,
    Recent,
    Projects,
    Templates,
    Demos,
    Samples,
    Recorded,
    Rendered,
    Scores,
}

const BROWSER_FOLDERS: [(Folder, &str); 9] = [
    (Folder::Plugins, "Plugin database"),
    (Folder::Recent, "Recent files"),
    (Folder::Projects, "My projects"),
    (Folder::Templates, "Templates"),
    (Folder::Demos, "Demo projects"),
    (Folder::Samples, "Samples"),
    (Folder::Recorded, "Recorded"),
    (Folder::Rendered, "Rendered"),
    (Folder::Scores, "Scores"),
];

/// Height of an automation lane in the playlist.
const AUTO_LANE_H: f32 = 46.0;

#[derive(Clone, Copy, PartialEq)]
pub enum DragKind {
    Move,
    Resize,
}

#[derive(Clone)]
pub struct PianoDrag {
    pub index: usize,
    pub kind: DragKind,
    pub grab_ticks: i64,
    pub grab_key: i32,
    pub orig: Note,
    /// Selected notes moving together (index, where they started).
    pub group: Vec<(usize, Note)>,
}

#[derive(Clone, Copy)]
struct ClipDrag {
    index: usize,
    grab_bars: i64,
}

#[derive(Clone, Copy, PartialEq)]
enum FileAction {
    Open,
    Save,
    SaveTemplate,
    ExportWav,
    ExportMp3,
    ExportMidi,
    /// One WAV per mixer track; the name is the start of each file name.
    ExportStems,
}

impl FileAction {
    fn words(self) -> (&'static str, &'static str) {
        match self {
            FileAction::Open => ("Open project", "Open"),
            FileAction::Save => ("Save project as", "Save"),
            FileAction::SaveTemplate => ("Save as template", "Save"),
            FileAction::ExportWav => ("Export to WAV", "Export"),
            FileAction::ExportMp3 => ("Export to MP3", "Export"),
            FileAction::ExportMidi => ("Export to MIDI", "Export"),
            FileAction::ExportStems => ("Export one WAV per mixer track", "Export"),
        }
    }

    fn dir(self) -> PathBuf {
        match self {
            FileAction::Open | FileAction::Save => file::projects_dir(),
            FileAction::SaveTemplate => file::folder(file::TEMPLATES),
            FileAction::ExportWav | FileAction::ExportMp3 | FileAction::ExportStems => file::folder(file::RENDERED),
            FileAction::ExportMidi => file::folder(file::SCORES),
        }
    }

    fn ext(self) -> &'static str {
        match self {
            FileAction::Open | FileAction::Save | FileAction::SaveTemplate => file::EXTENSION,
            FileAction::ExportWav | FileAction::ExportStems => "wav",
            FileAction::ExportMp3 => "mp3",
            FileAction::ExportMidi => "mid",
        }
    }
}

struct FileDialog {
    action: FileAction,
    name: String,
    error: Option<String>,
}

/// What a file chosen in the disk picker is for.
#[derive(Clone, Copy, PartialEq)]
pub enum PickFor {
    NewSampler,
    ReplaceSample(usize),
    ImportMidi,
}

pub struct FilePicker {
    pub purpose: PickFor,
    pub dir: PathBuf,
}

/// Whose parameters the plugin window shows.
#[derive(Clone, Copy, PartialEq)]
pub enum PluginTarget {
    Channel(usize),
    Effect(usize, usize),
}

/// Where an audio take started.
#[derive(Clone, Copy)]
pub struct TakeStart {
    pub position: f64,
    pub mode: Mode,
    pub pattern: usize,
}

/// Piano roll editing commands, from the keys or the EDIT menu.
#[derive(Clone, Copy, PartialEq)]
pub enum NoteAction {
    SelectAll,
    Copy,
    Cut,
    Paste,
    Duplicate,
    Delete,
}

pub struct StudioApp {
    shared: Shared,
    _stream: Option<cpal::Stream>,
    pub screenshot: Option<PathBuf>,
    frames: u32,
    /// Developer check: pointer events replayed before the screenshot
    /// (frame, event in screenshot pixels), from BEATSTUDIO_INPUT.
    script: Vec<(u32, ScriptEvent)>,
    /// Keys held by the script between a press and its release.
    script_mods: egui::Modifiers,
    shot_requested: bool,
    pub show_browser: bool,
    pub show_playlist: bool,
    pub show_rack: bool,
    pub show_piano: bool,
    pub show_mixer: bool,
    show_about: bool,
    packs_open: bool,
    project_open: bool,
    browser_open: HashSet<&'static str>,
    pub recording: bool,
    pub typing_piano: bool,
    keys_down: HashMap<Key, (usize, u8)>,
    taps: Vec<f64>,
    pub selected_channel: usize,
    pub selected_insert: usize,
    /// The effect slot shown under the slot list of the selected insert.
    pub selected_slot: Option<usize>,
    pub paint: Option<bool>,
    pub piano_drag: Option<PianoDrag>,
    clip_drag: Option<ClipDrag>,
    pub last_note_len: u32,
    pub piano_scrolled: bool,
    /// Selected notes (indices into the list of `note_sel_ctx` = (pattern, channel)).
    pub note_sel: Vec<usize>,
    pub note_sel_ctx: (usize, usize),
    pub note_clipboard: Vec<Note>,
    /// How many times the clipboard was pasted: each paste goes further right.
    pub paste_count: u32,
    /// Where a Ctrl+drag selection box started.
    pub rubber: Option<Pos2>,
    /// The piano roll was clicked last: its editing keys are active.
    pub piano_focus: bool,
    pub hint: String,
    file_dialog: Option<FileDialog>,
    show_info: bool,
    current_file: Option<PathBuf>,
    /// Project text at the last save or open, to tell whether there are changes.
    saved_text: String,
    modified: bool,
    next_check: f64,
    shown_title: String,
    pub settings: Settings,
    saved_settings: Settings,
    restart_audio: bool,
    show_audio: bool,
    show_general: bool,
    show_midi: bool,
    devices: Vec<String>,
    inputs: Vec<String>,
    midi: Option<MidiIn>,
    midi_ports: Vec<String>,
    midi_down: HashMap<u8, (usize, u8)>,
    midi_last: Option<u8>,
    next_midi_check: f64,
    last_autosave: f64,
    autosaved_text: String,
    pub undo: Vec<String>,
    pub redo: Vec<String>,
    pub undo_last: String,
    pub next_undo_check: f64,
    pub samples: HashMap<PathBuf, Arc<SampleData>>,
    pub sample_tx: Sender<(PathBuf, Result<SampleData, String>)>,
    pub sample_rx: Receiver<(PathBuf, Result<SampleData, String>)>,
    pub loading: HashSet<PathBuf>,
    pub failed: HashSet<PathBuf>,
    pub plugins: Vec<PluginInfo>,
    pub plugin_scan: Option<Receiver<Vec<PluginInfo>>>,
    pub plugin_window: Option<PluginTarget>,
    pub plugin_filter: String,
    /// Search text of the Browser's plugin list.
    plugin_search: String,
    pub picker: Option<FilePicker>,
    pub recorder: SharedRecorder,
    pub input_stream: Option<cpal::Stream>,
    pub input_failed: bool,
    pub take: Option<TakeStart>,
    /// Automation point being dragged (clip, point).
    pub auto_drag: Option<(usize, usize)>,
    /// Free drawing on an automation lane (clip, last tick, last value).
    pub auto_draw: Option<(usize, u32, f32)>,
    /// Bar the playlist ruler was right-clicked on.
    pub ruler_bar: u32,
    /// Piano roll shows the other instruments' notes faintly.
    pub ghost_notes: bool,
    mp3_ok: bool,
}

impl StudioApp {
    pub fn new(cc: &eframe::CreationContext<'_>, screenshot: Option<PathBuf>) -> Self {
        apply_style(&cc.egui_ctx);
        let shared: Shared = Arc::new(Mutex::new(Project::new()));
        // The screenshot check runs silent and with default settings.
        let settings = if screenshot.is_none() { Settings::load() } else { Settings::default() };
        cc.egui_ctx.set_zoom_factor(settings.ui_scale);
        let stream = if screenshot.is_none() {
            engine::start_audio(shared.clone(), settings.device.as_deref(), settings.buffer)
        } else {
            None
        };
        if let Ok(mut p) = shared.lock() {
            p.metronome = settings.metronome;
        }
        // Developer check: open every window with a demo song.
        let open_all = std::env::var_os("BEATSTUDIO_OPEN_ALL").is_some();
        if open_all {
            if let Ok(mut p) = shared.lock() {
                let mut demo = file::demo(file::DEMOS[0]);
                demo.metronome = p.metronome;
                *p = demo;
            }
        }
        let saved_text = shared.lock().map(|p| file::to_text(&p)).unwrap_or_default();
        let (sample_tx, sample_rx) = channel();
        let mut app = StudioApp {
            shared,
            _stream: stream,
            script: Vec::new(),
            script_mods: egui::Modifiers::NONE,
            screenshot,
            frames: 0,
            shot_requested: false,
            show_browser: true,
            show_playlist: true,
            show_rack: true,
            show_piano: open_all,
            show_mixer: open_all,
            show_about: false,
            packs_open: true,
            project_open: true,
            browser_open: HashSet::new(),
            recording: false,
            typing_piano: settings.typing_piano,
            keys_down: HashMap::new(),
            taps: Vec::new(),
            selected_channel: if open_all { 4 } else { 0 },
            selected_insert: if open_all { 5 } else { 0 },
            selected_slot: open_all.then_some(1),
            paint: None,
            piano_drag: None,
            clip_drag: None,
            last_note_len: TICKS_PER_STEP * 2,
            piano_scrolled: false,
            note_sel: Vec::new(),
            note_sel_ctx: (0, 0),
            note_clipboard: Vec::new(),
            paste_count: 0,
            rubber: None,
            piano_focus: false,
            hint: "Ready".into(),
            file_dialog: None,
            show_info: false,
            current_file: None,
            undo_last: saved_text.clone(),
            saved_text,
            modified: false,
            next_check: 0.0,
            shown_title: String::new(),
            saved_settings: settings.clone(),
            settings,
            restart_audio: false,
            show_audio: false,
            show_general: false,
            show_midi: false,
            devices: Vec::new(),
            inputs: Vec::new(),
            midi: None,
            midi_ports: Vec::new(),
            midi_down: HashMap::new(),
            midi_last: None,
            next_midi_check: 0.0,
            last_autosave: 0.0,
            autosaved_text: String::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            next_undo_check: 0.0,
            samples: HashMap::new(),
            sample_tx,
            sample_rx,
            loading: HashSet::new(),
            failed: HashSet::new(),
            plugins: Vec::new(),
            plugin_scan: None,
            plugin_window: None,
            plugin_filter: String::new(),
            plugin_search: String::new(),
            picker: None,
            recorder: Arc::new(Mutex::new(Recorder::default())),
            input_stream: None,
            input_failed: false,
            take: None,
            auto_drag: None,
            auto_draw: None,
            ruler_bar: 0,
            ghost_notes: true,
            mp3_ok: audiofile::mp3_available(),
        };
        if app.screenshot.is_some() {
            if let Ok(text) = std::env::var("BEATSTUDIO_INPUT") {
                app.script = parse_script(&text);
            }
        }
        // Developer check: only these windows, e.g. BEATSTUDIO_SHOW=piano,playlist.
        if let Ok(list) = std::env::var("BEATSTUDIO_SHOW") {
            let on = |w: &str| list.split(',').any(|x| x.trim() == w);
            app.show_browser = on("browser");
            app.show_playlist = on("playlist");
            app.show_rack = on("rack");
            app.show_piano = on("piano");
            app.show_mixer = on("mixer");
        }
        if app.screenshot.is_none() {
            app.load_plugin_list();
        }
        app
    }

    // ---------------------------------------------------------------- files

    /// Swaps in another project, keeping the sound device state.
    fn replace_project(&mut self, p: &mut Project, new: Project, file: Option<PathBuf>) {
        if self.modified || file::to_text(p) != self.saved_text {
            self.autosave(p);
        }
        let (sr, ok, metronome) = (p.sample_rate, p.audio_ok, p.metronome);
        *p = new;
        p.sample_rate = sr;
        p.audio_ok = ok;
        p.metronome = metronome;
        self.selected_channel = 0;
        self.selected_insert = 0;
        self.selected_slot = None;
        self.clip_drag = None;
        self.after_project_change(p);
        self.saved_text = file::to_text(p);
        self.modified = false;
        self.current_file = file;
        self.failed.clear();
        self.reset_history(p);
    }

    pub fn open_file(&mut self, p: &mut Project, path: PathBuf) -> Result<(), String> {
        let new = file::open(&path)?;
        self.hint = format!("Opened {}", file::display_name(&path));
        self.replace_project(p, new, Some(path));
        Ok(())
    }

    /// A template or demo starts a new, unsaved project.
    fn start_from(&mut self, p: &mut Project, new: Project, what: &str) {
        self.replace_project(p, new, None);
        self.saved_text = String::new();
        self.hint = format!("New project from {what}");
    }

    fn save_file(&mut self, p: &Project, path: PathBuf) -> Result<(), String> {
        file::save(p, &path).map_err(|e| format!("Could not save: {e}"))?;
        self.hint = format!("Saved {}", path.display());
        self.saved_text = file::to_text(p);
        self.modified = false;
        self.current_file = Some(path);
        Ok(())
    }

    /// Backup copy of unsaved work, kept apart from the user's own files.
    fn autosave(&mut self, p: &Project) {
        if self.screenshot.is_some() {
            return;
        }
        let text = file::to_text(p);
        if text == self.autosaved_text {
            return;
        }
        let path = file::path_for(file::AUTOSAVE_NAME, file::EXTENSION);
        match file::save(p, &path) {
            Ok(()) => {
                self.autosaved_text = text;
                self.hint = format!("Unsaved changes backed up in {}", file::AUTOSAVE_NAME);
            }
            Err(e) => self.hint = format!("Could not back up: {e}"),
        }
    }

    /// Save to the current file, or ask for a name the first time.
    fn quick_save(&mut self, p: &Project) {
        match self.current_file.clone() {
            Some(path) => {
                if let Err(e) = self.save_file(p, path) {
                    self.hint = e;
                }
            }
            None => self.ask_file(FileAction::Save, p),
        }
    }

    fn ask_file(&mut self, action: FileAction, p: &Project) {
        let name = match &self.current_file {
            Some(f) => file::display_name(f),
            None if !p.title.trim().is_empty() => p.title.trim().to_string(),
            None => "Untitled".into(),
        };
        self.file_dialog = Some(FileDialog { action, name, error: None });
    }

    fn file_window(&mut self, ctx: &egui::Context, p: &mut Project) {
        let Some(mut d) = self.file_dialog.take() else { return };
        let (title, verb) = d.action.words();
        let dir = d.action.dir();
        let ext = d.action.ext();
        let mut open = true;
        let mut done = false;
        egui::Window::new(title).open(&mut open).collapsible(false).resizable(false).default_width(340.0).show(ctx, |ui| {
            ui.label(egui::RichText::new(format!("Folder: {}", dir.display())).color(TEXT_DIM).size(11.0));
            ui.add_space(4.0);
            let existing = file::list_files(&dir, &[ext]);
            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                if existing.is_empty() {
                    ui.label(egui::RichText::new("No files here yet").color(TEXT_DIM));
                }
                for path in &existing {
                    let name = file::display_name(path);
                    let r = file_item(ui, &name, name == d.name);
                    if r.clicked() {
                        d.name = name.clone();
                    }
                    if r.double_clicked() && d.action == FileAction::Open {
                        match self.open_file(p, path.clone()) {
                            Ok(()) => done = true,
                            Err(e) => d.error = Some(e),
                        }
                    }
                }
            });
            ui.add_space(4.0);
            let mut go = false;
            ui.horizontal(|ui| {
                ui.label("Name");
                let r = ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(200.0));
                go |= r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                go |= ui.button(verb).clicked();
            });
            if d.action == FileAction::ExportMidi {
                let what = if p.mode == Mode::Song && !p.clips.is_empty() { "the song (playlist)" } else { "the current pattern" };
                ui.label(egui::RichText::new(format!("Exports {what}.")).color(TEXT_DIM).size(11.0));
            }
            if go && !done {
                let path = file::path_in(&dir, &d.name, ext);
                let exported = |secs: f32| format!("Exported {:.1} s to {}", secs, path.display());
                let result = match d.action {
                    FileAction::Open => self.open_file(p, path.clone()),
                    FileAction::Save => self.save_file(p, path.clone()),
                    FileAction::SaveTemplate => match file::save(p, &path) {
                        Ok(()) => {
                            self.hint = format!("Template saved: {}", d.name);
                            Ok(())
                        }
                        Err(e) => Err(format!("Could not save: {e}")),
                    },
                    FileAction::ExportWav => match file::export_wav(p, &path) {
                        Ok(secs) => {
                            self.hint = exported(secs);
                            Ok(())
                        }
                        Err(e) => Err(format!("Could not export: {e}")),
                    },
                    FileAction::ExportMp3 => match file::export_mp3(p, &path) {
                        Ok(secs) => {
                            self.hint = exported(secs);
                            Ok(())
                        }
                        Err(e) => Err(format!("Could not export: {e}")),
                    },
                    FileAction::ExportStems => match file::export_stems(p, &dir, &d.name) {
                        Ok((files, _)) if files.is_empty() => Err("No instrument plays into a mixer track".into()),
                        Ok((files, secs)) => {
                            self.hint = format!("Exported {} tracks of {:.1} s to {}", files.len(), secs, dir.display());
                            Ok(())
                        }
                        Err(e) => Err(format!("Could not export: {e}")),
                    },
                    FileAction::ExportMidi => match file::write_safely(&path, &smf::export(p)) {
                        Ok(()) => {
                            self.hint = format!("Exported MIDI to {}", path.display());
                            Ok(())
                        }
                        Err(e) => Err(format!("Could not export: {e}")),
                    },
                };
                match result {
                    Ok(()) => done = true,
                    Err(e) => d.error = Some(e),
                }
            }
            if let Some(e) = &d.error {
                ui.colored_label(RED, e);
            }
        });
        if open && !done {
            self.file_dialog = Some(d);
        }
    }

    fn info_window(&mut self, ctx: &egui::Context, p: &mut Project) {
        if !self.show_info {
            return;
        }
        let mut open = true;
        egui::Window::new("Project info").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            egui::Grid::new("info").num_columns(2).show(ui, |ui| {
                ui.label("Title");
                ui.text_edit_singleline(&mut p.title);
                ui.end_row();
                ui.label("Author");
                ui.text_edit_singleline(&mut p.author);
                ui.end_row();
            });
        });
        self.show_info = open;
    }

    fn audio_window(&mut self, ctx: &egui::Context, p: &Project) {
        if !self.show_audio {
            return;
        }
        let mut open = true;
        let input_peak = self.recorder.lock().map(|r| r.peak).unwrap_or(0.0);
        egui::Window::new("Audio settings").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            egui::Grid::new("audio").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label("Output");
                ui.horizontal(|ui| {
                    let shown = self.settings.device.clone().unwrap_or_else(|| "System default".into());
                    egui::ComboBox::from_id_salt("device").selected_text(shown).width(240.0).show_ui(ui, |ui| {
                        if ui.selectable_label(self.settings.device.is_none(), "System default").clicked() {
                            self.settings.device = None;
                            self.restart_audio = true;
                        }
                        for d in &self.devices {
                            if ui.selectable_label(self.settings.device.as_ref() == Some(d), d).clicked() {
                                self.settings.device = Some(d.clone());
                                self.restart_audio = true;
                            }
                        }
                    });
                    if ui.small_button("Refresh").clicked() {
                        self.devices = engine::output_devices();
                        self.inputs = record::input_devices();
                    }
                });
                ui.end_row();

                ui.label("Buffer");
                let ms = |frames: u32| frames as f32 / p.sample_rate * 1000.0;
                let shown = match self.settings.buffer {
                    Some(b) => format!("{b} samples ({:.1} ms)", ms(b)),
                    None => "Automatic".into(),
                };
                egui::ComboBox::from_id_salt("buffer").selected_text(shown).width(240.0).show_ui(ui, |ui| {
                    if ui.selectable_label(self.settings.buffer.is_none(), "Automatic").clicked() {
                        self.settings.buffer = None;
                        self.restart_audio = true;
                    }
                    for b in settings::BUFFER_SIZES {
                        let label = format!("{b} samples ({:.1} ms)", ms(b));
                        if ui.selectable_label(self.settings.buffer == Some(b), label).clicked() {
                            self.settings.buffer = Some(b);
                            self.restart_audio = true;
                        }
                    }
                });
                ui.end_row();

                ui.label("Status");
                if p.audio_ok {
                    ui.label(format!("playing at {} Hz", p.sample_rate as u32));
                } else {
                    ui.colored_label(RED, "no sound output");
                }
                ui.end_row();

                ui.label("Input");
                let shown = self.settings.input_device.clone().unwrap_or_else(|| "System default".into());
                let before = self.settings.input_device.clone();
                egui::ComboBox::from_id_salt("input").selected_text(shown).width(240.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.settings.input_device, None, "System default");
                    for d in &self.inputs {
                        ui.selectable_value(&mut self.settings.input_device, Some(d.clone()), d);
                    }
                });
                if self.settings.input_device != before {
                    self.input_stream = None;
                    self.input_failed = false;
                }
                ui.end_row();

                ui.label("Record audio");
                ui.checkbox(&mut self.settings.record_audio, "the record button also records sound from the input");
                ui.end_row();

                if self.input_stream.is_some() {
                    ui.label("Input level");
                    let (rect, _) = ui.allocate_exact_size(vec2(200.0, 12.0), Sense::hover());
                    meter_h(ui.painter(), rect, [input_peak; 2]);
                    ui.end_row();
                }
            });
            ui.label(egui::RichText::new("A smaller buffer reacts faster but may crackle.").color(TEXT_DIM).size(11.0));
        });
        self.show_audio = open;
    }

    fn general_window(&mut self, ctx: &egui::Context, p: &mut Project) {
        if !self.show_general {
            return;
        }
        let mut open = true;
        egui::Window::new("General settings").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            egui::Grid::new("general").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label("Interface size");
                egui::ComboBox::from_id_salt("scale").selected_text(format!("{:.0}%", self.settings.ui_scale * 100.0)).show_ui(ui, |ui| {
                    for s in settings::UI_SCALES {
                        if ui.selectable_label(self.settings.ui_scale == s, format!("{:.0}%", s * 100.0)).clicked() {
                            self.settings.ui_scale = s;
                            ctx.set_zoom_factor(s);
                        }
                    }
                });
                ui.end_row();

                ui.label("Automatic backup");
                let label = |m: u32| if m == 0 { "Off".to_string() } else { format!("every {m} min") };
                egui::ComboBox::from_id_salt("autosave").selected_text(label(self.settings.autosave_minutes)).show_ui(ui, |ui| {
                    for m in settings::AUTOSAVE_CHOICES {
                        ui.selectable_value(&mut self.settings.autosave_minutes, m, label(m));
                    }
                });
                ui.end_row();

                ui.label("Metronome");
                ui.checkbox(&mut p.metronome, "click on every beat (Ctrl+M)");
                ui.end_row();

                ui.label("Typing keyboard");
                if ui.checkbox(&mut self.typing_piano, "play notes with the computer keyboard").changed() && !self.typing_piano {
                    self.release_all_keys(p);
                }
                ui.end_row();
            });
            ui.label(
                egui::RichText::new(format!("Backups go to {}.{} in {}", file::AUTOSAVE_NAME, file::EXTENSION, file::projects_dir().display()))
                    .color(TEXT_DIM)
                    .size(11.0),
            );
        });
        self.show_general = open;
    }

    /// Applies changed settings: restarts the sound output if needed and writes
    /// the settings file. Runs with the project unlocked (the audio thread locks it).
    fn apply_settings(&mut self) {
        if self.restart_audio {
            self.restart_audio = false;
            self._stream = None;
            self._stream = engine::start_audio(self.shared.clone(), self.settings.device.as_deref(), self.settings.buffer);
            if self._stream.is_none() {
                self.hint = "Could not open that sound output".into();
            }
        }
        if self.settings != self.saved_settings && self.screenshot.is_none() {
            if let Err(e) = self.settings.save() {
                self.hint = format!("Could not save settings: {e}");
            }
            self.saved_settings = self.settings.clone();
        }
    }

    /// Notices unsaved changes (twice a second) and keeps the window title current.
    fn track_changes(&mut self, ctx: &egui::Context, p: &Project) {
        let now = ctx.input(|i| i.time);
        if now >= self.next_check {
            self.next_check = now + 0.5;
            self.modified = file::to_text(p) != self.saved_text;
        }
        let minutes = self.settings.autosave_minutes;
        if minutes == 0 {
            self.last_autosave = now;
        } else if now >= self.last_autosave + minutes as f64 * 60.0 {
            self.last_autosave = now;
            if self.modified {
                self.autosave(p);
            }
        }
        let name = match &self.current_file {
            Some(f) => file::display_name(f),
            None if !p.title.trim().is_empty() => p.title.trim().to_string(),
            None => "Untitled".into(),
        };
        let title = format!("{}{} - Beat Studio", name, if self.modified { " *" } else { "" });
        if title != self.shown_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.shown_title = title;
        }
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else { return };
        self.frames += 1;
        ctx.request_repaint();
        let script_end = self.script.last().map_or(0, |e| e.0 + 30);
        if self.frames >= 120.max(script_end) && !self.shot_requested {
            self.shot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
        }
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot {
            if let Err(e) = save_png(&path, &image) {
                eprintln!("could not save screenshot: {e}");
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context, p: &mut Project) {
        let (save, save_as, open, export, metro) = ctx.input(|i| {
            let cmd = i.modifiers.command;
            (
                cmd && !i.modifiers.shift && i.key_pressed(Key::S),
                cmd && i.modifiers.shift && i.key_pressed(Key::S),
                cmd && i.key_pressed(Key::O),
                cmd && i.key_pressed(Key::R),
                cmd && i.key_pressed(Key::M),
            )
        });
        if metro {
            p.metronome = !p.metronome;
        }
        if save {
            self.quick_save(p);
        }
        if save_as {
            self.ask_file(FileAction::Save, p);
        }
        if open {
            self.ask_file(FileAction::Open, p);
        }
        if export {
            self.ask_file(FileAction::ExportWav, p);
        }
        if !ctx.wants_keyboard_input() && self.piano_focus && self.show_piano {
            self.piano_keys(ctx, p);
        }
        if ctx.wants_keyboard_input() || ctx.input(|i| i.modifiers.command) {
            self.release_all_keys(p);
            return;
        }
        if self.typing_piano {
            self.typing_keys(ctx, p);
        }
        let piano = self.typing_piano;
        ctx.input(|i| {
            let key_pressed = |k: Key| shortcut_pressed(i, k, piano);
            if key_pressed(Key::Space) {
                p.playing = !p.playing;
            }
            if i.key_pressed(Key::F5) {
                self.show_playlist = !self.show_playlist;
            }
            if i.key_pressed(Key::F6) {
                self.show_rack = !self.show_rack;
            }
            if i.key_pressed(Key::F7) {
                self.show_piano = !self.show_piano;
            }
            if i.key_pressed(Key::F8) {
                self.show_browser = !self.show_browser;
            }
            if i.key_pressed(Key::F9) {
                self.show_mixer = !self.show_mixer;
            }
            if key_pressed(Key::L) {
                p.mode = if p.mode == Mode::Pattern { Mode::Song } else { Mode::Pattern };
                p.position = 0.0;
            }
            if key_pressed(Key::Plus) || key_pressed(Key::Equals) {
                p.ensure_pattern(p.current_pattern + 1);
                p.current_pattern += 1;
            }
            if key_pressed(Key::Minus) && p.current_pattern > 0 {
                p.current_pattern -= 1;
            }
        });
    }

    fn typing_keys(&mut self, ctx: &egui::Context, p: &mut Project) {
        // Key releases are missed while the window is in the background.
        if !ctx.input(|i| i.viewport().focused.unwrap_or(true)) {
            self.release_all_keys(p);
            return;
        }
        let events: Vec<(Key, bool)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key { key, physical_key, pressed, repeat: false, .. } => Some((physical_key.unwrap_or(*key), *pressed)),
                    _ => None,
                })
                .collect()
        });
        for (key, pressed) in events {
            let Some(&(_, note)) = TYPING_PIANO.iter().find(|(k, _)| *k == key) else { continue };
            if pressed {
                if self.keys_down.contains_key(&key) {
                    continue;
                }
                let n = self.live_note_on(p, note, engine::NOTE_VELOCITY);
                self.keys_down.insert(key, n);
            } else if let Some(n) = self.keys_down.remove(&key) {
                p.releases.push(n);
            }
        }
    }

    /// Plays a note on the selected channel until it is released, and writes it
    /// into the pattern while recording. Returns what to release later.
    fn live_note_on(&mut self, p: &mut Project, key: u8, vel: u8) -> (usize, u8) {
        let ch = self.selected_channel.min(p.channels.len() - 1);
        let ahead = self.recording && p.record_note(ch, key, vel);
        if self.recording && p.mode == Mode::Song {
            self.hint = "Note recording works in pattern mode (PAT)".into();
        }
        if !ahead {
            p.held.push((ch, key, vel));
        }
        (ch, key)
    }

    /// Notes from the MIDI keyboard. Runs whatever has keyboard focus.
    fn midi_keys(&mut self, p: &mut Project) {
        let Some(m) = &self.midi else {
            p.releases.extend(self.midi_down.drain().map(|(_, n)| n));
            return;
        };
        for e in m.events() {
            match e {
                MidiEvent::On { key, vel } => {
                    if let Some(n) = self.midi_down.remove(&key) {
                        p.releases.push(n);
                    }
                    let n = self.live_note_on(p, key, vel);
                    self.midi_down.insert(key, n);
                    self.midi_last = Some(key);
                }
                MidiEvent::Off { key } => {
                    if let Some(n) = self.midi_down.remove(&key) {
                        p.releases.push(n);
                    }
                }
            }
        }
    }

    /// Every 3 s: notices an unplugged keyboard and connects to the chosen one.
    /// Runs with the project unlocked, as opening a port can take a moment.
    fn midi_connect(&mut self, now: f64) {
        if self.screenshot.is_some() || now < self.next_midi_check {
            return;
        }
        self.next_midi_check = now + 3.0;
        self.midi_ports = midi::input_ports();
        let want = self.settings.midi_input.clone();
        if let Some(m) = &self.midi {
            let unplugged = !self.midi_ports.contains(&m.name);
            if unplugged || (want != midi::AUTO && want != m.name) {
                if unplugged {
                    self.hint = format!("MIDI keyboard disconnected: {}", m.name);
                }
                self.midi = None;
            }
        }
        if self.midi.is_none() && want != midi::OFF && !self.midi_ports.is_empty() {
            self.midi = MidiIn::open(&want);
            if let Some(m) = &self.midi {
                self.hint = format!("MIDI keyboard: {}", m.name);
            }
        }
    }

    fn midi_window(&mut self, ctx: &egui::Context) {
        if !self.show_midi {
            return;
        }
        let mut open = true;
        egui::Window::new("MIDI settings").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            egui::Grid::new("midi").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label("Input");
                ui.horizontal(|ui| {
                    let label = |v: &str| match v {
                        midi::AUTO => "Automatic (first keyboard found)".to_string(),
                        midi::OFF => "Off".to_string(),
                        v => v.to_string(),
                    };
                    let before = self.settings.midi_input.clone();
                    egui::ComboBox::from_id_salt("midi_in").selected_text(label(&before)).width(260.0).show_ui(ui, |ui| {
                        let choices = [midi::AUTO.to_string(), midi::OFF.to_string()].into_iter().chain(self.midi_ports.iter().cloned());
                        for c in choices {
                            let text = label(&c);
                            ui.selectable_value(&mut self.settings.midi_input, c, text);
                        }
                    });
                    if ui.small_button("Refresh").clicked() || self.settings.midi_input != before {
                        if self.settings.midi_input == midi::OFF {
                            self.midi = None;
                        }
                        self.next_midi_check = 0.0;
                    }
                });
                ui.end_row();

                ui.label("Status");
                match &self.midi {
                    Some(m) => ui.label(format!("connected to {}", m.name)),
                    None if self.settings.midi_input == midi::OFF => ui.label("off"),
                    None => ui.colored_label(RED, "no MIDI keyboard found"),
                };
                ui.end_row();

                ui.label("Last note");
                ui.label(self.midi_last.map(key_name).unwrap_or_else(|| "-".into()));
                ui.end_row();
            });
            ui.label(
                egui::RichText::new("Notes play on the selected channel. Turn on recording to write them into the pattern.")
                    .color(TEXT_DIM)
                    .size(11.0),
            );
        });
        self.show_midi = open;
    }

    fn release_all_keys(&mut self, p: &mut Project) {
        p.releases.extend(self.keys_down.drain().map(|(_, n)| n));
    }

    /// Tempo from the average gap between the last taps; a pause of 2 s starts over.
    fn tap_tempo(&mut self, now: f64, p: &mut Project) {
        if self.taps.last().is_some_and(|&t| now - t > 2.0) {
            self.taps.clear();
        }
        self.taps.push(now);
        if self.taps.len() > 8 {
            self.taps.remove(0);
        }
        if self.taps.len() >= 2 {
            let gap = (self.taps[self.taps.len() - 1] - self.taps[0]) / (self.taps.len() - 1) as f64;
            p.tempo = engine::quantize_tempo((60.0 / gap).round());
            self.hint = format!("Tap tempo: {:.0} BPM", p.tempo);
        } else {
            self.hint = "Tap tempo: keep tapping in time".into();
        }
    }

    /// Right-click menu on a knob: automate it in the playlist.
    pub fn automate_menu(&mut self, resp: &egui::Response, p: &mut Project, target: Target) {
        resp.context_menu(|ui| {
            let exists = p.automations.iter().any(|a| a.target == target);
            let label = if exists { "Show automation clip" } else { "Create automation clip" };
            if ui.button(label).clicked() {
                self.create_automation(p, target);
                ui.close_menu();
            }
        });
    }

    pub fn create_automation(&mut self, p: &mut Project, target: Target) {
        self.show_playlist = true;
        if p.automations.iter().any(|a| a.target == target) {
            self.hint = format!("{} is already automated - see the top of the playlist", target.label(p));
            return;
        }
        let (lo, hi) = target.range();
        let v = target.base_value(p).unwrap_or(hi).clamp(lo, hi);
        let end = p.song_length_ticks().max(4 * TICKS_PER_BAR);
        p.automations.push(Automation { target, points: vec![(0, v), (end, v)] });
        self.hint = format!("Automation for {}: click or drag on its lane in the playlist (Shift+drag draws freely)", target.label(p));
    }

    // ---------------------------------------------------------------- top bar

    fn top_bar(&mut self, ctx: &egui::Context, p: &mut Project) {
        egui::TopBottomPanel::top("top")
            .frame(egui::Frame::none().fill(HEADER).inner_margin(egui::Margin::symmetric(6.0, 4.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    self.menus(ui, p);
                    ui.add_space(12.0);
                    // One line: a long hint is cut rather than pushing the bars down.
                    ui.add(egui::Label::new(egui::RichText::new(&self.hint).color(TEXT_DIM).size(11.0)).truncate());
                });
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
                    self.transport(ui, p);
                });
            });
    }

    pub fn instrument_plugin_menu(&mut self, ui: &mut Ui, p: &mut Project, replace: Option<usize>) {
        let list: Vec<PluginInfo> = self.plugins.iter().filter(|i| i.instrument).cloned().collect();
        if list.is_empty() {
            let msg = if self.plugin_scan.is_some() { "Looking for plugins..." } else { "No CLAP instruments found" };
            ui.label(egui::RichText::new(msg).color(TEXT_DIM));
        }
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for info in list {
                if ui.button(&info.name).on_hover_text(&info.vendor).clicked() {
                    self.add_plugin_instrument(p, &info, replace);
                    ui.close_menu();
                }
            }
        });
    }

    pub fn effect_plugin_menu(&mut self, ui: &mut Ui, p: &mut Project, insert: usize, slot: usize) {
        let list: Vec<PluginInfo> = self.plugins.iter().filter(|i| !i.instrument).cloned().collect();
        if list.is_empty() {
            let msg = if self.plugin_scan.is_some() { "Looking for plugins..." } else { "No CLAP effects found" };
            ui.label(egui::RichText::new(msg).color(TEXT_DIM));
        }
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for info in list {
                if ui.button(&info.name).on_hover_text(&info.vendor).clicked() {
                    self.add_plugin_effect(p, &info, insert, slot);
                    ui.close_menu();
                }
            }
        });
    }

    pub fn add_channel_menu(&mut self, ui: &mut Ui, p: &mut Project) {
        for inst in Instrument::ALL {
            if ui.button(inst.label()).clicked() {
                p.add_channel(inst);
                self.selected_channel = p.channels.len() - 1;
                ui.close_menu();
            }
        }
        ui.separator();
        if ui.button("Sampler (sound file)...").clicked() {
            self.pick(PickFor::NewSampler);
            ui.close_menu();
        }
        ui.menu_button("Plugin instrument", |ui| self.instrument_plugin_menu(ui, p, None));
    }

    fn menus(&mut self, ui: &mut Ui, p: &mut Project) {
        ui.menu_button("FILE", |ui| {
            if ui.button("New").clicked() {
                self.replace_project(p, Project::new(), None);
                ui.close_menu();
            }
            ui.menu_button("New from template", |ui| {
                let templates = file::list_files(&file::folder(file::TEMPLATES), &[file::EXTENSION]);
                if templates.is_empty() {
                    ui.label(egui::RichText::new("No templates yet: use \"Save as template\"").color(TEXT_DIM));
                }
                for path in templates {
                    if ui.button(file::display_name(&path)).clicked() {
                        match file::open(&path) {
                            Ok(new) => self.start_from(p, new, &file::display_name(&path)),
                            Err(e) => self.hint = e,
                        }
                        ui.close_menu();
                    }
                }
                ui.separator();
                for name in file::DEMOS {
                    if ui.button(format!("Demo: {name}")).clicked() {
                        self.start_from(p, file::demo(name), name);
                        ui.close_menu();
                    }
                }
            });
            if ui.button("Open...   Ctrl+O").clicked() {
                self.ask_file(FileAction::Open, p);
                ui.close_menu();
            }
            ui.menu_button("Open recent", |ui| {
                let recent = file::list_projects();
                if recent.is_empty() {
                    ui.label(egui::RichText::new("No projects saved yet").color(TEXT_DIM));
                }
                for path in recent.into_iter().take(10) {
                    if ui.button(file::display_name(&path)).clicked() {
                        if let Err(e) = self.open_file(p, path) {
                            self.hint = e;
                        }
                        ui.close_menu();
                    }
                }
            });
            if ui.button("Save   Ctrl+S").clicked() {
                self.quick_save(p);
                ui.close_menu();
            }
            if ui.button("Save as...   Ctrl+Shift+S").clicked() {
                self.ask_file(FileAction::Save, p);
                ui.close_menu();
            }
            if ui.button("Save as template...").clicked() {
                self.ask_file(FileAction::SaveTemplate, p);
                ui.close_menu();
            }
            ui.separator();
            ui.menu_button("Import", |ui| {
                if ui.button("Sound file (new sampler channel)...").clicked() {
                    self.pick(PickFor::NewSampler);
                    ui.close_menu();
                }
                if ui.button("MIDI file (new pattern)...").clicked() {
                    self.pick(PickFor::ImportMidi);
                    ui.close_menu();
                }
            });
            ui.menu_button("Export", |ui| {
                if ui.button("WAV file...   Ctrl+R").clicked() {
                    self.ask_file(FileAction::ExportWav, p);
                    ui.close_menu();
                }
                let mp3 = ui.add_enabled(self.mp3_ok, egui::Button::new("MP3 file..."));
                if mp3.on_disabled_hover_text("Needs the LAME library (package \"lame\")").clicked() {
                    self.ask_file(FileAction::ExportMp3, p);
                    ui.close_menu();
                }
                if ui.button("MIDI file...").clicked() {
                    self.ask_file(FileAction::ExportMidi, p);
                    ui.close_menu();
                }
                if ui.button("Mixer tracks (one WAV each)...").on_hover_text("Stems: each mixer track that an instrument plays into, alone").clicked() {
                    self.ask_file(FileAction::ExportStems, p);
                    ui.close_menu();
                }
            });
            if ui.button("Project info...").clicked() {
                self.show_info = true;
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Clear current pattern").clicked() {
                for list in &mut p.patterns[p.current_pattern].notes {
                    list.clear();
                }
                ui.close_menu();
            }
        });
        ui.menu_button("EDIT", |ui| {
            if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo   Ctrl+Z")).clicked() {
                self.undo_step(p, true);
                ui.close_menu();
            }
            if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo   Ctrl+Y")).clicked() {
                self.undo_step(p, false);
                ui.close_menu();
            }
            ui.separator();
            ui.label(egui::RichText::new("Piano roll").color(TEXT_DIM).size(11.0));
            for (label, action) in [
                ("Select all   Ctrl+A", NoteAction::SelectAll),
                ("Copy   Ctrl+C", NoteAction::Copy),
                ("Cut   Ctrl+X", NoteAction::Cut),
                ("Paste   Ctrl+V", NoteAction::Paste),
                ("Duplicate   Ctrl+B", NoteAction::Duplicate),
                ("Delete   Del", NoteAction::Delete),
            ] {
                if ui.button(label).clicked() {
                    self.note_action(p, action);
                    ui.close_menu();
                }
            }
            ui.separator();
            if ui.button("Clear playlist").clicked() {
                p.clips.clear();
                ui.close_menu();
            }
        });
        ui.menu_button("ADD", |ui| self.add_channel_menu(ui, p));
        ui.menu_button("PATTERNS", |ui| {
            if ui.button("New pattern").clicked() {
                let n = p.patterns.len();
                p.ensure_pattern(n);
                p.current_pattern = n;
                ui.close_menu();
            }
            if ui.button("Clone pattern").clicked() {
                let mut copy = p.patterns[p.current_pattern].clone();
                copy.name = format!("Pattern {}", p.patterns.len() + 1);
                p.patterns.push(copy);
                p.current_pattern = p.patterns.len() - 1;
                ui.close_menu();
            }
            if ui.add_enabled(p.patterns.len() > 1, egui::Button::new("Delete pattern")).on_hover_text("Also removes its clips from the playlist (Ctrl+Z to undo)").clicked() {
                let cur = p.current_pattern;
                p.remove_pattern(cur);
                ui.close_menu();
            }
            ui.horizontal(|ui| {
                ui.label("Name");
                let cur = p.current_pattern;
                ui.text_edit_singleline(&mut p.patterns[cur].name);
            });
        });
        ui.menu_button("VIEW", |ui| {
            ui.checkbox(&mut self.show_playlist, "Playlist   F5");
            ui.checkbox(&mut self.show_rack, "Channel rack   F6");
            ui.checkbox(&mut self.show_piano, "Piano roll   F7");
            ui.checkbox(&mut self.show_browser, "Browser   F8");
            ui.checkbox(&mut self.show_mixer, "Mixer   F9");
        });
        ui.menu_button("OPTIONS", |ui| {
            ui.checkbox(&mut p.metronome, "Metronome   Ctrl+M");
            if ui.checkbox(&mut self.typing_piano, "Typing keyboard to piano").changed() && !self.typing_piano {
                self.release_all_keys(p);
            }
            ui.checkbox(&mut self.settings.record_audio, "Record audio from the input");
            ui.separator();
            if ui.button("Audio settings...").clicked() {
                self.show_audio = true;
                self.devices = engine::output_devices();
                self.inputs = record::input_devices();
                ui.close_menu();
            }
            if ui.button("MIDI settings...").clicked() {
                self.show_midi = true;
                self.next_midi_check = 0.0;
                ui.close_menu();
            }
            if ui.button("General settings...").clicked() {
                self.show_general = true;
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Rescan plugins").clicked() {
                self.start_plugin_scan();
                ui.close_menu();
            }
        });
        ui.menu_button("TOOLS", |ui| {
            if ui.button("Fill every 4 steps").clicked() {
                let ch = self.selected_channel.min(p.channels.len() - 1);
                for s in (0..STEPS_PER_BAR).step_by(4) {
                    p.patterns[p.current_pattern].set_step(ch, s, true);
                }
                ui.close_menu();
            }
            ui.menu_button("Automate", |ui| {
                if ui.button("Tempo").clicked() {
                    self.create_automation(p, Target::Tempo);
                    ui.close_menu();
                }
                if ui.button("Master volume").clicked() {
                    self.create_automation(p, Target::MixerVolume(0));
                    ui.close_menu();
                }
                let ch = self.selected_channel.min(p.channels.len() - 1);
                if ui.button(format!("{} volume", p.channels[ch].name)).clicked() {
                    self.create_automation(p, Target::ChannelVolume(ch));
                    ui.close_menu();
                }
                if ui.button(format!("{} pan", p.channels[ch].name)).clicked() {
                    self.create_automation(p, Target::ChannelPan(ch));
                    ui.close_menu();
                }
                ui.label(egui::RichText::new("or right-click any knob").color(TEXT_DIM).size(10.0));
            });
        });
        ui.menu_button("HELP", |ui| {
            if ui.button("About").clicked() {
                self.show_about = true;
                ui.close_menu();
            }
        });
    }

    fn transport(&mut self, ui: &mut Ui, p: &mut Project) {
        egui::Frame::none().fill(PANEL_DEEP).rounding(3.0).inner_margin(3.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (mode, label) in [(Mode::Pattern, "PAT"), (Mode::Song, "SONG")] {
                    if led_button(ui, label, p.mode == mode).clicked() {
                        p.mode = mode;
                        p.position = 0.0;
                    }
                }
            });
        });

        let play = big_button(ui, if p.playing { "pause" } else { "play" }, p.playing.then_some(LED_ON));
        if play.on_hover_text("Play / pause (Space)").clicked() {
            p.playing = !p.playing;
        }
        if big_button(ui, "stop", None).on_hover_text("Stop").clicked() {
            p.stop();
        }
        let rec = big_button(ui, "record", Some(if self.recording { RED } else { Color32::from_rgb(0xB0, 0x60, 0x5C) }));
        let rec_tip = if self.settings.record_audio {
            "Record: notes from the keyboards, and sound from the input (right-click for options)"
        } else {
            "Record: notes from the keyboards go into the pattern while it plays (right-click for options)"
        };
        let rec = rec.on_hover_text(rec_tip);
        if rec.clicked() {
            self.recording = !self.recording;
            self.hint = match (self.recording, self.settings.record_audio) {
                (true, true) => "Recording armed: press play to record notes and sound".into(),
                (true, false) => "Recording armed: press play, then play notes on the keyboard".into(),
                (false, _) => "Recording off".into(),
            };
        }
        rec.context_menu(|ui| {
            ui.label(egui::RichText::new("Notes are always recorded").color(TEXT_DIM));
            ui.checkbox(&mut self.settings.record_audio, "Also record audio from the input");
            if ui.button("Audio settings...").clicked() {
                self.show_audio = true;
                self.devices = engine::output_devices();
                self.inputs = record::input_devices();
                ui.close_menu();
            }
        });
        if led_button(ui, "METRO", p.metronome).on_hover_text("Metronome (Ctrl+M)").clicked() {
            p.metronome = !p.metronome;
        }

        let automated = p.automation_tick().is_some() && p.automations.iter().any(|a| a.target == Target::Tempo);
        lcd(ui, |ui| {
            if automated {
                ui.label(egui::RichText::new(format!("{:.3}", p.live_tempo())).color(ACCENT))
                    .on_hover_text("The tempo follows its automation clip");
            } else {
                let r = ui.add(egui::DragValue::new(&mut p.tempo).range(TEMPO_MIN..=TEMPO_MAX).speed(0.2).fixed_decimals(3));
                p.tempo = engine::quantize_tempo(p.tempo);
                if r.hovered() {
                    self.hint = "Tempo in beats per minute - drag up/down or double-click to type".into();
                }
                self.automate_menu(&r, p, Target::Tempo);
            }
            ui.label(egui::RichText::new("BPM").color(TEXT_DIM).size(9.0));
        });
        if ui.small_button("TAP").on_hover_text("Click in time to set the tempo").clicked() {
            let now = ui.input(|i| i.time);
            self.tap_tempo(now, p);
        }

        let pos = p.position as u32;
        let bar = pos / TICKS_PER_BAR + 1;
        let step = (pos % TICKS_PER_BAR) / TICKS_PER_STEP + 1;
        let tick = pos % TICKS_PER_STEP;
        lcd(ui, |ui| {
            ui.label(egui::RichText::new(format!("{bar}:{step:02}:{tick:02}")).color(LCD_TEXT).font(FontId::monospace(16.0)));
            ui.label(egui::RichText::new("B:S:T").color(TEXT_DIM).size(9.0));
        });

        egui::Frame::none().fill(PANEL_DEEP).rounding(3.0).inner_margin(3.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if ui.small_button("◀").clicked() && p.current_pattern > 0 {
                    p.current_pattern -= 1;
                }
                let current = p.patterns[p.current_pattern].name.clone();
                egui::ComboBox::from_id_salt("pattern").selected_text(current).width(96.0).show_ui(ui, |ui| {
                    for i in 0..p.patterns.len() {
                        let name = p.patterns[i].name.clone();
                        ui.selectable_value(&mut p.current_pattern, i, name);
                    }
                });
                if ui.small_button("▶").clicked() {
                    p.ensure_pattern(p.current_pattern + 1);
                    p.current_pattern += 1;
                }
                if ui.small_button("+").on_hover_text("New pattern").clicked() {
                    let n = p.patterns.len();
                    p.ensure_pattern(n);
                    p.current_pattern = n;
                }
            });
        });

        ui.label(egui::RichText::new("MAIN").color(TEXT_DIM).size(10.0));
        let mut mv = p.mixer[0].volume;
        let r = knob(ui, &mut mv, 0.0, engine::FADER_MAX, engine::FADER_DEFAULT, 26.0);
        if r.hovered() {
            self.hint = format!("Main volume {:.1} dB", 20.0 * (mv.max(1.0) / engine::VOL_SCALE).log10());
        }
        p.mixer[0].volume = mv;
        self.automate_menu(&r, p, Target::MixerVolume(0));
        let peak = p.mixer[0].peak;
        let (rect, _) = ui.allocate_exact_size(vec2(60.0, 22.0), Sense::hover());
        meter_h(ui.painter(), rect, peak);

        ui.add_space(10.0);
        for (flag, label, tip) in [
            (&mut self.show_playlist, "Playlist", "Playlist (F5)"),
            (&mut self.show_rack, "Channel rack", "Channel rack (F6)"),
            (&mut self.show_piano, "Piano roll", "Piano roll (F7)"),
            (&mut self.show_browser, "Browser", "Browser (F8)"),
            (&mut self.show_mixer, "Mixer", "Mixer (F9)"),
        ] {
            let on = *flag;
            if led_button(ui, label, on).on_hover_text(tip).clicked() {
                *flag = !on;
            }
        }
    }

    // ---------------------------------------------------------------- browser

    fn browser(&mut self, ctx: &egui::Context, p: &mut Project) {
        if !self.show_browser {
            return;
        }
        egui::SidePanel::left("browser")
            .resizable(true)
            .default_width(196.0)
            .frame(egui::Frame::none().fill(PANEL_DARK).inner_margin(6.0).stroke(Stroke::new(1.0_f32, LINE)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Browser").color(TEXT).strong());
                    ui.label(egui::RichText::new("- All").color(TEXT_DIM));
                });
                ui.separator();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    if folder(ui, "Current project", self.project_open).clicked() {
                        self.project_open = !self.project_open;
                    }
                    if self.project_open {
                        ui.indent("proj", |ui| {
                            ui.label(egui::RichText::new("Patterns").color(TEXT_DIM).size(11.0));
                            for i in 0..p.patterns.len() {
                                let name = p.patterns[i].name.clone();
                                if file_item(ui, &name, i == p.current_pattern).clicked() {
                                    p.current_pattern = i;
                                }
                            }
                            ui.label(egui::RichText::new("Generators").color(TEXT_DIM).size(11.0));
                            for i in 0..p.channels.len() {
                                let name = p.channels[i].name.clone();
                                if file_item(ui, &name, i == self.selected_channel).clicked() {
                                    self.selected_channel = i;
                                    p.previews.push((i, engine::STEP_KEY));
                                }
                            }
                            if !p.automations.is_empty() {
                                ui.label(egui::RichText::new("Automation").color(TEXT_DIM).size(11.0));
                                for a in &p.automations {
                                    file_item(ui, &a.target.label(p), false).on_hover_text("Shown at the top of the playlist");
                                }
                            }
                        });
                    }
                    if folder(ui, "Packs", self.packs_open).clicked() {
                        self.packs_open = !self.packs_open;
                    }
                    if self.packs_open {
                        ui.indent("packs", |ui| {
                            ui.label(egui::RichText::new("click to add a channel").color(TEXT_DIM).size(10.0));
                            for inst in Instrument::ALL {
                                if file_item(ui, &format!("{} sound", inst.label()), false).clicked() {
                                    p.add_channel(inst);
                                    self.selected_channel = p.channels.len() - 1;
                                    p.previews.push((self.selected_channel, engine::STEP_KEY));
                                }
                            }
                            if file_item(ui, "Sound file from disk...", false).clicked() {
                                self.pick(PickFor::NewSampler);
                            }
                        });
                    }
                    for (kind, name) in BROWSER_FOLDERS {
                        let open = self.browser_open.contains(name);
                        if folder(ui, name, open).clicked() && !self.browser_open.remove(name) {
                            self.browser_open.insert(name);
                        }
                        if open {
                            ui.indent(name, |ui| self.browser_folder(ui, p, kind));
                        }
                    }
                });
            });
    }

    fn browser_folder(&mut self, ui: &mut Ui, p: &mut Project, kind: Folder) {
        let dim = |ui: &mut Ui, t: &str| {
            ui.label(egui::RichText::new(t).color(TEXT_DIM).size(10.0));
        };
        let (dir, exts, empty): (PathBuf, Vec<&str>, &str) = match kind {
            Folder::Plugins => {
                if self.plugin_scan.is_some() {
                    dim(ui, "looking for plugins...");
                } else if self.plugins.is_empty() {
                    dim(ui, "no CLAP plugins found (~/.clap, /usr/lib/clap)");
                }
                if self.plugins.len() > 12 {
                    ui.add(egui::TextEdit::singleline(&mut self.plugin_search).hint_text("search plugins").desired_width(150.0));
                }
                let needle = self.plugin_search.to_lowercase();
                let list: Vec<PluginInfo> =
                    self.plugins.iter().filter(|i| needle.is_empty() || i.name.to_lowercase().contains(&needle) || i.vendor.to_lowercase().contains(&needle)).cloned().collect();
                for (title, inst) in [("Instruments - click to add a channel", true), ("Effects - click to add to the selected insert", false)] {
                    if list.iter().any(|i| i.instrument == inst) {
                        dim(ui, title);
                    }
                    for info in list.iter().filter(|i| i.instrument == inst) {
                        if file_item(ui, &info.name, false).on_hover_text(&info.vendor).clicked() {
                            if inst {
                                self.add_plugin_instrument(p, info, None);
                            } else {
                                let insert = self.selected_insert;
                                match Self::free_slot(p, insert) {
                                    Some(slot) => {
                                        self.add_plugin_effect(p, info, insert, slot);
                                        self.show_mixer = true;
                                    }
                                    None => self.hint = format!("{} has no free effect slot", p.mixer[insert].name),
                                }
                            }
                        }
                    }
                }
                if ui.small_button("Rescan").clicked() {
                    self.start_plugin_scan();
                }
                return;
            }
            Folder::Demos => {
                for name in file::DEMOS {
                    if file_item(ui, name, false).on_hover_text("Click to open as a new project").clicked() {
                        self.start_from(p, file::demo(name), name);
                    }
                }
                return;
            }
            Folder::Recent | Folder::Projects => (file::projects_dir(), vec![file::EXTENSION], "no saved projects"),
            Folder::Templates => (file::folder(file::TEMPLATES), vec![file::EXTENSION], "FILE > Save as template adds one"),
            Folder::Samples => (file::folder(file::SAMPLES), audiofile::AUDIO_EXTENSIONS.to_vec(), "put sound files here, or drop them on the window"),
            Folder::Recorded => (file::folder(file::RECORDED), vec!["wav"], "audio takes appear here"),
            Folder::Rendered => (file::folder(file::RENDERED), vec!["wav", "mp3"], "exported songs appear here"),
            Folder::Scores => (file::folder(file::SCORES), vec!["mid", "midi"], "MIDI files to import"),
        };
        let mut files = file::list_files(&dir, &exts);
        match kind {
            Folder::Projects => files.sort_by_key(|f| file::display_name(f).to_lowercase()),
            Folder::Recent => files.truncate(10),
            _ => {}
        }
        if files.is_empty() {
            dim(ui, empty);
        }
        for path in files {
            let sel = self.current_file.as_ref() == Some(&path);
            let tip = match kind {
                Folder::Templates => "Click to start a new project from it",
                Folder::Recent | Folder::Projects => "Click to open",
                Folder::Scores => "Click to import into a new pattern",
                _ => "Click to add as a sampler channel",
            };
            if file_item(ui, &file::display_name(&path), sel).on_hover_text(tip).clicked() {
                if kind == Folder::Templates {
                    match file::open(&path) {
                        Ok(new) => self.start_from(p, new, &file::display_name(&path)),
                        Err(e) => self.hint = e,
                    }
                } else {
                    self.open_any(p, path);
                }
            }
        }
    }

    // ---------------------------------------------------------------- playlist

    fn desktop(&mut self, ctx: &egui::Context, p: &mut Project) {
        egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
            let full = ui.max_rect();
            gradient(ui.painter(), full, BG_TOP, BG_BOTTOM);
            if self.show_playlist {
                let inner = full.shrink(6.0);
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
                egui::Frame::none().fill(PANEL).stroke(Stroke::new(1.0_f32, LINE)).rounding(3.0).show(&mut child, |ui| {
                    ui.set_min_size(inner.size());
                    self.playlist(ui, p);
                });
            }
        });
    }

    fn playlist(&mut self, ui: &mut Ui, p: &mut Project) {
        egui::Frame::none().fill(HEADER).inner_margin(egui::Margin::symmetric(8.0, 4.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Playlist").color(TEXT).strong());
                ui.label(egui::RichText::new("- Arrangement").color(TEXT_DIM));
                ui.label(egui::RichText::new(format!("›  {}", p.patterns[p.current_pattern].name)).color(ACCENT));
                ui.add_space(12.0);
                ui.menu_button("+ automation", |ui| {
                    if ui.button("Tempo").clicked() {
                        self.create_automation(p, Target::Tempo);
                        ui.close_menu();
                    }
                    if ui.button("Master volume").clicked() {
                        self.create_automation(p, Target::MixerVolume(0));
                        ui.close_menu();
                    }
                    for c in 0..p.channels.len() {
                        if ui.button(format!("{} volume", p.channels[c].name)).clicked() {
                            self.create_automation(p, Target::ChannelVolume(c));
                            ui.close_menu();
                        }
                    }
                    ui.label(egui::RichText::new("or right-click any knob").color(TEXT_DIM).size(10.0));
                });
            });
        });

        let avail = ui.available_rect_before_wrap();
        let picker_w = 110.0;
        let picker = Rect::from_min_size(avail.min, vec2(picker_w, avail.height()));
        ui.painter().rect_filled(picker, 0.0, PANEL_DARK);
        let mut y = picker.top() + 6.0;
        for i in 0..p.patterns.len() {
            let r = Rect::from_min_size(pos2(picker.left() + 6.0, y), vec2(picker_w - 12.0, 18.0));
            let resp = ui.interact(r, ui.id().with(("pick", i)), Sense::click());
            let sel = i == p.current_pattern;
            ui.painter().rect_filled(r, 2.0, if sel { pattern_color(i) } else { PANEL });
            let col = if sel { PANEL_DEEP } else { TEXT };
            ui.painter().text(r.left_center() + vec2(6.0, 0.0), Align2::LEFT_CENTER, &p.patterns[i].name, FontId::proportional(11.0), col);
            if resp.clicked() {
                p.current_pattern = i;
            }
            y += 20.0;
        }

        let area = Rect::from_min_max(pos2(picker.right(), avail.top()), avail.max);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
        egui::ScrollArea::both().auto_shrink([false, false]).show(&mut child, |ui| self.arrangement(ui, p));
    }

    fn arrangement(&mut self, ui: &mut Ui, p: &mut Project) {
        let bar_w = 44.0;
        let row_h = 30.0;
        let head_w = 86.0;
        let ruler_h = 20.0;
        let bars = (p.song_length_ticks().div_ceil(TICKS_PER_BAR) + 16).max(64);
        let tracks_h = PLAYLIST_TRACKS as f32 * row_h;
        let autos_h = p.automations.len() as f32 * AUTO_LANE_H;
        let total = vec2(head_w + bars as f32 * bar_w, ruler_h + tracks_h + autos_h);
        let (rect, _) = ui.allocate_exact_size(total, Sense::hover());
        let painter = ui.painter_at(ui.clip_rect());
        // Automation lanes sit between the ruler and the tracks.
        let autos_top = rect.top() + ruler_h;
        let grid = Rect::from_min_max(pos2(rect.left() + head_w, autos_top + autos_h), pos2(rect.right(), autos_top + autos_h + tracks_h));
        let tick_x = |t: f32| grid.left() + t / TICKS_PER_BAR as f32 * bar_w;

        for t in 0..PLAYLIST_TRACKS {
            let y0 = grid.top() + t as f32 * row_h;
            let lane = Rect::from_min_max(pos2(grid.left(), y0), pos2(grid.right(), y0 + row_h));
            painter.rect_filled(lane, 0.0, if t % 2 == 0 { PANEL } else { PANEL_ALT });
        }
        for b in 0..=bars {
            let x = grid.left() + b as f32 * bar_w;
            painter.line_segment([pos2(x, grid.top()), pos2(x, grid.bottom())], Stroke::new(1.0_f32, LINE));
            if b < bars {
                for q in 1..BEATS_PER_BAR {
                    let xq = x + q as f32 * bar_w / BEATS_PER_BAR as f32;
                    painter.line_segment([pos2(xq, grid.top()), pos2(xq, grid.bottom())], Stroke::new(1.0_f32, LINE_FAINT));
                }
            }
        }
        for t in 0..=PLAYLIST_TRACKS {
            let y = grid.top() + t as f32 * row_h;
            painter.line_segment([pos2(grid.left(), y), pos2(grid.right(), y)], Stroke::new(1.0_f32, LINE));
        }

        // Ruler: click to move the song position.
        let ruler = Rect::from_min_max(pos2(grid.left(), rect.top()), pos2(grid.right(), autos_top));
        painter.rect_filled(ruler, 0.0, PANEL_DARK);
        for b in 0..bars {
            let x = grid.left() + b as f32 * bar_w;
            painter.text(pos2(x + 3.0, ruler.center().y), Align2::LEFT_CENTER, format!("{}", b + 1), FontId::proportional(10.0), TEXT_DIM);
        }
        let rr = ui.interact(ruler, ui.id().with("ruler"), Sense::click());
        if rr.secondary_clicked() {
            if let Some(pp) = rr.interact_pointer_pos() {
                self.ruler_bar = ((pp.x - grid.left()) / bar_w).floor().max(0.0) as u32;
            }
        }
        let at = self.ruler_bar;
        rr.context_menu(|ui| {
            ui.label(egui::RichText::new(format!("Bar {}", at + 1)).color(TEXT_DIM).size(10.0));
            if ui.button("Insert a bar here").on_hover_text("Clips and automation from here move one bar later").clicked() {
                arrange::insert_bars(&mut p.clips, at, 1);
                for a in &mut p.automations {
                    arrange::insert_ticks(&mut a.points, at * TICKS_PER_BAR, TICKS_PER_BAR);
                }
                ui.close_menu();
            }
            if ui.button("Delete this bar").on_hover_text("Removes the clips starting in it; what follows moves one bar earlier").clicked() {
                arrange::delete_bars(&mut p.clips, at, 1);
                for a in &mut p.automations {
                    arrange::delete_ticks(&mut a.points, at * TICKS_PER_BAR, TICKS_PER_BAR);
                }
                ui.close_menu();
            }
        });
        if rr.clicked() {
            if let Some(pp) = rr.interact_pointer_pos() {
                let bar = ((pp.x - grid.left()) / bar_w).floor().max(0.0) as u32;
                p.mode = Mode::Song;
                p.position = (bar * TICKS_PER_BAR) as f64;
            }
        }

        // Clips.
        let mut hit: Option<usize> = None;
        let pointer = ui.input(|i| i.pointer.hover_pos());
        for (ci, c) in p.clips.iter().enumerate() {
            let pat = &p.patterns[c.pattern];
            let plen = pat.length_ticks() as f32;
            let r = Rect::from_min_size(
                pos2(grid.left() + c.start_bar as f32 * bar_w, grid.top() + c.track as f32 * row_h),
                vec2(plen / TICKS_PER_BAR as f32 * bar_w, row_h),
            )
            .shrink(1.0);
            let col = pattern_color(c.pattern);
            painter.rect_filled(r, 2.0, col.gamma_multiply(0.55));
            painter.rect_filled(Rect::from_min_size(r.min, vec2(r.width(), 11.0)), 2.0, col);
            painter.text(r.min + vec2(3.0, 0.0), Align2::LEFT_TOP, &pat.name, FontId::proportional(9.5), PANEL_DEEP);
            let nch = pat.notes.len().max(1) as f32;
            for (ch, list) in pat.notes.iter().enumerate() {
                for n in list.iter().filter(|n| n.vel > 0) {
                    let x = r.left() + n.start as f32 / plen * r.width();
                    let yy = r.top() + 12.0 + ch as f32 / nch * (r.height() - 13.0);
                    let w = (n.len as f32 / plen * r.width()).max(1.5);
                    painter.rect_filled(Rect::from_min_size(pos2(x, yy), vec2(w, 2.0)), 0.0, Color32::from_gray(235));
                }
            }
            if pointer.is_some_and(|pp| r.contains(pp)) {
                hit = Some(ci);
            }
        }

        // Automation lanes.
        let vis = ui.clip_rect();
        let mut remove_auto: Option<usize> = None;
        for ai in 0..p.automations.len() {
            let y0 = autos_top + ai as f32 * AUTO_LANE_H;
            let lane = Rect::from_min_max(pos2(grid.left(), y0), pos2(grid.right(), y0 + AUTO_LANE_H));
            painter.rect_filled(lane, 0.0, PANEL_DEEP);
            painter.line_segment([lane.left_bottom(), lane.right_bottom()], Stroke::new(1.0_f32, LINE));
            for b in (0..=bars).step_by(4) {
                let x = grid.left() + b as f32 * bar_w;
                painter.line_segment([pos2(x, lane.top()), pos2(x, lane.bottom())], Stroke::new(1.0_f32, LINE_FAINT));
            }
            let target = p.automations[ai].target;
            let (lo, hi) = target.range();
            let val_y = |v: f32| lane.bottom() - 4.0 - (v - lo) / (hi - lo) * (AUTO_LANE_H - 8.0);
            let y_val = |y: f32| (lo + (lane.bottom() - 4.0 - y) / (AUTO_LANE_H - 8.0) * (hi - lo)).clamp(lo, hi);
            let col = Color32::from_rgb(0xA0, 0xC8, 0xF0);
            let pts: Vec<Pos2> = p.automations[ai].points.iter().map(|&(t, v)| pos2(tick_x(t as f32), val_y(v))).collect();
            if let (Some(first), Some(last)) = (pts.first(), pts.last()) {
                let mut line = vec![pos2(lane.left(), first.y)];
                line.extend(pts.iter().copied());
                line.push(pos2(lane.right(), last.y));
                painter.add(egui::Shape::line(line, Stroke::new(1.5_f32, col)));
            }
            let lane_hit = Rect::from_min_max(pos2(vis.left() + head_w, lane.top()), lane.max);
            let lr = ui.interact(lane_hit, ui.id().with(("auto", ai)), Sense::click_and_drag());
            let near = lr.hover_pos().or(lr.interact_pointer_pos()).and_then(|hp| pts.iter().position(|q| q.distance(hp) < 7.0));
            // Handles only where they have room: a drawn curve has a point per step.
            let mut last_dot: Option<Pos2> = None;
            for (k, q) in pts.iter().enumerate() {
                let lit = near == Some(k) || self.auto_drag == Some((ai, k));
                if lit || last_dot.is_none_or(|d| d.distance(*q) >= 8.0) {
                    painter.circle_filled(*q, if lit { 5.0 } else { 3.5 }, if lit { ACCENT } else { col });
                    last_dot = Some(*q);
                }
            }
            let snap = |x: f32| {
                let t = ((x - grid.left()) / bar_w * TICKS_PER_BAR as f32).max(0.0);
                ((t / TICKS_PER_STEP as f32).round() as u32) * TICKS_PER_STEP
            };
            let shift = ui.input(|i| i.modifiers.shift);
            if lr.drag_started_by(egui::PointerButton::Primary) && shift {
                if let Some(pp) = lr.interact_pointer_pos() {
                    let (t, v) = (snap(pp.x), y_val(pp.y));
                    p.automations[ai].draw(t, v, t, v, TICKS_PER_STEP);
                    self.auto_draw = Some((ai, t, v));
                }
            } else if lr.drag_started_by(egui::PointerButton::Primary) {
                if let Some(k) = near {
                    self.auto_drag = Some((ai, k));
                } else if let Some(pp) = lr.interact_pointer_pos() {
                    let t = snap(pp.x);
                    p.automations[ai].points.push((t, y_val(pp.y)));
                    p.automations[ai].sort();
                    self.auto_drag = p.automations[ai].points.iter().position(|q| q.0 == t).map(|k| (ai, k));
                }
            }
            if let (Some((da, t0, v0)), Some(pp)) = (self.auto_draw, lr.interact_pointer_pos()) {
                if da == ai && lr.dragged() {
                    let (t, v) = (snap(pp.x), y_val(pp.y));
                    p.automations[ai].draw(t0, v0, t, v, TICKS_PER_STEP);
                    self.auto_draw = Some((ai, t, v));
                }
            }
            if let (Some((da, dk)), Some(pp)) = (self.auto_drag, lr.interact_pointer_pos()) {
                if da == ai && lr.dragged() && dk < p.automations[ai].points.len() {
                    let pts = &p.automations[ai].points;
                    // A point stays between its neighbours.
                    let min = if dk > 0 { pts[dk - 1].0 } else { 0 };
                    let max = pts.get(dk + 1).map_or(u32::MAX, |q| q.0);
                    p.automations[ai].points[dk] = (snap(pp.x).clamp(min, max), y_val(pp.y));
                }
            }
            if lr.drag_stopped() {
                self.auto_drag = None;
                self.auto_draw = None;
            }
            if lr.clicked() && near.is_none() {
                if let Some(pp) = lr.interact_pointer_pos() {
                    p.automations[ai].points.push((snap(pp.x), y_val(pp.y)));
                    p.automations[ai].sort();
                }
            }
            if lr.secondary_clicked() {
                if let Some(k) = near {
                    p.automations[ai].points.remove(k);
                }
            }
            if lr.hovered() {
                self.hint = match near {
                    Some(k) => {
                        let v = p.automations[ai].points[k].1;
                        format!("{}: {v:.2} - drag to move, right-click to delete", target.label(p))
                    }
                    None => format!("{}: click to add a point, Shift+drag to draw", target.label(p)),
                };
            }
            // Header pinned on the left like the track names.
            let head = Rect::from_min_max(pos2(vis.left(), y0), pos2(vis.left() + head_w, y0 + AUTO_LANE_H));
            painter.rect_filled(head, 0.0, HEADER);
            painter.line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0_f32, LINE));
            painter.text(head.left_top() + vec2(6.0, 6.0), Align2::LEFT_TOP, "Automation", FontId::proportional(9.0), TEXT_DIM);
            let label = target.label(p);
            let galley = painter.layout(label, FontId::proportional(10.0), col, head_w - 10.0);
            painter.galley(head.left_top() + vec2(6.0, 18.0), galley, col);
            let hr = ui.interact(head, ui.id().with(("auto_head", ai)), Sense::click());
            hr.on_hover_text("Right-click to delete this automation clip").context_menu(|ui| {
                if ui.button("Delete automation clip").clicked() {
                    remove_auto = Some(ai);
                    ui.close_menu();
                }
            });
        }
        if let Some(ai) = remove_auto {
            p.automations.remove(ai);
            self.auto_drag = None;
            self.auto_draw = None;
        }

        // Track headers stay pinned on the left while scrolling.
        let heads = Rect::from_min_max(pos2(vis.left(), grid.top()), pos2(vis.left() + head_w, grid.bottom()));
        painter.rect_filled(heads, 0.0, HEADER);
        for t in 0..PLAYLIST_TRACKS {
            let y0 = grid.top() + t as f32 * row_h;
            painter.line_segment([pos2(heads.left(), y0), pos2(heads.right(), y0)], Stroke::new(1.0_f32, LINE));
            painter.circle_filled(pos2(heads.left() + 10.0, y0 + row_h / 2.0), 3.5, LED_ON);
            painter.text(pos2(heads.left() + 20.0, y0 + row_h / 2.0), Align2::LEFT_CENTER, format!("Track {}", t + 1), FontId::proportional(11.0), TEXT);
        }
        painter.rect_filled(Rect::from_min_max(pos2(vis.left(), rect.top()), pos2(vis.left() + head_w, autos_top)), 0.0, PANEL_DARK);

        let gr = ui.interact(Rect::from_min_max(pos2(vis.left() + head_w, grid.top()), grid.max), ui.id().with("grid"), Sense::click_and_drag());
        let cell = |pp: Pos2| {
            let bar = ((pp.x - grid.left()) / bar_w).floor().max(0.0) as i64;
            let track = ((pp.y - grid.top()) / row_h).floor().clamp(0.0, (PLAYLIST_TRACKS - 1) as f32) as usize;
            (bar, track)
        };
        if gr.secondary_clicked() {
            if let Some(ci) = hit.take() {
                p.clips.remove(ci);
            }
        }
        if gr.drag_started_by(egui::PointerButton::Primary) {
            if let (Some(ci), Some(pp)) = (hit, gr.interact_pointer_pos()) {
                let (bar, _) = cell(pp);
                // Ctrl or Shift: drag a copy, the original stays.
                let copy = ui.input(|i| i.modifiers.ctrl || i.modifiers.shift);
                let ci = if copy {
                    let c = &p.clips[ci];
                    p.clips.push(Clip { track: c.track, start_bar: c.start_bar, pattern: c.pattern });
                    p.clips.len() - 1
                } else {
                    ci
                };
                self.clip_drag = Some(ClipDrag { index: ci, grab_bars: bar - p.clips[ci].start_bar as i64 });
                p.current_pattern = p.clips[ci].pattern;
            }
        }
        if let (Some(d), Some(pp)) = (self.clip_drag, gr.interact_pointer_pos()) {
            if gr.dragged() && d.index < p.clips.len() {
                let (bar, track) = cell(pp);
                p.clips[d.index].start_bar = (bar - d.grab_bars).max(0) as u32;
                p.clips[d.index].track = track;
            }
        }
        if gr.drag_stopped() {
            self.clip_drag = None;
        }
        if gr.clicked() {
            self.piano_focus = false;
            if let Some(pp) = gr.interact_pointer_pos() {
                match hit {
                    Some(ci) => p.current_pattern = p.clips[ci].pattern,
                    None => {
                        let (bar, track) = cell(pp);
                        p.clips.push(Clip { track, start_bar: bar as u32, pattern: p.current_pattern });
                    }
                }
            }
        }
        if gr.hovered() {
            self.hint = "Playlist: click to place the selected pattern, drag to move (Ctrl+drag: copy), right-click to delete".into();
        }

        // In pattern mode the play head shows where the pattern sits in the song.
        if let Some(tick) = p.automation_tick() {
            let x = grid.left() + tick as f32 / TICKS_PER_BAR as f32 * bar_w;
            painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.5_f32, ACCENT));
            let tri = vec![pos2(x - 5.0, rect.top()), pos2(x + 5.0, rect.top()), pos2(x, rect.top() + 7.0)];
            painter.add(egui::Shape::convex_polygon(tri, ACCENT, Stroke::NONE));
        }
    }

    fn about(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let mut open = true;
        egui::Window::new("About").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label("Beat Studio - a small pattern-based music maker.");
            ui.label(format!("{} ticks per beat, {STEPS_PER_BAR} steps per bar.", engine::PPQ));
            ui.label("Space: play / stop   L: pattern / song   F5-F9: windows");
            ui.label("Ctrl+Z / Ctrl+Y: undo / redo   Ctrl+S: save   Ctrl+R: export");
            ui.label("Drop sound, MIDI or project files on the window to open them.");
        });
        self.show_about = open;
    }
}

impl eframe::App for StudioApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if self.script.is_empty() {
            return;
        }
        let ppp = ctx.pixels_per_point();
        for (_, e) in self.script.iter().filter(|e| e.0 == self.frames) {
            let at = |x: f32, y: f32| pos2(x / ppp, y / ppp);
            raw.events.push(match *e {
                ScriptEvent::Move(x, y) => egui::Event::PointerMoved(at(x, y)),
                ScriptEvent::Key(key, pressed, modifiers) => {
                    raw.modifiers = modifiers;
                    egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }
                }
                ScriptEvent::Button(x, y, button, pressed, modifiers) => {
                    // Held from the press to the release, like a real key.
                    self.script_mods = if pressed { modifiers } else { egui::Modifiers::NONE };
                    raw.modifiers = modifiers;
                    egui::Event::PointerButton { pos: at(x, y), button, pressed, modifiers }
                }
            });
        }
        if self.script_mods != egui::Modifiers::NONE {
            raw.modifiers = self.script_mods;
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let shared = self.shared.clone();
        let mut guard = match shared.lock() {
            Ok(g) => g,
            Err(poison) => poison.into_inner(),
        };
        let p: &mut Project = &mut guard;
        self.undo_keys(ctx, p);
        self.shortcuts(ctx, p);
        self.midi_keys(p);
        self.dropped_files(ctx, p);
        self.top_bar(ctx, p);
        self.browser(ctx, p);
        self.desktop(ctx, p);
        self.channel_rack(ctx, p);
        self.piano_roll(ctx, p);
        self.mixer(ctx, p);
        self.plugin_params_window(ctx, p);
        self.about(ctx);
        self.file_window(ctx, p);
        self.picker_window(ctx, p);
        self.info_window(ctx, p);
        self.audio_window(ctx, p);
        self.general_window(ctx, p);
        self.midi_window(ctx);
        self.maintain_samples(p);
        self.maintain_plugins(p);
        self.maintain_recording(p);
        self.track_undo(ctx, p);
        self.track_changes(ctx, p);
        self.settings.metronome = p.metronome;
        self.settings.typing_piano = self.typing_piano;
        drop(guard);
        self.apply_settings();
        self.midi_connect(ctx.input(|i| i.time));
        self.handle_screenshot(ctx);
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

#[derive(Clone, Copy)]
enum ScriptEvent {
    Move(f32, f32),
    Key(egui::Key, bool, egui::Modifiers),
    Button(f32, f32, egui::PointerButton, bool, egui::Modifiers),
}

/// Reads BEATSTUDIO_INPUT: actions separated by ";", in screenshot pixels:
/// `click x y [ctrl|shift]`, `rclick x y`, `move x y`, `drag x1 y1 x2 y2 [ctrl|shift]`,
/// `key ctrl+a` (egui key names: `Delete`, `ArrowUp`...), `wait frames`. Each action starts a few frames after the previous one.
fn parse_script(text: &str) -> Vec<(u32, ScriptEvent)> {
    let mut out = Vec::new();
    let mut f = 20u32;
    for step in text.split(';') {
        let w: Vec<&str> = step.split_whitespace().collect();
        let num = |i: usize| w.get(i).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
        let mods = match w.last().copied() {
            Some("ctrl") => egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
            Some("shift") => egui::Modifiers::SHIFT,
            _ => egui::Modifiers::NONE,
        };
        let press = |x, y, b, down, m| ScriptEvent::Button(x, y, b, down, m);
        match w.first().copied() {
            Some("click") | Some("rclick") => {
                let b = if w[0] == "rclick" { egui::PointerButton::Secondary } else { egui::PointerButton::Primary };
                let (x, y) = (num(1), num(2));
                out.push((f, ScriptEvent::Move(x, y)));
                out.push((f + 2, press(x, y, b, true, mods)));
                out.push((f + 4, press(x, y, b, false, mods)));
                f += 12;
            }
            Some("drag") => {
                let (x1, y1, x2, y2) = (num(1), num(2), num(3), num(4));
                out.push((f, ScriptEvent::Move(x1, y1)));
                out.push((f + 2, press(x1, y1, egui::PointerButton::Primary, true, mods)));
                for k in 1..=8 {
                    let t = k as f32 / 8.0;
                    out.push((f + 2 + k * 2, ScriptEvent::Move(x1 + (x2 - x1) * t, y1 + (y2 - y1) * t)));
                }
                out.push((f + 22, press(x2, y2, egui::PointerButton::Primary, false, mods)));
                f += 32;
            }
            Some("key") => {
                // key ctrl+shift+z, key delete, key arrowup...
                let combo = w.get(1).copied().unwrap_or("");
                let mut m = egui::Modifiers::NONE;
                let mut key = None;
                for part in combo.split('+') {
                    match part {
                        "ctrl" => m = m | egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
                        "shift" => m = m | egui::Modifiers::SHIFT,
                        "alt" => m = m | egui::Modifiers::ALT,
                        name => key = egui::Key::from_name(&name[..1].to_uppercase()).filter(|_| name.len() == 1).or_else(|| egui::Key::from_name(name)),
                    }
                }
                if let Some(k) = key {
                    out.push((f, ScriptEvent::Key(k, true, m)));
                    out.push((f + 2, ScriptEvent::Key(k, false, m)));
                }
                f += 8;
            }
            Some("move") => {
                out.push((f, ScriptEvent::Move(num(1), num(2))));
                f += 10;
            }
            Some("wait") => f += num(1) as u32,
            _ => {}
        }
    }
    out
}
