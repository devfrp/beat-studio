// The work behind the windows: undo, decoding sound files, plugins (scan,
// loading, parameter window), the disk file picker, dropped files and audio
// recording. Another `impl StudioApp`, like editors.rs.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, vec2, Key};

use crate::audiofile;
use crate::effects::Effect;
use crate::engine::{Channel, Clip, Instrument, Mode, Note, Project, SampleData, SampleRef, NOTE_VELOCITY, PLAYLIST_TRACKS, STEP_KEY, TICKS_PER_BAR, TICKS_PER_STEP};
use crate::file;
use crate::plugin::{self, PluginCore, PluginEffect, PluginInfo, PluginInstrument, PluginSlot};
use crate::settings;
use crate::smf;
use crate::ui::{PickFor, PluginTarget, StudioApp, TakeStart};
use crate::widgets::*;

const UNDO_STEPS: usize = 200;

impl StudioApp {
    // ---------------------------------------------------------------- undo

    pub fn reset_history(&mut self, p: &Project) {
        self.undo.clear();
        self.redo.clear();
        self.undo_last = file::to_text(p);
    }

    /// Records an undo step once a gesture is over: no mouse button held and
    /// no text being typed. Checks after input, and every second for changes
    /// made by recording.
    pub fn track_undo(&mut self, ctx: &egui::Context, p: &Project) {
        let (busy, input, now) = ctx.input(|i| (i.pointer.any_down(), !i.events.is_empty(), i.time));
        if busy || ctx.wants_keyboard_input() || (!input && now < self.next_undo_check) {
            return;
        }
        self.next_undo_check = now + 1.0;
        let text = file::to_text(p);
        if text != self.undo_last {
            self.undo.push(std::mem::replace(&mut self.undo_last, text));
            if self.undo.len() > UNDO_STEPS {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    /// Undo (`back`) or redo one step.
    pub fn undo_step(&mut self, p: &mut Project, back: bool) {
        let popped = if back { self.undo.pop() } else { self.redo.pop() };
        let Some(text) = popped else {
            self.hint = if back { "Nothing to undo".into() } else { "Nothing to redo".into() };
            return;
        };
        let current = file::to_text(p);
        if back {
            self.redo.push(current);
        } else {
            self.undo.push(current);
        }
        match file::from_text(&text) {
            Ok(mut new) => {
                new.keep_runtime(p);
                *p = new;
                self.hint = if back { "Undone".into() } else { "Redone".into() };
            }
            Err(e) => self.hint = format!("Could not undo: {e}"),
        }
        self.undo_last = text;
        self.after_project_change(p);
    }

    /// Keeps selections and open windows valid after the project was replaced.
    pub fn after_project_change(&mut self, p: &Project) {
        self.selected_channel = self.selected_channel.min(p.channels.len() - 1);
        self.piano_drag = None;
        self.note_sel.clear();
        self.auto_drag = None;
        self.plugin_window = match self.plugin_window {
            Some(PluginTarget::Channel(c)) if p.channels.get(c).is_some_and(|c| c.plugin.is_some()) => self.plugin_window,
            Some(PluginTarget::Effect(i, s)) if p.mixer[i].fx[s].as_ref().is_some_and(|e| e.plugin.is_some()) => self.plugin_window,
            _ => None,
        };
        if self.selected_slot.is_some_and(|s| p.mixer[self.selected_insert].fx[s].is_none()) {
            self.selected_slot = None;
        }
    }

    // ---------------------------------------------------------------- sound files

    fn decode_in_background(&mut self, path: PathBuf) {
        self.loading.insert(path.clone());
        let tx = self.sample_tx.clone();
        std::thread::spawn(move || {
            let result = audiofile::decode(&path);
            let _ = tx.send((path, result));
        });
    }

    /// Gives decoded sounds to the channels that wait for them, and starts
    /// decoding the files not read yet (in the background: the audio thread
    /// must not wait for the disk).
    pub fn maintain_samples(&mut self, p: &mut Project) {
        while let Ok((path, result)) = self.sample_rx.try_recv() {
            self.loading.remove(&path);
            match result {
                Ok(data) => {
                    self.samples.insert(path, Arc::new(data));
                }
                Err(e) => {
                    self.hint = format!("Could not read {}: {e}", file::display_name(&path));
                    self.failed.insert(path);
                }
            }
        }
        let mut wanted = Vec::new();
        for c in &mut p.channels {
            let Some(s) = &mut c.sample else { continue };
            if s.data.is_some() {
                continue;
            }
            match self.samples.get(&s.path) {
                Some(d) => s.data = Some(d.clone()),
                None if !self.loading.contains(&s.path) && !self.failed.contains(&s.path) => wanted.push(s.path.clone()),
                None => {}
            }
        }
        for path in wanted {
            if !self.loading.contains(&path) {
                self.decode_in_background(path);
            }
        }
    }

    pub fn add_sampler(&mut self, p: &mut Project, path: PathBuf) {
        self.failed.remove(&path);
        let data = self.samples.get(&path).cloned();
        let c = file::sampler_channel(p, &path, data);
        self.hint = format!("Added {} - click its steps to play it", c.name);
        p.push_channel(c);
        self.selected_channel = p.channels.len() - 1;
        self.show_rack = true;
    }

    pub fn replace_sample(&mut self, p: &mut Project, ch: usize, path: PathBuf) {
        self.failed.remove(&path);
        let data = self.samples.get(&path).cloned();
        let name = p.channel_name(&file::display_name(&path));
        let Some(c) = p.channels.get_mut(ch) else { return };
        c.instrument = Instrument::Sampler;
        c.plugin = None;
        c.name = name;
        c.sample = Some(SampleRef { path, data });
        if self.plugin_window == Some(PluginTarget::Channel(ch)) {
            self.plugin_window = None;
        }
    }

    pub fn import_midi(&mut self, p: &mut Project, path: &Path) {
        let result = std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| smf::parse(&b));
        match result {
            Ok(imp) => {
                let pat = smf::import_into(p, &imp, &file::display_name(path));
                self.hint = format!("Imported {} into {}", file::display_name(path), p.patterns[pat].name);
                self.show_rack = true;
            }
            Err(e) => self.hint = format!("Could not import: {e}"),
        }
    }

    /// Opens whatever kind of file it is: a sound, a MIDI file or a project.
    pub fn open_any(&mut self, p: &mut Project, path: PathBuf) {
        let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).unwrap_or_default();
        if audiofile::is_audio(&path) {
            self.add_sampler(p, path);
        } else if ext == "mid" || ext == "midi" {
            self.import_midi(p, &path);
        } else if ext == file::EXTENSION {
            if let Err(e) = self.open_file(p, path) {
                self.hint = e;
            }
        } else {
            self.hint = format!("Cannot open {}: not a sound, MIDI or project file", file::display_name(&path));
        }
    }

    /// Files dropped on the window from the file manager.
    pub fn dropped_files(&mut self, ctx: &egui::Context, p: &mut Project) {
        let files: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        for f in files {
            self.open_any(p, f);
        }
    }

    // ---------------------------------------------------------------- file picker

    pub fn pick(&mut self, purpose: PickFor) {
        let start = match purpose {
            PickFor::ImportMidi => file::folder(file::SCORES),
            _ => file::folder(file::SAMPLES),
        };
        let dir = if start.is_dir() { start } else { std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/")) };
        self.picker = Some(crate::ui::FilePicker { purpose, dir });
    }

    pub fn picker_window(&mut self, ctx: &egui::Context, p: &mut Project) {
        let Some(mut pk) = self.picker.take() else { return };
        let (title, exts): (&str, &[&str]) = match pk.purpose {
            PickFor::ImportMidi => ("Import a MIDI file", &["mid", "midi"]),
            _ => ("Choose a sound file", &audiofile::AUDIO_EXTENSIONS),
        };
        let mut open = true;
        let mut chosen: Option<PathBuf> = None;
        egui::Window::new(title).open(&mut open).collapsible(false).default_size(vec2(420.0, 420.0)).show(ctx, |ui| {
            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
            ui.horizontal_wrapped(|ui| {
                for (label, dir) in [
                    ("Home", home.clone()),
                    ("Music", home.join("Music")),
                    ("Downloads", home.join("Downloads")),
                    ("Samples", file::folder(file::SAMPLES)),
                    ("Recorded", file::folder(file::RECORDED)),
                    ("Scores", file::folder(file::SCORES)),
                ] {
                    if dir.is_dir() && ui.small_button(label).clicked() {
                        pk.dir = dir;
                    }
                }
            });
            ui.label(egui::RichText::new(pk.dir.display().to_string()).color(TEXT_DIM).size(11.0));
            ui.separator();
            let mut dirs = Vec::new();
            let mut files = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&pk.dir) {
                for e in rd.flatten() {
                    let path = e.path();
                    let hidden = path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
                    if hidden {
                        continue;
                    }
                    if path.is_dir() {
                        dirs.push(path);
                    } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| exts.contains(&e.to_lowercase().as_str())) {
                        files.push(path);
                    }
                }
            }
            dirs.sort();
            files.sort();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                if let Some(parent) = pk.dir.parent() {
                    if folder(ui, "..", false).clicked() {
                        pk.dir = parent.to_path_buf();
                    }
                }
                for d in dirs {
                    let name = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if folder(ui, &name, false).clicked() {
                        pk.dir = d;
                    }
                }
                for f in files {
                    let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if file_item(ui, &name, false).on_hover_text("Click to use this file").clicked() {
                        chosen = Some(f);
                    }
                }
            });
            ui.label(egui::RichText::new("You can also drop files on the window from your file manager.").color(TEXT_DIM).size(10.0));
        });
        if let Some(path) = chosen {
            match pk.purpose {
                PickFor::NewSampler => self.add_sampler(p, path),
                PickFor::ReplaceSample(ch) => self.replace_sample(p, ch, path),
                PickFor::ImportMidi => self.import_midi(p, &path),
            }
        } else if open {
            self.picker = Some(pk);
        }
    }

    // ---------------------------------------------------------------- plugins

    fn plugin_cache_path() -> PathBuf {
        settings::config_dir().join("plugins.txt")
    }

    /// The plugin list from the last scan; scans in the background the first time.
    pub fn load_plugin_list(&mut self) {
        match std::fs::read_to_string(Self::plugin_cache_path()) {
            Ok(text) => self.plugins = plugin::cache_from_text(&text),
            Err(_) => self.start_plugin_scan(),
        }
    }

    pub fn start_plugin_scan(&mut self) {
        if self.plugin_scan.is_some() || self.screenshot.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(plugin::scan_all());
        });
        self.plugin_scan = Some(rx);
        self.hint = "Looking for CLAP plugins...".into();
    }

    /// Scan results, plugins waiting to load, and the plugins' main-thread requests.
    pub fn maintain_plugins(&mut self, p: &mut Project) {
        if let Some(rx) = &self.plugin_scan {
            if let Ok(list) = rx.try_recv() {
                self.hint = format!("Found {} CLAP plugins", list.len());
                let _ = std::fs::create_dir_all(settings::config_dir());
                let _ = std::fs::write(Self::plugin_cache_path(), plugin::cache_to_text(&list));
                self.plugins = list;
                self.plugin_scan = None;
            }
        }
        let errors = file::start_plugins(p);
        if let Some(e) = errors.first() {
            self.hint = format!("Plugin not loaded - {e}");
        }
        let rate = p.sample_rate;
        let (inst, fx) = p.plugin_slots();
        for s in inst {
            if let Some(l) = &mut s.live {
                l.core.idle(rate);
            }
        }
        for s in fx {
            if let Some(l) = &mut s.live {
                l.core.idle(rate);
            }
        }
    }

    /// Puts an instrument plugin on a new channel, or on channel `replace`.
    pub fn add_plugin_instrument(&mut self, p: &mut Project, info: &PluginInfo, replace: Option<usize>) {
        let core = match PluginCore::load(info, p.sample_rate, None) {
            Ok(c) => c,
            Err(e) => {
                self.hint = format!("Could not load {}: {e}", info.name);
                return;
            }
        };
        let mut slot = PluginSlot::new(info.clone());
        slot.live = Some(PluginInstrument::new(core));
        let name = p.channel_name(&info.name);
        let ch = match replace.filter(|&c| c < p.channels.len()) {
            Some(ch) => {
                let c = &mut p.channels[ch];
                c.instrument = Instrument::Plugin;
                c.sample = None;
                c.name = name;
                c.plugin = Some(Box::new(slot));
                ch
            }
            None => {
                let mut c = Channel::new(name, Instrument::Plugin, 0);
                c.plugin = Some(Box::new(slot));
                p.push_channel(c);
                p.channels.len() - 1
            }
        };
        self.selected_channel = ch;
        self.show_rack = true;
        self.hint = format!("{} is ready - play it from the piano roll or the keyboard", info.name);
    }

    pub fn add_plugin_effect(&mut self, p: &mut Project, info: &PluginInfo, insert: usize, slot: usize) {
        match PluginCore::load(info, p.sample_rate, None) {
            Ok(core) => {
                let mut s = PluginSlot::new(info.clone());
                s.live = Some(PluginEffect::new(core));
                p.mixer[insert].fx[slot] = Some(Effect::plugin(s));
                self.selected_slot = Some(slot);
                self.hint = format!("{} added to {}", info.name, p.mixer[insert].name);
            }
            Err(e) => self.hint = format!("Could not load {}: {e}", info.name),
        }
    }

    /// The first free effect slot of an insert.
    pub fn free_slot(p: &Project, insert: usize) -> Option<usize> {
        p.mixer[insert].fx.iter().position(|e| e.is_none())
    }

    /// Generic sliders for a plugin's parameters. The plugin's own window is
    /// not shown (it would need the window system's embedding).
    pub fn plugin_params_window(&mut self, ctx: &egui::Context, p: &mut Project) {
        let Some(target) = self.plugin_window else { return };
        let (core, error, name) = match target {
            PluginTarget::Channel(ch) => match p.channels.get_mut(ch).and_then(|c| c.plugin.as_mut()) {
                Some(s) => (s.live.as_mut().map(|l| &mut l.core), s.error.clone(), s.info.name.clone()),
                None => {
                    self.plugin_window = None;
                    return;
                }
            },
            PluginTarget::Effect(i, sl) => match p.mixer[i].fx[sl].as_mut().and_then(|e| e.plugin.as_mut()) {
                Some(s) => (s.live.as_mut().map(|l| &mut l.core), s.error.clone(), s.info.name.clone()),
                None => {
                    self.plugin_window = None;
                    return;
                }
            },
        };
        let mut open = true;
        let filter = &mut self.plugin_filter;
        let mut hint = None;
        egui::Window::new(format!("{name} - parameters"))
            .id(egui::Id::new("plugin_params"))
            .open(&mut open)
            .default_size(vec2(420.0, 440.0))
            .show(ctx, |ui| {
                let Some(core) = core else {
                    ui.colored_label(RED, error.unwrap_or_else(|| "Loading...".into()));
                    return;
                };
                ui.horizontal(|ui| {
                    ui.label("Search");
                    ui.add(egui::TextEdit::singleline(filter).desired_width(180.0));
                    ui.label(egui::RichText::new(format!("{} parameters", core.params().len())).color(TEXT_DIM).size(11.0));
                });
                ui.label(
                    egui::RichText::new("The plugin's own window cannot be shown here; all its settings are below. Double-click a name to reset it.")
                        .color(TEXT_DIM)
                        .size(10.0),
                );
                ui.separator();
                let needle = filter.to_lowercase();
                let shown: Vec<_> = core.params().iter().filter(|q| needle.is_empty() || q.name.to_lowercase().contains(&needle)).cloned().collect();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 22.0, shown.len(), |ui, range| {
                    for q in &shown[range] {
                        ui.horizontal(|ui| {
                            let label = ui.add(egui::Label::new(egui::RichText::new(&q.name).size(11.0)).truncate().sense(egui::Sense::click()));
                            let mut v = core.value(q.id).unwrap_or(q.default);
                            ui.add_space((150.0 - label.rect.width()).max(0.0));
                            let mut slider = egui::Slider::new(&mut v, q.min..=q.max).show_value(false);
                            if q.stepped {
                                slider = slider.step_by(1.0);
                            }
                            let r = ui.add_sized(vec2(150.0, 18.0), slider);
                            if r.changed() {
                                core.set(q.id, v);
                            }
                            if label.double_clicked() {
                                core.set(q.id, q.default);
                            }
                            let text = core.value_text(q.id, v).unwrap_or_else(|| format!("{v:.3}"));
                            ui.label(egui::RichText::new(text).color(TEXT_DIM).size(10.5));
                            if r.hovered() {
                                hint = Some(q.name.clone());
                            }
                        });
                    }
                });
            });
        if let Some(h) = hint {
            self.hint = format!("{name}: {h}");
        }
        if !open {
            self.plugin_window = None;
        }
    }

    // ---------------------------------------------------------------- audio recording

    /// Starts and stops capturing sound with playback while the record button
    /// is on and audio recording is chosen. A finished take becomes a sampler
    /// channel with a note where the take started.
    pub fn maintain_recording(&mut self, p: &mut Project) {
        let want_input = self.recording && self.settings.record_audio && self.screenshot.is_none();
        if want_input && self.input_stream.is_none() && !self.input_failed {
            self.input_stream = crate::record::start_input(self.recorder.clone(), self.settings.input_device.as_deref());
            if self.input_stream.is_none() {
                self.input_failed = true;
                self.hint = "No sound input found for audio recording".into();
            }
        }
        if !want_input {
            self.input_failed = false;
        }
        let capture = want_input && p.playing && self.input_stream.is_some();
        match (capture, self.take.is_some()) {
            (true, false) => {
                if let Ok(mut r) = self.recorder.lock() {
                    r.frames.clear();
                    r.active = true;
                }
                self.take = Some(TakeStart { position: p.position, mode: p.mode, pattern: p.current_pattern });
                self.hint = "Recording sound from the input...".into();
            }
            (false, true) => {
                let (frames, rate) = match self.recorder.lock() {
                    Ok(mut r) => {
                        r.active = false;
                        (std::mem::take(&mut r.frames), r.rate)
                    }
                    Err(_) => (Vec::new(), 0.0),
                };
                if let Some(start) = self.take.take() {
                    self.finish_take(p, frames, rate, start);
                }
            }
            _ => {}
        }
        if !want_input && self.take.is_none() {
            self.input_stream = None;
        }
    }

    fn finish_take(&mut self, p: &mut Project, frames: Vec<[f32; 2]>, rate: f32, start: TakeStart) {
        if rate <= 0.0 || (frames.len() as f32) < rate * 0.2 {
            self.hint = "The take was too short to keep".into();
            return;
        }
        let dir = file::folder(file::RECORDED);
        let path = (1..).map(|n| file::path_in(&dir, &format!("Take {n}"), "wav")).find(|p| !p.exists()).unwrap_or_default();
        let data = Arc::new(SampleData { rate, frames });
        self.samples.insert(path.clone(), data.clone());
        let (write_path, write_data) = (path.clone(), data.clone());
        std::thread::spawn(move || {
            if let Err(e) = file::write_frames(&write_path, &write_data.frames, write_data.rate as u32) {
                eprintln!("could not save the recording: {e}");
            }
        });
        let secs = data.seconds();
        let c = file::sampler_channel(p, &path, Some(data));
        let name = c.name.clone();
        p.push_channel(c);
        let ch = p.channels.len() - 1;
        let snap = |t: f64| ((t / TICKS_PER_STEP as f64).round() as u32) * TICKS_PER_STEP;
        let note = |start: u32| Note { key: STEP_KEY, start, len: TICKS_PER_STEP, vel: NOTE_VELOCITY };
        match start.mode {
            Mode::Pattern => {
                let pat = start.pattern.min(p.patterns.len() - 1);
                let len = p.patterns[pat].length_ticks();
                p.patterns[pat].notes[ch].push(note(snap(start.position) % len));
            }
            Mode::Song => {
                let bar = (start.position / TICKS_PER_BAR as f64).floor() as u32;
                let offset = snap(start.position - (bar * TICKS_PER_BAR) as f64).min(TICKS_PER_BAR - TICKS_PER_STEP);
                let pat = p.patterns.len();
                p.ensure_pattern(pat);
                p.patterns[pat].name = name.clone();
                p.patterns[pat].notes[ch].push(note(offset));
                let busy = |t: usize| p.clips.iter().any(|c| c.track == t && c.start_bar <= bar && bar < c.start_bar + p.patterns[c.pattern].length_ticks() / TICKS_PER_BAR);
                let track = (0..PLAYLIST_TRACKS).find(|&t| !busy(t)).unwrap_or(PLAYLIST_TRACKS - 1);
                p.clips.push(Clip { track, start_bar: bar, pattern: pat });
            }
        }
        self.selected_channel = ch;
        self.hint = format!("Recorded {secs:.1} s into {name}");
    }

    /// Ctrl+Z / Ctrl+Y (or Ctrl+Shift+Z), wherever the keyboard focus is
    /// (except while typing text).
    pub fn undo_keys(&mut self, ctx: &egui::Context, p: &mut Project) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let (undo, redo) = ctx.input(|i| {
            let cmd = i.modifiers.command;
            (cmd && !i.modifiers.shift && i.key_pressed(Key::Z), cmd && (i.key_pressed(Key::Y) || (i.modifiers.shift && i.key_pressed(Key::Z))))
        });
        if undo {
            self.undo_step(p, true);
        }
        if redo {
            self.undo_step(p, false);
        }
    }
}
