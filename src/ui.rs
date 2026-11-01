// Main window: toolbar, browser, playlist. The editor windows live in editors.rs.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Key, Pos2, Rect, Sense, Stroke, Ui};

use crate::engine::{
    self, Clip, Instrument, Mode, Note, Project, Shared, BEATS_PER_BAR, PLAYLIST_TRACKS, STEPS_PER_BAR,
    TEMPO_MAX, TEMPO_MIN, TICKS_PER_BAR, TICKS_PER_STEP,
};
use crate::widgets::*;

const BROWSER_FOLDERS: [&str; 18] = [
    "Recent files",
    "Plugin database",
    "Plugin presets",
    "Channel presets",
    "Mixer presets",
    "Scores",
    "Clipboard files",
    "Demo projects",
    "Envelopes",
    "Impulses",
    "Misc",
    "My projects",
    "Project bones",
    "Recorded",
    "Rendered",
    "Sliced audio",
    "Soundfonts",
    "Templates",
];

#[derive(Clone, Copy, PartialEq)]
pub enum DragKind {
    Move,
    Resize,
}

#[derive(Clone, Copy)]
pub struct PianoDrag {
    pub index: usize,
    pub kind: DragKind,
    pub grab_ticks: i64,
    pub grab_key: i32,
    pub orig: Note,
}

#[derive(Clone, Copy)]
struct ClipDrag {
    index: usize,
    grab_bars: i64,
}

pub struct StudioApp {
    shared: Shared,
    _stream: Option<cpal::Stream>,
    screenshot: Option<PathBuf>,
    frames: u32,
    shot_requested: bool,
    pub show_browser: bool,
    pub show_playlist: bool,
    pub show_rack: bool,
    pub show_piano: bool,
    pub show_mixer: bool,
    show_about: bool,
    packs_open: bool,
    project_open: bool,
    recording: bool,
    pub selected_channel: usize,
    pub selected_insert: usize,
    pub paint: Option<bool>,
    pub piano_drag: Option<PianoDrag>,
    clip_drag: Option<ClipDrag>,
    pub last_note_len: u32,
    pub piano_scrolled: bool,
    pub hint: String,
}

impl StudioApp {
    pub fn new(cc: &eframe::CreationContext<'_>, screenshot: Option<PathBuf>) -> Self {
        apply_style(&cc.egui_ctx);
        let shared: Shared = Arc::new(Mutex::new(Project::new()));
        let stream = if screenshot.is_none() { engine::start_audio(shared.clone()) } else { None };
        // Developer check: open every window with a sample beat.
        let open_all = std::env::var_os("BEATSTUDIO_OPEN_ALL").is_some();
        if open_all {
            if let Ok(mut p) = shared.lock() {
                for s in [0, 4, 8, 12] {
                    p.patterns[0].set_step(0, s, true);
                }
                p.patterns[0].set_step(1, 4, true);
                p.patterns[0].set_step(1, 12, true);
                for s in (2..16).step_by(4) {
                    p.patterns[0].set_step(2, s, true);
                }
                p.patterns[0].notes[4].push(Note { key: 64, start: 0, len: 96, vel: 100 });
                p.clips.push(Clip { track: 0, start_bar: 0, pattern: 0 });
                p.clips.push(Clip { track: 0, start_bar: 1, pattern: 0 });
            }
        }
        StudioApp {
            shared,
            _stream: stream,
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
            recording: false,
            selected_channel: 0,
            selected_insert: 0,
            paint: None,
            piano_drag: None,
            clip_drag: None,
            last_note_len: TICKS_PER_STEP * 2,
            piano_scrolled: false,
            hint: "Ready".into(),
        }
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else { return };
        self.frames += 1;
        ctx.request_repaint();
        if self.frames >= 120 && !self.shot_requested {
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
        if ctx.wants_keyboard_input() {
            return;
        }
        ctx.input(|i| {
            if i.key_pressed(Key::Space) {
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
            if i.key_pressed(Key::L) {
                p.mode = if p.mode == Mode::Pattern { Mode::Song } else { Mode::Pattern };
                p.position = 0.0;
            }
            if i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals) {
                p.ensure_pattern(p.current_pattern + 1);
                p.current_pattern += 1;
            }
            if i.key_pressed(Key::Minus) && p.current_pattern > 0 {
                p.current_pattern -= 1;
            }
        });
    }

    // ---------------------------------------------------------------- top bar

    fn top_bar(&mut self, ctx: &egui::Context, p: &mut Project) {
        egui::TopBottomPanel::top("top")
            .frame(egui::Frame::none().fill(HEADER).inner_margin(egui::Margin::symmetric(6.0, 4.0)))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    self.menus(ui, p);
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new(&self.hint).color(TEXT_DIM).size(11.0));
                });
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
                    self.transport(ui, p);
                });
            });
    }

    fn menus(&mut self, ui: &mut Ui, p: &mut Project) {
        ui.menu_button("FILE", |ui| {
            if ui.button("New").clicked() {
                let (sr, ok) = (p.sample_rate, p.audio_ok);
                *p = Project::new();
                p.sample_rate = sr;
                p.audio_ok = ok;
                self.selected_channel = 0;
                ui.close_menu();
            }
            if ui.button("Clear current pattern").clicked() {
                for list in &mut p.patterns[p.current_pattern].notes {
                    list.clear();
                }
                ui.close_menu();
            }
        });
        ui.menu_button("EDIT", |ui| {
            if ui.button("Clear playlist").clicked() {
                p.clips.clear();
                ui.close_menu();
            }
        });
        ui.menu_button("ADD", |ui| {
            for inst in Instrument::ALL {
                if ui.button(inst.label()).clicked() {
                    p.add_channel(inst);
                    self.selected_channel = p.channels.len() - 1;
                    ui.close_menu();
                }
            }
        });
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
        });
        ui.menu_button("VIEW", |ui| {
            ui.checkbox(&mut self.show_playlist, "Playlist   F5");
            ui.checkbox(&mut self.show_rack, "Channel rack   F6");
            ui.checkbox(&mut self.show_piano, "Piano roll   F7");
            ui.checkbox(&mut self.show_browser, "Browser   F8");
            ui.checkbox(&mut self.show_mixer, "Mixer   F9");
        });
        ui.menu_button("OPTIONS", |ui| {
            ui.label(if p.audio_ok { "Sound output: on" } else { "Sound output: not available" });
            ui.label(format!("Sample rate: {} Hz", p.sample_rate as u32));
        });
        ui.menu_button("TOOLS", |ui| {
            if ui.button("Fill every 4 steps").clicked() {
                let ch = self.selected_channel.min(p.channels.len() - 1);
                for s in (0..STEPS_PER_BAR).step_by(4) {
                    p.patterns[p.current_pattern].set_step(ch, s, true);
                }
                ui.close_menu();
            }
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
        if rec.on_hover_text("Record (marks the button only)").clicked() {
            self.recording = !self.recording;
        }

        lcd(ui, |ui| {
            let r = ui.add(egui::DragValue::new(&mut p.tempo).range(TEMPO_MIN..=TEMPO_MAX).speed(0.2).fixed_decimals(3));
            p.tempo = engine::quantize_tempo(p.tempo);
            if r.hovered() {
                self.hint = "Tempo in beats per minute - drag up/down or double-click to type".into();
            }
            ui.label(egui::RichText::new("BPM").color(TEXT_DIM).size(9.0));
        });

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
        if knob(ui, &mut mv, 0.0, engine::FADER_MAX, engine::FADER_DEFAULT, 26.0).hovered() {
            self.hint = format!("Main volume {:.1} dB", 20.0 * (mv.max(1.0) / engine::VOL_SCALE).log10());
        }
        p.mixer[0].volume = mv;
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
            .frame(egui::Frame::none().fill(PANEL_DARK).inner_margin(6.0).stroke(Stroke::new(1.0, LINE)))
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
                        });
                    }
                    for name in BROWSER_FOLDERS {
                        if folder(ui, name, false).clicked() {
                            self.hint = format!("{name}: nothing here yet");
                        }
                    }
                });
            });
    }

    // ---------------------------------------------------------------- playlist

    fn desktop(&mut self, ctx: &egui::Context, p: &mut Project) {
        egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
            let full = ui.max_rect();
            gradient(ui.painter(), full, BG_TOP, BG_BOTTOM);
            if self.show_playlist {
                let inner = full.shrink(6.0);
                let mut child = ui.child_ui(inner, egui::Layout::top_down(egui::Align::Min), None);
                egui::Frame::none().fill(PANEL).stroke(Stroke::new(1.0, LINE)).rounding(3.0).show(&mut child, |ui| {
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
        let mut child = ui.child_ui(area, egui::Layout::top_down(egui::Align::Min), None);
        egui::ScrollArea::both().auto_shrink([false, false]).show(&mut child, |ui| self.arrangement(ui, p));
    }

    fn arrangement(&mut self, ui: &mut Ui, p: &mut Project) {
        let bar_w = 44.0;
        let row_h = 30.0;
        let head_w = 86.0;
        let ruler_h = 20.0;
        let bars = (p.song_length_ticks().div_ceil(TICKS_PER_BAR) + 16).max(64);
        let total = vec2(head_w + bars as f32 * bar_w, ruler_h + PLAYLIST_TRACKS as f32 * row_h);
        let (rect, _) = ui.allocate_exact_size(total, Sense::hover());
        let painter = ui.painter_at(ui.clip_rect());
        let grid = Rect::from_min_max(pos2(rect.left() + head_w, rect.top() + ruler_h), rect.max);

        for t in 0..PLAYLIST_TRACKS {
            let y0 = grid.top() + t as f32 * row_h;
            let lane = Rect::from_min_max(pos2(grid.left(), y0), pos2(grid.right(), y0 + row_h));
            painter.rect_filled(lane, 0.0, if t % 2 == 0 { PANEL } else { PANEL_ALT });
        }
        for b in 0..=bars {
            let x = grid.left() + b as f32 * bar_w;
            painter.line_segment([pos2(x, grid.top()), pos2(x, grid.bottom())], Stroke::new(1.0, LINE));
            if b < bars {
                for q in 1..BEATS_PER_BAR {
                    let xq = x + q as f32 * bar_w / BEATS_PER_BAR as f32;
                    painter.line_segment([pos2(xq, grid.top()), pos2(xq, grid.bottom())], Stroke::new(1.0, LINE_FAINT));
                }
            }
        }
        for t in 0..=PLAYLIST_TRACKS {
            let y = grid.top() + t as f32 * row_h;
            painter.line_segment([pos2(grid.left(), y), pos2(grid.right(), y)], Stroke::new(1.0, LINE));
        }

        // Ruler: click to move the song position.
        let ruler = Rect::from_min_max(pos2(grid.left(), rect.top()), pos2(grid.right(), grid.top()));
        painter.rect_filled(ruler, 0.0, PANEL_DARK);
        for b in 0..bars {
            let x = grid.left() + b as f32 * bar_w;
            painter.text(pos2(x + 3.0, ruler.center().y), Align2::LEFT_CENTER, format!("{}", b + 1), FontId::proportional(10.0), TEXT_DIM);
        }
        let rr = ui.interact(ruler, ui.id().with("ruler"), Sense::click());
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

        // Track headers stay pinned on the left while scrolling.
        let vis = ui.clip_rect();
        let heads = Rect::from_min_max(pos2(vis.left(), grid.top()), pos2(vis.left() + head_w, grid.bottom()));
        painter.rect_filled(heads, 0.0, HEADER);
        for t in 0..PLAYLIST_TRACKS {
            let y0 = grid.top() + t as f32 * row_h;
            painter.line_segment([pos2(heads.left(), y0), pos2(heads.right(), y0)], Stroke::new(1.0, LINE));
            painter.circle_filled(pos2(heads.left() + 10.0, y0 + row_h / 2.0), 3.5, LED_ON);
            painter.text(pos2(heads.left() + 20.0, y0 + row_h / 2.0), Align2::LEFT_CENTER, format!("Track {}", t + 1), FontId::proportional(11.0), TEXT);
        }
        painter.rect_filled(Rect::from_min_max(pos2(vis.left(), rect.top()), pos2(vis.left() + head_w, grid.top())), 0.0, PANEL_DARK);

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
            self.hint = "Playlist: click to place the selected pattern, right-click to delete".into();
        }

        if p.mode == Mode::Song {
            let x = grid.left() + p.position as f32 / TICKS_PER_BAR as f32 * bar_w;
            painter.line_segment([pos2(x, rect.top()), pos2(x, grid.bottom())], Stroke::new(1.5, ACCENT));
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
        });
        self.show_about = open;
    }
}

impl eframe::App for StudioApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let shared = self.shared.clone();
        let mut guard = match shared.lock() {
            Ok(g) => g,
            Err(poison) => poison.into_inner(),
        };
        let p: &mut Project = &mut guard;
        self.shortcuts(ctx, p);
        self.top_bar(ctx, p);
        self.browser(ctx, p);
        self.desktop(ctx, p);
        self.channel_rack(ctx, p);
        self.piano_roll(ctx, p);
        self.mixer(ctx, p);
        self.about(ctx);
        drop(guard);
        self.handle_screenshot(ctx);
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}
