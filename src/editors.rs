// Channel rack, piano roll and mixer windows.

use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Key, Pos2, Rect, Sense, Stroke, Ui};

use crate::effects::{Effect, EffectKind, FX_SLOTS};
use crate::notetools;
use crate::engine::{self, Instrument, Mode, Note, Project, Target, MIXER_INSERTS, NOTE_VELOCITY, STEPS_PER_BAR, TICKS_PER_BAR, TICKS_PER_STEP};
use crate::ui::{DragKind, NoteAction, PianoDrag, PickFor, PluginTarget, StudioApp};
use crate::widgets::*;

/// Height of the velocity lane at the bottom of the piano roll.
const VEL_LANE_H: f32 = 58.0;
const NOTE_SELECTED: Color32 = Color32::from_rgb(0xF2, 0x8E, 0x86);

fn mix(a: Color32, b: Color32, f: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * f) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

impl StudioApp {
    // ---------------------------------------------------------------- channel rack

    pub fn channel_rack(&mut self, ctx: &egui::Context, p: &mut Project) {
        if !self.show_rack {
            return;
        }
        let mut open = true;
        egui::Window::new("Channel rack")
            .open(&mut open)
            .default_pos(pos2(330.0, 120.0))
            .default_width(640.0)
            .resizable(true)
            .show(ctx, |ui| self.rack_ui(ui, p));
        if !open {
            self.show_rack = false;
        }
    }

    fn rack_ui(&mut self, ui: &mut Ui, p: &mut Project) {
        let pat = p.current_pattern;
        let steps = ((p.patterns[pat].length_ticks() / TICKS_PER_STEP) as usize).clamp(STEPS_PER_BAR, 64);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&p.patterns[pat].name).color(ACCENT));
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Swing").color(TEXT_DIM).size(11.0));
            let mut sw = p.swing;
            if knob(ui, &mut sw, 0.0, 1.0, engine::MAIN_SWING, 20.0).hovered() {
                self.hint = format!("Swing {:.0}%", sw * 100.0);
            }
            p.swing = sw;
            ui.add_space(8.0);
            if ui.small_button("+ bar").on_hover_text("Make the pattern one bar longer").clicked() {
                let len = p.patterns[pat].length_ticks();
                let ch = self.selected_channel.min(p.channels.len() - 1);
                // A silent marker note keeps the extra bar.
                p.patterns[pat].notes[ch].push(Note { key: engine::STEP_KEY, start: len + TICKS_PER_BAR - 1, len: 1, vel: 0 });
            }
        });
        ui.add_space(2.0);

        let pointer_down = ui.input(|i| i.pointer.primary_down());
        let pressed = ui.input(|i| i.pointer.primary_pressed());
        if !pointer_down {
            self.paint = None;
        }
        let playing_step = (p.playing && p.mode == Mode::Pattern).then_some((p.position as u32 / TICKS_PER_STEP) as usize);

        let mut remove: Option<usize> = None;
        for ch in 0..p.channels.len() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                let enabled = p.channels[ch].enabled;
                if led(ui, enabled).on_hover_text("Mute / unmute").clicked() {
                    p.channels[ch].enabled = !enabled;
                }
                let (mut pan, mut vol) = (p.channels[ch].pan, p.channels[ch].volume);
                let pr = knob(ui, &mut pan, -engine::PAN_CENTRE, engine::PAN_CENTRE, 0.0, 18.0);
                if pr.hovered() {
                    self.hint = format!("{} pan {:+.0}% - right-click to automate", p.channels[ch].name, pan / engine::PAN_CENTRE * 100.0);
                }
                let vr = knob(ui, &mut vol, 0.0, engine::VOL_SCALE, engine::CHANNEL_VOLUME, 18.0);
                if vr.hovered() {
                    self.hint = format!("{} volume {:.0}% - right-click to automate", p.channels[ch].name, vol / engine::VOL_SCALE * 100.0);
                }
                p.channels[ch].pan = pan;
                p.channels[ch].volume = vol;
                self.automate_menu(&pr, p, Target::ChannelPan(ch));
                self.automate_menu(&vr, p, Target::ChannelVolume(ch));
                let mut track = p.channels[ch].mixer;
                lcd_small(ui, |ui| {
                    ui.add(egui::DragValue::new(&mut track).range(0..=MIXER_INSERTS).speed(0.1)).on_hover_text("Mixer track");
                });
                p.channels[ch].mixer = track;

                let sel = ch == self.selected_channel;
                let (r, resp) = ui.allocate_exact_size(vec2(118.0, 22.0), Sense::click());
                let c = &p.channels[ch];
                let (status, tip) = self.channel_status(c);
                let fill = match c.instrument {
                    _ if sel => Color32::from_rgb(0x8A, 0x94, 0x9B),
                    Instrument::Sampler => Color32::from_rgb(0x4E, 0x5A, 0x52),
                    Instrument::Plugin => Color32::from_rgb(0x52, 0x50, 0x62),
                    _ => HEADER,
                };
                ui.painter().rect_filled(r, 2.0, fill);
                let col = if sel { PANEL_DEEP } else { TEXT };
                ui.painter().with_clip_rect(r.shrink(2.0)).text(r.left_center() + vec2(6.0, 0.0), Align2::LEFT_CENTER, &c.name, FontId::proportional(12.0), col);
                if let Some(s) = status {
                    ui.painter().circle_filled(r.right_center() - vec2(6.0, 0.0), 3.0, s);
                }
                let resp = match tip {
                    Some(t) => resp.on_hover_text(t),
                    None => resp,
                };
                if resp.clicked() {
                    self.selected_channel = ch;
                    p.previews.push((ch, engine::STEP_KEY));
                }
                if resp.double_clicked() {
                    self.selected_channel = ch;
                    self.show_piano = true;
                }
                resp.context_menu(|ui| {
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut p.channels[ch].name);
                    });
                    if ui.button("Piano roll").clicked() {
                        self.selected_channel = ch;
                        self.show_piano = true;
                        ui.close_menu();
                    }
                    if p.channels[ch].plugin.is_some() && ui.button("Plugin parameters...").clicked() {
                        self.plugin_window = Some(PluginTarget::Channel(ch));
                        ui.close_menu();
                    }
                    if p.channels[ch].plugin.is_some() && ui.button("Plugin window").clicked() {
                        self.toggle_plugin_gui(p, PluginTarget::Channel(ch));
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Load sound file...").clicked() {
                        self.pick(PickFor::ReplaceSample(ch));
                        ui.close_menu();
                    }
                    ui.menu_button("Use plugin instrument", |ui| self.instrument_plugin_menu(ui, p, Some(ch)));
                    ui.menu_button("Use built-in sound", |ui| {
                        for inst in Instrument::ALL {
                            if ui.button(inst.label()).clicked() {
                                let c = &mut p.channels[ch];
                                c.instrument = inst;
                                c.sample = None;
                                c.plugin = None;
                                ui.close_menu();
                            }
                        }
                    });
                    ui.separator();
                    if ui.button("Clear notes").clicked() {
                        p.patterns[pat].notes[ch].clear();
                        ui.close_menu();
                    }
                    if ui.button("Delete").clicked() {
                        remove = Some(ch);
                        ui.close_menu();
                    }
                });

                ui.add_space(4.0);
                for s in 0..steps {
                    let (r, _) = ui.allocate_exact_size(vec2(17.0, 22.0), Sense::hover());
                    let on = p.patterns[pat].has_step(ch, s);
                    let base = if (s / 4) % 2 == 0 { STEP_A } else { STEP_B };
                    let mut fill = if on { STEP_ON } else { base };
                    if playing_step == Some(s) {
                        fill = if on { ACCENT } else { base.linear_multiply(1.35) };
                    }
                    ui.painter().rect_filled(r.shrink2(vec2(1.0, 0.0)), 2.0, fill);
                    if !on {
                        let mark = Rect::from_min_size(r.left_bottom() + vec2(3.0, -6.0), vec2(r.width() - 6.0, 2.0));
                        ui.painter().rect_filled(mark, 1.0, base.linear_multiply(0.7));
                    }
                    if ui.rect_contains_pointer(r) {
                        if pressed {
                            self.paint = Some(!on);
                            self.selected_channel = ch;
                            self.piano_focus = false;
                        }
                        if let Some(v) = self.paint {
                            if v != on {
                                p.patterns[pat].set_step(ch, s, v);
                                if v {
                                    p.previews.push((ch, engine::STEP_KEY));
                                }
                            }
                        }
                    }
                }
            });
        }
        if let Some(ch) = remove {
            p.remove_channel(ch);
            self.plugin_window = match self.plugin_window {
                Some(PluginTarget::Channel(c)) if c == ch => None,
                Some(PluginTarget::Channel(c)) if c > ch => Some(PluginTarget::Channel(c - 1)),
                other => other,
            };
            self.after_project_change(p);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.menu_button("  +  ", |ui| self.add_channel_menu(ui, p)).response.on_hover_text("Add a channel");
            ui.label(egui::RichText::new("click or drag across steps · double-click a name for the piano roll · right-click for more").color(TEXT_DIM).size(10.0));
        });
    }

    /// A status dot (loading / failed) and a tooltip for sampler and plugin channels.
    fn channel_status(&self, c: &engine::Channel) -> (Option<Color32>, Option<String>) {
        if let Some(s) = &c.sample {
            let tip = Some(format!("Sound file: {}", s.path.display()));
            return match () {
                _ if s.data.is_some() => (None, tip),
                _ if self.failed.contains(&s.path) => (Some(RED), Some(format!("Cannot read {}", s.path.display()))),
                _ => (Some(ACCENT), tip),
            };
        }
        if let Some(pl) = &c.plugin {
            return match &pl.error {
                Some(e) => (Some(RED), Some(format!("{} could not load: {e}", pl.info.name))),
                None => (None, Some(format!("Plugin: {} ({}) - right-click for its parameters", pl.info.name, pl.info.vendor))),
            };
        }
        (None, None)
    }

    // ---------------------------------------------------------------- piano roll

    pub fn piano_roll(&mut self, ctx: &egui::Context, p: &mut Project) {
        if !self.show_piano {
            return;
        }
        let ch = self.selected_channel.min(p.channels.len() - 1);
        let title = format!("Piano roll - {}", p.channels[ch].name);
        let mut open = true;
        egui::Window::new(title)
            .id(egui::Id::new("piano_roll"))
            .open(&mut open)
            .default_pos(pos2(260.0, 300.0))
            .default_size(vec2(760.0, 400.0))
            .resizable(true)
            .show(ctx, |ui| self.piano_ui(ui, p, ch));
        if !open {
            self.show_piano = false;
            self.piano_focus = false;
        }
    }

    /// Keeps the note selection tied to the pattern and channel it was made in.
    fn check_selection(&mut self, p: &Project) -> (usize, usize) {
        let pat = p.current_pattern;
        let ch = self.selected_channel.min(p.channels.len() - 1);
        if self.note_sel_ctx != (pat, ch) {
            self.note_sel.clear();
            self.note_sel_ctx = (pat, ch);
        }
        let n = p.patterns[pat].notes[ch].len();
        self.note_sel.retain(|&i| i < n);
        (pat, ch)
    }

    /// Applies a piano roll tool to the selected notes (all notes when none
    /// is selected). Length markers (velocity 0) are left alone.
    fn note_tool(&mut self, p: &mut Project, pat: usize, ch: usize, tool: NoteTool) {
        let list = &mut p.patterns[pat].notes[ch];
        let picked: Vec<usize> = if self.note_sel.is_empty() {
            (0..list.len()).filter(|&i| list[i].vel > 0).collect()
        } else {
            self.note_sel.iter().copied().filter(|&i| i < list.len() && list[i].vel > 0).collect()
        };
        if picked.is_empty() {
            self.hint = "No notes to change".into();
            return;
        }
        let mut notes: Vec<Note> = picked.iter().map(|&i| list[i]).collect();
        match tool {
            NoteTool::Quantize => notetools::quantize(&mut notes, TICKS_PER_STEP),
            NoteTool::Legato => notetools::legato(&mut notes),
            NoteTool::Reverse => notetools::reverse(&mut notes),
            NoteTool::OctaveUp | NoteTool::OctaveDown => {
                let by = if matches!(tool, NoteTool::OctaveUp) { 12 } else { -12 };
                if !notetools::transpose(&mut notes, by) {
                    self.hint = "Some notes would leave the keyboard".into();
                    return;
                }
            }
            NoteTool::FlipKeys => notetools::flip_keys(&mut notes),
            NoteTool::Arpeggiate => notetools::arpeggiate(&mut notes, TICKS_PER_STEP),
            NoteTool::Humanize => {
                // A new variation each time.
                let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos());
                notetools::humanize(&mut notes, 12, seed);
            }
            NoteTool::Chop => {
                let pieces = notetools::chop(&notes, TICKS_PER_STEP);
                let mut rest: Vec<Note> = list.iter().enumerate().filter(|(i, _)| !picked.contains(i)).map(|(_, n)| *n).collect();
                let first = rest.len();
                rest.extend(pieces);
                *list = rest;
                if !self.note_sel.is_empty() {
                    self.note_sel = (first..list.len()).collect();
                }
                return;
            }
        }
        for (&i, n) in picked.iter().zip(notes) {
            list[i] = n;
        }
    }

    fn piano_ui(&mut self, ui: &mut Ui, p: &mut Project, ch: usize) {
        let (pat, _) = self.check_selection(p);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&p.patterns[pat].name).color(ACCENT));
            ui.label(egui::RichText::new("Snap: step").color(TEXT_DIM).size(11.0));
            ui.label(egui::RichText::new(format!("New note: {} steps", self.last_note_len / TICKS_PER_STEP)).color(TEXT_DIM).size(11.0));
            if !self.note_sel.is_empty() {
                ui.label(egui::RichText::new(format!("{} selected", self.note_sel.len())).color(NOTE_SELECTED).size(11.0));
            }
            if ui.small_button("Clear").clicked() {
                p.patterns[pat].notes[ch].clear();
                self.note_sel.clear();
            }
            let what = if self.note_sel.is_empty() { "all notes" } else { "the selected notes" };
            ui.menu_button("Tools", |ui| {
                ui.label(egui::RichText::new(format!("On {what}")).color(TEXT_DIM).size(10.0));
                for (tool, label, tip) in [
                    (NoteTool::Quantize, "Quantize", "Snap starts and lengths to the step grid"),
                    (NoteTool::Legato, "Legato", "Each note lasts until the next one starts"),
                    (NoteTool::Chop, "Chop", "Cut notes into one-step pieces"),
                    (NoteTool::Reverse, "Reverse", "Play the notes backwards in time"),
                    (NoteTool::OctaveUp, "Octave up", "Move the notes up 12 semitones"),
                    (NoteTool::OctaveDown, "Octave down", "Move the notes down 12 semitones"),
                    (NoteTool::FlipKeys, "Flip keys", "Mirror the melody upside down"),
                    (NoteTool::Arpeggiate, "Arpeggiate", "Spread each chord one step per note"),
                    (NoteTool::Humanize, "Humanize velocity", "Vary the velocities a little"),
                ] {
                    if ui.button(label).on_hover_text(tip).clicked() {
                        self.note_tool(p, pat, ch, tool);
                        ui.close_menu();
                    }
                }
            });
            ui.label(
                egui::RichText::new("click: add · drag: move · drag the end: resize · right-click: delete · Ctrl+drag: select · bottom: velocity")
                    .color(TEXT_DIM)
                    .size(10.0),
            );
        });

        let keys = 120usize;
        let mut sa = egui::ScrollArea::both().auto_shrink([false, false]).id_salt("piano_scroll");
        if !self.piano_scrolled {
            sa = sa.vertical_scroll_offset((keys as f32 - 76.0) * 13.0);
            self.piano_scrolled = true;
        }
        sa.show(ui, |ui| self.piano_grid(ui, p, ch, keys));
    }

    fn piano_grid(&mut self, ui: &mut Ui, p: &mut Project, ch: usize, keys: usize) {
        let pat = p.current_pattern;
        let key_h = 13.0;
        let step_w = 18.0;
        let kb_w = 52.0;
        let bars = (p.patterns[pat].length_ticks() / TICKS_PER_BAR + 1).max(4);
        let width = kb_w + bars as f32 * STEPS_PER_BAR as f32 * step_w;
        // Room under the lowest key so it can scroll above the velocity lane.
        let (rect, _) = ui.allocate_exact_size(vec2(width, keys as f32 * key_h + VEL_LANE_H), Sense::hover());
        let painter = ui.painter_at(ui.clip_rect());
        let grid = Rect::from_min_max(pos2(rect.left() + kb_w, rect.top()), pos2(rect.right(), rect.top() + keys as f32 * key_h));
        let key_y = |k: u8| grid.top() + (keys as f32 - 1.0 - k as f32) * key_h;
        let black = |k: u8| matches!(k % 12, 1 | 3 | 6 | 8 | 10);
        let tick_x = |t: u32| grid.left() + t as f32 / TICKS_PER_STEP as f32 * step_w;
        let vis = ui.clip_rect();
        let lane = Rect::from_min_max(pos2(vis.left() + kb_w, vis.bottom() - VEL_LANE_H), vis.right_bottom());

        for k in 0..keys as u8 {
            let row = Rect::from_min_size(pos2(grid.left(), key_y(k)), vec2(grid.width(), key_h));
            painter.rect_filled(row, 0.0, if black(k) { PANEL_DARK } else { PANEL });
            if k % 12 == 0 {
                painter.line_segment([row.left_bottom(), row.right_bottom()], Stroke::new(1.0_f32, LINE));
            }
        }
        for s in 0..=bars as usize * STEPS_PER_BAR {
            let x = grid.left() + s as f32 * step_w;
            let col = if s % STEPS_PER_BAR == 0 {
                Color32::from_rgb(0x14, 0x17, 0x1A)
            } else if s % 4 == 0 {
                LINE
            } else {
                LINE_FAINT
            };
            painter.line_segment([pos2(x, grid.top()), pos2(x, grid.bottom())], Stroke::new(1.0_f32, col));
        }
        let end_x = tick_x(p.patterns[pat].length_ticks());
        painter.rect_filled(Rect::from_min_max(pos2(end_x, grid.top()), grid.max), 0.0, Color32::from_black_alpha(50));

        let hover = ui.input(|i| i.pointer.hover_pos()).filter(|hp| !lane.contains(*hp));
        let mut hit: Option<(usize, DragKind)> = None;
        for (i, n) in p.patterns[pat].notes[ch].iter().enumerate() {
            if n.vel == 0 {
                continue;
            }
            let r = Rect::from_min_max(pos2(tick_x(n.start), key_y(n.key)), pos2(tick_x(n.start + n.len), key_y(n.key) + key_h)).shrink(0.5);
            let selected = self.note_sel.contains(&i);
            let base = if selected { NOTE_SELECTED } else { NOTE };
            painter.rect_filled(r, 2.0, mix(PANEL, base, 0.35 + 0.65 * n.vel as f32 / 127.0));
            painter.rect_stroke(r, 2.0, Stroke::new(1.0_f32, if selected { RED } else { NOTE_EDGE }));
            if r.width() > 26.0 {
                painter.text(r.left_center() + vec2(3.0, 0.0), Align2::LEFT_CENTER, key_name(n.key), FontId::proportional(9.0), PANEL_DEEP);
            }
            if let Some(hp) = hover {
                if r.contains(hp) {
                    let kind = if hp.x > r.right() - 6.0 { DragKind::Resize } else { DragKind::Move };
                    hit = Some((i, kind));
                }
            }
        }

        if p.playing && p.mode == Mode::Pattern {
            let x = tick_x(p.position as u32);
            painter.line_segment([pos2(x, grid.top()), pos2(x, grid.bottom())], Stroke::new(1.5_f32, ACCENT));
        }

        // Keyboard pinned to the left edge; click a key to hear it.
        for k in 0..keys as u8 {
            let r = Rect::from_min_size(pos2(vis.left(), key_y(k)), vec2(kb_w, key_h));
            let fill = if black(k) { Color32::from_rgb(0x20, 0x22, 0x24) } else { Color32::from_rgb(0xE4, 0xE6, 0xE6) };
            painter.rect_filled(r.shrink2(vec2(0.0, 0.5)), 1.0, fill);
            if k % 12 == 0 {
                painter.text(r.right_center() - vec2(4.0, 0.0), Align2::RIGHT_CENTER, key_name(k), FontId::proportional(9.0), PANEL_DEEP);
            }
            if ui.interact(r, ui.id().with(("key", k)), Sense::click()).clicked() {
                p.previews.push((ch, k));
            }
        }

        let area = Rect::from_min_max(pos2(vis.left() + kb_w, grid.top()), pos2(vis.right(), grid.bottom().min(lane.top())));
        let gr = ui.interact(area, ui.id().with("notes"), Sense::click_and_drag());
        let at = |pp: Pos2| {
            let t = ((pp.x - grid.left()) / step_w * TICKS_PER_STEP as f32).max(0.0) as i64;
            let k = (keys as f32 - 1.0 - ((pp.y - grid.top()) / key_h).floor()).clamp(0.0, keys as f32 - 1.0) as i32;
            (t, k)
        };
        let snap = |t: i64| (t / TICKS_PER_STEP as i64) * TICKS_PER_STEP as i64;
        let ctrl = ui.input(|i| i.modifiers.command);
        if gr.secondary_clicked() {
            if let Some((i, _)) = hit.take() {
                p.patterns[pat].notes[ch].remove(i);
                self.note_sel.retain(|&j| j != i);
                for j in &mut self.note_sel {
                    if *j > i {
                        *j -= 1;
                    }
                }
            }
        }
        let started = gr.drag_started_by(egui::PointerButton::Primary);
        if (started || gr.clicked()) && self.piano_drag.is_none() && self.rubber.is_none() {
            self.piano_focus = true;
            if let Some(pp) = gr.interact_pointer_pos() {
                let (t, k) = at(pp);
                match (hit, ctrl) {
                    // Ctrl+click toggles a note in the selection.
                    (Some((i, _)), true) if !started => {
                        if let Some(at) = self.note_sel.iter().position(|&j| j == i) {
                            self.note_sel.remove(at);
                        } else {
                            self.note_sel.push(i);
                        }
                    }
                    // Ctrl+drag on empty space draws a selection box.
                    (None, true) => {
                        if started {
                            self.rubber = Some(ui.input(|i| i.pointer.press_origin()).unwrap_or(pp));
                        } else {
                            self.note_sel.clear();
                        }
                    }
                    (Some((i, kind)), _) => {
                        if gr.clicked() && !ctrl {
                            self.note_sel = vec![i];
                        } else if !self.note_sel.contains(&i) {
                            if ctrl {
                                self.note_sel.push(i);
                            } else {
                                self.note_sel = vec![i];
                            }
                        }
                        let list = &p.patterns[pat].notes[ch];
                        let orig = list[i];
                        let group = self.note_sel.iter().map(|&j| (j, list[j])).collect();
                        self.piano_drag = Some(PianoDrag { index: i, kind, grab_ticks: t - orig.start as i64, grab_key: k - orig.key as i32, orig, group });
                    }
                    (None, false) => {
                        let list = &mut p.patterns[pat].notes[ch];
                        let n = Note { key: k as u8, start: snap(t) as u32, len: self.last_note_len, vel: NOTE_VELOCITY };
                        list.push(n);
                        p.previews.push((ch, n.key));
                        let index = list.len() - 1;
                        self.note_sel = vec![index];
                        let group = vec![(index, n)];
                        self.piano_drag = Some(PianoDrag { index, kind: DragKind::Move, grab_ticks: t - n.start as i64, grab_key: k - n.key as i32, orig: n, group });
                    }
                }
            }
        }
        if let (Some(d), Some(pp)) = (&self.piano_drag, gr.interact_pointer_pos()) {
            let list = &mut p.patterns[pat].notes[ch];
            let (t, k) = at(pp);
            match d.kind {
                DragKind::Move => {
                    let min_start = d.group.iter().map(|g| g.1.start as i64).min().unwrap_or(0);
                    let (lo_key, hi_key) = d.group.iter().fold((127i32, 0i32), |(lo, hi), g| (lo.min(g.1.key as i32), hi.max(g.1.key as i32)));
                    let dt = (snap((t - d.grab_ticks).max(0)) - d.orig.start as i64).max(-min_start);
                    let dk = ((k - d.grab_key) - d.orig.key as i32).clamp(-lo_key, keys as i32 - 1 - hi_key);
                    for &(j, o) in &d.group {
                        if let Some(n) = list.get_mut(j) {
                            n.start = (o.start as i64 + dt) as u32;
                            n.key = (o.key as i32 + dk) as u8;
                        }
                    }
                }
                DragKind::Resize => {
                    let end = snap(t + TICKS_PER_STEP as i64 / 2).max(d.orig.start as i64 + TICKS_PER_STEP as i64);
                    let delta = end - d.orig.start as i64 - d.orig.len as i64;
                    for &(j, o) in &d.group {
                        if let Some(n) = list.get_mut(j) {
                            n.len = (o.len as i64 + delta).max(TICKS_PER_STEP as i64) as u32;
                        }
                    }
                }
            }
        }
        if let Some(start) = self.rubber {
            let now = ui.input(|i| i.pointer.interact_pos()).unwrap_or(start);
            let sel = Rect::from_two_pos(start, now);
            painter.rect(sel, 0.0, Color32::from_rgba_unmultiplied(0xF2, 0x8E, 0x86, 30), Stroke::new(1.0_f32, NOTE_SELECTED));
            if !ui.input(|i| i.pointer.primary_down()) {
                self.rubber = None;
                self.note_sel = p.patterns[pat].notes[ch]
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| n.vel > 0)
                    .filter(|(_, n)| {
                        let r = Rect::from_min_max(pos2(tick_x(n.start), key_y(n.key)), pos2(tick_x(n.start + n.len), key_y(n.key) + key_h));
                        r.intersects(sel)
                    })
                    .map(|(i, _)| i)
                    .collect();
            }
        }
        if gr.drag_stopped() || gr.clicked() || !ui.input(|i| i.pointer.primary_down()) {
            if let Some(d) = self.piano_drag.take() {
                if let Some(n) = p.patterns[pat].notes[ch].get(d.index) {
                    if d.kind == DragKind::Resize {
                        self.last_note_len = n.len;
                    }
                }
            }
        }
        if gr.hovered() {
            self.hint = match hit {
                Some((_, DragKind::Resize)) => "Drag to change the note length".into(),
                Some(_) => "Drag to move the note (and the other selected ones), right-click to delete, Ctrl+click to select".into(),
                None => "Click to add a note, Ctrl+drag to select notes (then Ctrl+C / Ctrl+V / Del / arrows)".into(),
            };
        }

        self.velocity_lane(ui, p, ch, lane, &tick_x, step_w, kb_w);
    }

    /// Bars for the velocity of each note; drag across them to change it
    /// (only the selected notes when there is a selection).
    #[allow(clippy::too_many_arguments)]
    fn velocity_lane(&mut self, ui: &mut Ui, p: &mut Project, ch: usize, lane: Rect, tick_x: &dyn Fn(u32) -> f32, step_w: f32, kb_w: f32) {
        let pat = p.current_pattern;
        let painter = ui.painter_at(ui.clip_rect());
        painter.rect_filled(lane, 0.0, PANEL_DEEP);
        painter.line_segment([lane.left_top(), lane.right_top()], Stroke::new(1.0_f32, LINE));
        let corner = Rect::from_min_max(pos2(lane.left() - kb_w, lane.top()), pos2(lane.left(), lane.bottom()));
        painter.rect_filled(corner, 0.0, PANEL_DARK);
        painter.text(corner.center(), Align2::CENTER_CENTER, "Velocity", FontId::proportional(9.0), TEXT_DIM);
        let inner_h = VEL_LANE_H - 8.0;
        for (i, n) in p.patterns[pat].notes[ch].iter().enumerate() {
            if n.vel == 0 {
                continue;
            }
            let x = tick_x(n.start);
            if x < lane.left() - 4.0 || x > lane.right() {
                continue;
            }
            let h = n.vel as f32 / 127.0 * inner_h;
            let col = if self.note_sel.contains(&i) { NOTE_SELECTED } else { NOTE };
            painter.line_segment([pos2(x + 2.0, lane.bottom() - 4.0), pos2(x + 2.0, lane.bottom() - 4.0 - h)], Stroke::new(3.0_f32, col));
            painter.circle_filled(pos2(x + 2.0, lane.bottom() - 4.0 - h), 3.0, col);
        }
        let lr = ui.interact(lane, ui.id().with("velocity"), Sense::click_and_drag());
        if lr.hovered() {
            self.hint = "Velocity: drag up or down across the bars to change how hard the notes play".into();
        }
        if let (true, Some(pp)) = (lr.dragged() || lr.clicked() || lr.drag_started(), lr.interact_pointer_pos()) {
            self.piano_focus = true;
            let v = ((lane.bottom() - 4.0 - pp.y) / inner_h * 127.0).round().clamp(1.0, 127.0) as u8;
            let reach = (step_w * 0.5).max(6.0);
            let sel = self.note_sel.clone();
            for (i, n) in p.patterns[pat].notes[ch].iter_mut().enumerate() {
                let x = tick_x(n.start);
                if n.vel > 0 && (sel.is_empty() || sel.contains(&i)) && pp.x >= x - 2.0 && pp.x <= x + reach {
                    n.vel = v;
                }
            }
            self.hint = format!("Velocity {v}");
        }
    }

    /// Editing keys of the piano roll, active after it was clicked.
    pub fn piano_keys(&mut self, ctx: &egui::Context, p: &mut Project) {
        let (cmd, shift) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift));
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
        if cmd {
            for (k, a) in [(Key::A, NoteAction::SelectAll), (Key::C, NoteAction::Copy), (Key::X, NoteAction::Cut), (Key::V, NoteAction::Paste), (Key::B, NoteAction::Duplicate)] {
                if pressed(k) {
                    self.note_action(p, a);
                }
            }
            return;
        }
        if pressed(Key::Delete) || pressed(Key::Backspace) {
            self.note_action(p, NoteAction::Delete);
        }
        let (pat, ch) = self.check_selection(p);
        if self.note_sel.is_empty() {
            return;
        }
        let up = if shift { 12 } else { 1 };
        let moves = [(Key::ArrowUp, 0i64, up), (Key::ArrowDown, 0, -up), (Key::ArrowRight, TICKS_PER_STEP as i64, 0), (Key::ArrowLeft, -(TICKS_PER_STEP as i64), 0)];
        for (k, dt, dk) in moves {
            if !pressed(k) {
                continue;
            }
            let list = &mut p.patterns[pat].notes[ch];
            let ok = self.note_sel.iter().all(|&i| {
                let n = list[i];
                n.start as i64 + dt >= 0 && (0..=127).contains(&(n.key as i32 + dk))
            });
            if ok {
                for &i in &self.note_sel {
                    let n = &mut list[i];
                    n.start = (n.start as i64 + dt) as u32;
                    n.key = (n.key as i32 + dk) as u8;
                }
                if dk != 0 {
                    if let Some(&i) = self.note_sel.first() {
                        p.previews.push((ch, list[i].key));
                    }
                }
            }
        }
    }

    pub fn note_action(&mut self, p: &mut Project, action: NoteAction) {
        let (pat, ch) = self.check_selection(p);
        let list = &mut p.patterns[pat].notes[ch];
        let selected = |sel: &[usize], list: &[Note]| -> Vec<Note> {
            let mut v: Vec<Note> = sel.iter().map(|&i| list[i]).collect();
            v.sort_by_key(|n| n.start);
            v
        };
        // Copies land right after the copied notes, rounded up to a whole bar.
        let span = |notes: &[Note]| {
            let start = notes.iter().map(|n| n.start).min().unwrap_or(0);
            let end = notes.iter().map(|n| n.start + n.len).max().unwrap_or(0);
            (end - start).div_ceil(TICKS_PER_BAR).max(1) * TICKS_PER_BAR
        };
        match action {
            NoteAction::SelectAll => {
                self.note_sel = (0..list.len()).filter(|&i| list[i].vel > 0).collect();
                self.hint = format!("{} notes selected", self.note_sel.len());
            }
            NoteAction::Copy | NoteAction::Cut => {
                if self.note_sel.is_empty() {
                    self.hint = "Select notes first (Ctrl+drag, or Ctrl+A)".into();
                    return;
                }
                self.note_clipboard = selected(&self.note_sel, list);
                self.paste_count = 0;
                self.hint = format!("{} notes copied", self.note_clipboard.len());
                if action == NoteAction::Cut {
                    self.delete_selected(p, pat, ch);
                    self.hint = format!("{} notes cut", self.note_clipboard.len());
                }
            }
            NoteAction::Delete => self.delete_selected(p, pat, ch),
            NoteAction::Paste => {
                if self.note_clipboard.is_empty() {
                    self.hint = "Nothing to paste: copy notes first".into();
                    return;
                }
                self.paste_count += 1;
                let shift = span(&self.note_clipboard) * self.paste_count;
                let first = list.len();
                list.extend(self.note_clipboard.iter().map(|n| Note { start: n.start + shift, ..*n }));
                self.note_sel = (first..list.len()).collect();
                self.hint = format!("{} notes pasted", self.note_clipboard.len());
            }
            NoteAction::Duplicate => {
                if self.note_sel.is_empty() {
                    self.hint = "Select notes first (Ctrl+drag, or Ctrl+A)".into();
                    return;
                }
                let notes = selected(&self.note_sel, list);
                let shift = span(&notes);
                let first = list.len();
                list.extend(notes.iter().map(|n| Note { start: n.start + shift, ..*n }));
                self.note_sel = (first..list.len()).collect();
                self.hint = format!("{} notes duplicated", notes.len());
            }
        }
    }

    fn delete_selected(&mut self, p: &mut Project, pat: usize, ch: usize) {
        let mut sel = std::mem::take(&mut self.note_sel);
        sel.sort_unstable();
        sel.dedup();
        let list = &mut p.patterns[pat].notes[ch];
        for &i in sel.iter().rev() {
            if i < list.len() {
                list.remove(i);
            }
        }
    }

    // ---------------------------------------------------------------- mixer

    pub fn mixer(&mut self, ctx: &egui::Context, p: &mut Project) {
        if !self.show_mixer {
            return;
        }
        let mut open = true;
        egui::Window::new("Mixer")
            .open(&mut open)
            .default_pos(pos2(230.0, 400.0))
            .default_size(vec2(960.0, 330.0))
            .resizable(true)
            .show(ctx, |ui| self.mixer_ui(ui, p));
        if !open {
            self.show_mixer = false;
        }
    }

    fn mixer_ui(&mut self, ui: &mut Ui, p: &mut Project) {
        let strip_w = 50.0;
        let strip_h = 280.0;
        egui::SidePanel::right("insert_detail")
            .resizable(false)
            .exact_width(220.0)
            .frame(egui::Frame::none().fill(PANEL_DARK).inner_margin(6.0))
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.insert_panel(ui, p));
            });
        egui::ScrollArea::horizontal().auto_shrink([false, false]).show_viewport(ui, |ui, view| {
            let count = MIXER_INSERTS + 1;
            let (rect, _) = ui.allocate_exact_size(vec2(count as f32 * strip_w + 12.0, strip_h), Sense::hover());
            let first = (view.left() / strip_w).floor().max(0.0) as usize;
            let last = ((view.right() / strip_w).ceil() as usize + 1).min(count);
            for i in first..last {
                let x = rect.left() + i as f32 * strip_w + if i > 0 { 12.0 } else { 0.0 };
                self.strip(ui, p, i, Rect::from_min_size(pos2(x, rect.top()), vec2(strip_w - 2.0, strip_h)));
            }
        });
    }

    /// Name, routing, effect slots and the selected effect's knobs.
    fn insert_panel(&mut self, ui: &mut Ui, p: &mut Project) {
        let i = self.selected_insert;
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut p.mixer[i].name).desired_width(120.0).text_color(ACCENT));
            if i > 0 {
                let solo = p.mixer[i].solo;
                if led_button(ui, "Solo", solo).on_hover_text("Hear only this insert (and what feeds it)").clicked() {
                    p.mixer[i].solo = !solo;
                }
            }
        });
        let routed: Vec<String> = p.channels.iter().filter(|c| c.mixer == i).map(|c| c.name.clone()).collect();
        let from: Vec<String> = (1..p.mixer.len()).filter(|&j| p.mixer[j].routes.iter().any(|r| r.0 == i)).map(|j| p.mixer[j].name.clone()).collect();
        let mut sources = routed;
        sources.extend(from);
        let text = if sources.is_empty() { "Nothing routed here".to_string() } else { format!("From: {}", sources.join(", ")) };
        ui.label(egui::RichText::new(text).color(TEXT_DIM).size(10.0));

        if i > 0 {
            ui.add_space(2.0);
            ui.label(egui::RichText::new("Sends to").color(TEXT_DIM).size(11.0));
            let mut remove = None;
            for k in 0..p.mixer[i].routes.len() {
                let (to, mut level) = p.mixer[i].routes[k];
                ui.horizontal(|ui| {
                    let r = knob(ui, &mut level, 0.0, 1.0, 1.0, 16.0);
                    if r.hovered() {
                        self.hint = format!("Send level {:.0}%", level * 100.0);
                    }
                    let name = p.mixer.get(to).map_or("?".into(), |t| t.name.clone());
                    ui.label(egui::RichText::new(format!("to {name}")).size(11.0));
                    if ui.small_button("x").on_hover_text("Stop sending there").clicked() {
                        remove = Some(k);
                    }
                });
                p.mixer[i].routes[k].1 = level;
            }
            if let Some(k) = remove {
                p.mixer[i].routes.remove(k);
            }
            if p.mixer[i].routes.is_empty() {
                ui.colored_label(RED, egui::RichText::new("not sent anywhere: silent").size(10.0));
            }
            egui::ComboBox::from_id_salt("send_to").selected_text("Send to...").width(130.0).show_ui(ui, |ui| {
                for to in 0..p.mixer.len() {
                    if p.mixer[i].routes.iter().any(|r| r.0 == to) || !p.can_route(i, to) {
                        continue;
                    }
                    let name = p.mixer[to].name.clone();
                    if ui.selectable_label(false, name).clicked() {
                        p.mixer[i].routes.push((to, 1.0));
                    }
                }
            });
        }
        ui.separator();

        ui.label(egui::RichText::new("Effects").color(TEXT_DIM).size(11.0));
        for s in 0..FX_SLOTS {
            let sel = self.selected_slot == Some(s);
            egui::Frame::none().fill(if sel { HEADER } else { PANEL_DEEP }).rounding(2.0).inner_margin(3.0).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    match &mut p.mixer[i].fx[s] {
                        Some(e) => {
                            let on = e.enabled;
                            if led(ui, on).on_hover_text("Turn the effect on / off").clicked() {
                                e.enabled = !on;
                            }
                            let failed = e.plugin.as_ref().and_then(|pl| pl.error.clone());
                            let text = egui::RichText::new(format!("{}  {}", s + 1, e.name())).size(11.0).color(if failed.is_some() { RED } else { TEXT });
                            let r = ui.add(egui::Label::new(text).truncate().sense(Sense::click()));
                            if r.clicked() {
                                self.selected_slot = Some(s);
                            }
                            if r.double_clicked() && e.plugin.is_some() {
                                self.plugin_window = Some(PluginTarget::Effect(i, s));
                            }
                            let r = match failed {
                                Some(f) => r.on_hover_text(format!("Could not load: {f}")),
                                None => r,
                            };
                            r.context_menu(|ui| self.slot_menu(ui, p, i, s));
                        }
                        None => {
                            ui.menu_button(egui::RichText::new(format!("{}  (empty) +", s + 1)).size(11.0).color(TEXT_DIM), |ui| {
                                for kind in EffectKind::BUILT_IN {
                                    if ui.button(kind.label()).clicked() {
                                        p.mixer[i].fx[s] = Some(Effect::new(kind));
                                        self.selected_slot = Some(s);
                                        ui.close_menu();
                                    }
                                }
                                ui.separator();
                                ui.menu_button("Plugins", |ui| self.effect_plugin_menu(ui, p, i, s));
                            });
                        }
                    }
                });
            });
        }
        ui.separator();

        let Some(s) = self.selected_slot else {
            ui.label(egui::RichText::new("Click an effect to see its knobs. Right-click knobs to automate them.").color(TEXT_DIM).size(10.0));
            return;
        };
        let Some(e) = &p.mixer[i].fx[s] else {
            self.selected_slot = None;
            return;
        };
        if e.plugin.is_some() {
            ui.label(egui::RichText::new(e.name()).color(ACCENT));
            let mut toggle = false;
            ui.horizontal(|ui| {
                if ui.button("Parameters...").clicked() {
                    self.plugin_window = Some(PluginTarget::Effect(i, s));
                }
                toggle = ui.button("Plugin window").clicked();
            });
            if toggle {
                self.toggle_plugin_gui(p, PluginTarget::Effect(i, s));
            }
            return;
        }
        let kind = e.kind;
        ui.label(egui::RichText::new(kind.label()).color(ACCENT));
        ui.horizontal(|ui| {
            for k in 0..crate::effects::PARAMS {
                ui.vertical(|ui| {
                    ui.set_width(44.0);
                    let mut v = p.mixer[i].fx[s].as_ref().map_or(0.0, |e| e.params[k]);
                    let r = knob(ui, &mut v, 0.0, 1.0, kind.defaults()[k], 28.0);
                    if let Some(e) = &mut p.mixer[i].fx[s] {
                        e.params[k] = v;
                    }
                    ui.label(egui::RichText::new(kind.param_names()[k]).size(9.5).color(TEXT_DIM));
                    ui.label(egui::RichText::new(kind.describe(k, v)).size(9.5));
                    if r.hovered() {
                        self.hint = format!("{} {}: {} - right-click to automate", kind.label(), kind.param_names()[k], kind.describe(k, v));
                    }
                    self.automate_menu(&r, p, Target::Effect(i, s, k));
                });
            }
        });
    }

    fn slot_menu(&mut self, ui: &mut Ui, p: &mut Project, i: usize, s: usize) {
        if p.mixer[i].fx[s].as_ref().is_some_and(|e| e.plugin.is_some()) && ui.button("Parameters...").clicked() {
            self.plugin_window = Some(PluginTarget::Effect(i, s));
            ui.close_menu();
        }
        if p.mixer[i].fx[s].as_ref().is_some_and(|e| e.plugin.is_some()) && ui.button("Plugin window").clicked() {
            self.toggle_plugin_gui(p, PluginTarget::Effect(i, s));
            ui.close_menu();
        }
        if s > 0 && ui.button("Move up").clicked() {
            p.mixer[i].fx.swap(s, s - 1);
            self.retarget_slot(p, i, s, s - 1);
            ui.close_menu();
        }
        if s + 1 < FX_SLOTS && ui.button("Move down").clicked() {
            p.mixer[i].fx.swap(s, s + 1);
            self.retarget_slot(p, i, s, s + 1);
            ui.close_menu();
        }
        if ui.button("Remove").clicked() {
            p.remove_effect(i, s);
            if self.plugin_window == Some(PluginTarget::Effect(i, s)) {
                self.plugin_window = None;
            }
            self.selected_slot = None;
            ui.close_menu();
        }
    }

    /// Keeps automation and the open windows on an effect that changed slot.
    fn retarget_slot(&mut self, p: &mut Project, i: usize, a: usize, b: usize) {
        for au in &mut p.automations {
            if let Target::Effect(ai, s, _) = &mut au.target {
                if *ai == i && (*s == a || *s == b) {
                    *s = if *s == a { b } else { a };
                }
            }
        }
        self.selected_slot = Some(b);
        if let Some(PluginTarget::Effect(pi, ps)) = &mut self.plugin_window {
            if *pi == i && (*ps == a || *ps == b) {
                *ps = if *ps == a { b } else { a };
            }
        }
    }

    fn strip(&mut self, ui: &mut Ui, p: &mut Project, i: usize, r: Rect) {
        let sel = i == self.selected_insert;
        let used = i == 0 || p.channels.iter().any(|c| c.mixer == i) || p.mixer[i].fx.iter().any(|e| e.is_some());
        let painter = ui.painter();
        painter.rect_filled(r, 2.0, if sel { Color32::from_rgb(0x4E, 0x58, 0x60) } else { PANEL_DARK });
        if sel {
            painter.rect_stroke(r, 2.0, Stroke::new(1.0_f32, ACCENT));
        }
        painter.with_clip_rect(r.shrink(1.0)).text(
            pos2(r.center().x, r.top() + 9.0),
            Align2::CENTER_CENTER,
            &p.mixer[i].name,
            FontId::proportional(9.5),
            if used { TEXT } else { TEXT_DIM },
        );
        if p.mixer[i].fx.iter().flatten().any(|e| e.enabled) {
            painter.circle_filled(pos2(r.right() - 5.0, r.top() + 18.0), 2.5, ACCENT);
        }

        let enabled = p.mixer[i].enabled;
        let led_r = Rect::from_center_size(pos2(r.center().x - if i > 0 { 8.0 } else { 0.0 }, r.top() + 26.0), vec2(12.0, 8.0));
        painter.rect_filled(led_r, 2.0, if enabled { LED_ON } else { LED_OFF });
        if ui.interact(led_r.expand(3.0), ui.id().with(("mute", i)), Sense::click()).on_hover_text("Mute / unmute").clicked() {
            p.mixer[i].enabled = !enabled;
        }
        if i > 0 {
            let solo = p.mixer[i].solo;
            let sr = Rect::from_center_size(pos2(r.center().x + 9.0, r.top() + 26.0), vec2(13.0, 10.0));
            ui.painter().rect_filled(sr, 2.0, if solo { ACCENT } else { PANEL_DEEP });
            ui.painter().text(sr.center(), Align2::CENTER_CENTER, "S", FontId::proportional(8.0), if solo { PANEL_DEEP } else { TEXT_DIM });
            if ui.interact(sr.expand(2.0), ui.id().with(("solo", i)), Sense::click()).on_hover_text("Solo").clicked() {
                p.mixer[i].solo = !solo;
            }
        }

        let kr = Rect::from_center_size(pos2(r.center().x, r.top() + 46.0), vec2(20.0, 20.0));
        let mut pan = p.mixer[i].pan;
        let pr = ui.put(kr, |ui: &mut Ui| knob(ui, &mut pan, -engine::PAN_CENTRE, engine::PAN_CENTRE, 0.0, 18.0));
        if pr.hovered() {
            self.hint = format!("{} pan {:+.0}% - right-click to automate", p.mixer[i].name, pan / engine::PAN_CENTRE * 100.0);
        }
        p.mixer[i].pan = pan;
        self.automate_menu(&pr, p, Target::MixerPan(i));

        let top = r.top() + 64.0;
        let bottom = r.bottom() - 22.0;
        meter_v(ui.painter(), Rect::from_min_max(pos2(r.left() + 5.0, top), pos2(r.left() + 15.0, bottom)), p.mixer[i].peak);
        let track = Rect::from_min_max(pos2(r.center().x + 4.0, top), pos2(r.center().x + 8.0, bottom));
        ui.painter().rect_filled(track, 2.0, PANEL_DEEP);
        let max = engine::FADER_MAX;
        let fr = ui.interact(
            Rect::from_min_max(pos2(r.center().x - 6.0, top - 6.0), pos2(r.right() - 2.0, bottom + 6.0)),
            ui.id().with(("fader", i)),
            Sense::click_and_drag(),
        );
        if fr.dragged() {
            p.mixer[i].volume = (p.mixer[i].volume - fr.drag_delta().y / (bottom - top) * max).clamp(0.0, max);
        }
        if fr.double_clicked() {
            p.mixer[i].volume = engine::FADER_DEFAULT;
        }
        let hy = bottom - (p.mixer[i].volume / max) * (bottom - top);
        let handle = Rect::from_center_size(pos2(track.center().x, hy), vec2(22.0, 10.0));
        ui.painter().rect_filled(handle, 2.0, Color32::from_rgb(0xB8, 0xC0, 0xC6));
        ui.painter().line_segment([handle.left_center(), handle.right_center()], Stroke::new(1.0_f32, PANEL_DEEP));
        if fr.hovered() || fr.dragged() {
            let v = p.mixer[i].volume;
            let db = if v <= 0.0001 { f32::NEG_INFINITY } else { 20.0 * (v / engine::VOL_SCALE).log10() };
            self.hint = format!("{} volume {:.1} dB - right-click to automate", p.mixer[i].name, db);
        }
        self.automate_menu(&fr, p, Target::MixerVolume(i));
        let num = if i == 0 { "M".to_string() } else { i.to_string() };
        ui.painter().text(pos2(r.center().x, r.bottom() - 10.0), Align2::CENTER_CENTER, num, FontId::monospace(10.0), TEXT_DIM);

        let click = ui.interact(Rect::from_min_max(r.min, pos2(r.right(), r.top() + 18.0)), ui.id().with(("strip", i)), Sense::click());
        if click.clicked() || fr.drag_started() || pr.clicked() {
            if self.selected_insert != i {
                self.selected_slot = None;
            }
            self.selected_insert = i;
        }
    }
}

#[derive(Clone, Copy)]
enum NoteTool {
    Quantize,
    Legato,
    Chop,
    Reverse,
    OctaveUp,
    OctaveDown,
    FlipKeys,
    Arpeggiate,
    Humanize,
}
