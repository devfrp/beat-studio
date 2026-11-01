// Channel rack, piano roll and mixer windows.

use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui};

use crate::engine::{self, Instrument, Mode, Note, Project, MIXER_INSERTS, NOTE_VELOCITY, STEPS_PER_BAR, TICKS_PER_BAR, TICKS_PER_STEP};
use crate::ui::{DragKind, PianoDrag, StudioApp};
use crate::widgets::*;

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
        let playing_step = (p.playing && p.mode == Mode::Pattern).then(|| (p.position as u32 / TICKS_PER_STEP) as usize);

        let mut remove: Option<usize> = None;
        for ch in 0..p.channels.len() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                let c = &mut p.channels[ch];
                if led(ui, c.enabled).on_hover_text("Mute / unmute").clicked() {
                    c.enabled = !c.enabled;
                }
                if knob(ui, &mut c.pan, -engine::PAN_CENTRE, engine::PAN_CENTRE, 0.0, 18.0).hovered() {
                    self.hint = format!("{} pan {:+.0}%", c.name, c.pan / engine::PAN_CENTRE * 100.0);
                }
                if knob(ui, &mut c.volume, 0.0, engine::VOL_SCALE, engine::CHANNEL_VOLUME, 18.0).hovered() {
                    self.hint = format!("{} volume {:.0}%", c.name, c.volume / engine::VOL_SCALE * 100.0);
                }
                let mut track = c.mixer;
                lcd_small(ui, |ui| {
                    ui.add(egui::DragValue::new(&mut track).range(0..=MIXER_INSERTS).speed(0.1)).on_hover_text("Mixer track");
                });
                c.mixer = track;

                let sel = ch == self.selected_channel;
                let (r, resp) = ui.allocate_exact_size(vec2(118.0, 22.0), Sense::click());
                ui.painter().rect_filled(r, 2.0, if sel { Color32::from_rgb(0x8A, 0x94, 0x9B) } else { HEADER });
                let col = if sel { PANEL_DEEP } else { TEXT };
                ui.painter().text(r.left_center() + vec2(6.0, 0.0), Align2::LEFT_CENTER, &c.name, FontId::proportional(12.0), col);
                if resp.clicked() {
                    self.selected_channel = ch;
                    p.previews.push((ch, engine::STEP_KEY));
                }
                if resp.double_clicked() {
                    self.selected_channel = ch;
                    self.show_piano = true;
                }
                resp.context_menu(|ui| {
                    if ui.button("Piano roll").clicked() {
                        self.selected_channel = ch;
                        self.show_piano = true;
                        ui.close_menu();
                    }
                    if ui.button("Clear").clicked() {
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
            self.selected_channel = self.selected_channel.min(p.channels.len() - 1);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.menu_button("  +  ", |ui| {
                for inst in Instrument::ALL {
                    if ui.button(inst.label()).clicked() {
                        p.add_channel(inst);
                        self.selected_channel = p.channels.len() - 1;
                        ui.close_menu();
                    }
                }
            })
            .response
            .on_hover_text("Add a channel");
            ui.label(egui::RichText::new("click or drag across steps · double-click a name for the piano roll").color(TEXT_DIM).size(10.0));
        });
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
            .default_size(vec2(760.0, 380.0))
            .resizable(true)
            .show(ctx, |ui| self.piano_ui(ui, p, ch));
        if !open {
            self.show_piano = false;
        }
    }

    fn piano_ui(&mut self, ui: &mut Ui, p: &mut Project, ch: usize) {
        let pat = p.current_pattern;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&p.patterns[pat].name).color(ACCENT));
            ui.label(egui::RichText::new("Snap: step").color(TEXT_DIM).size(11.0));
            ui.label(egui::RichText::new(format!("New note: {} steps", self.last_note_len / TICKS_PER_STEP)).color(TEXT_DIM).size(11.0));
            if ui.small_button("Clear").clicked() {
                p.patterns[pat].notes[ch].clear();
            }
            ui.label(egui::RichText::new("click: add · drag: move · drag the end: resize · right-click: delete").color(TEXT_DIM).size(10.0));
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
        let (rect, _) = ui.allocate_exact_size(vec2(width, keys as f32 * key_h), Sense::hover());
        let painter = ui.painter_at(ui.clip_rect());
        let grid = Rect::from_min_max(pos2(rect.left() + kb_w, rect.top()), rect.max);
        let key_y = |k: u8| grid.top() + (keys as f32 - 1.0 - k as f32) * key_h;
        let black = |k: u8| matches!(k % 12, 1 | 3 | 6 | 8 | 10);
        let tick_x = |t: u32| grid.left() + t as f32 / TICKS_PER_STEP as f32 * step_w;

        for k in 0..keys as u8 {
            let row = Rect::from_min_size(pos2(grid.left(), key_y(k)), vec2(grid.width(), key_h));
            painter.rect_filled(row, 0.0, if black(k) { PANEL_DARK } else { PANEL });
            if k % 12 == 0 {
                painter.line_segment([row.left_bottom(), row.right_bottom()], Stroke::new(1.0, LINE));
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
            painter.line_segment([pos2(x, grid.top()), pos2(x, grid.bottom())], Stroke::new(1.0, col));
        }
        let end_x = tick_x(p.patterns[pat].length_ticks());
        painter.rect_filled(Rect::from_min_max(pos2(end_x, grid.top()), grid.max), 0.0, Color32::from_black_alpha(50));

        let hover = ui.input(|i| i.pointer.hover_pos());
        let mut hit: Option<(usize, DragKind)> = None;
        for (i, n) in p.patterns[pat].notes[ch].iter().enumerate() {
            if n.vel == 0 {
                continue;
            }
            let r = Rect::from_min_max(pos2(tick_x(n.start), key_y(n.key)), pos2(tick_x(n.start + n.len), key_y(n.key) + key_h)).shrink(0.5);
            painter.rect_filled(r, 2.0, NOTE);
            painter.rect_stroke(r, 2.0, Stroke::new(1.0, NOTE_EDGE));
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
            painter.line_segment([pos2(x, grid.top()), pos2(x, grid.bottom())], Stroke::new(1.5, ACCENT));
        }

        // Keyboard pinned to the left edge; click a key to hear it.
        let vis = ui.clip_rect();
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

        let area = Rect::from_min_max(pos2(vis.left() + kb_w, grid.top()), pos2(vis.right(), grid.bottom()));
        let gr = ui.interact(area, ui.id().with("notes"), Sense::click_and_drag());
        let at = |pp: Pos2| {
            let t = ((pp.x - grid.left()) / step_w * TICKS_PER_STEP as f32).max(0.0) as i64;
            let k = (keys as f32 - 1.0 - ((pp.y - grid.top()) / key_h).floor()).clamp(0.0, keys as f32 - 1.0) as i32;
            (t, k)
        };
        let snap = |t: i64| (t / TICKS_PER_STEP as i64) * TICKS_PER_STEP as i64;
        if gr.secondary_clicked() {
            if let Some((i, _)) = hit.take() {
                p.patterns[pat].notes[ch].remove(i);
            }
        }
        let start_edit = gr.drag_started_by(egui::PointerButton::Primary) || gr.clicked();
        if start_edit && self.piano_drag.is_none() {
            if let Some(pp) = gr.interact_pointer_pos() {
                let (t, k) = at(pp);
                let list = &mut p.patterns[pat].notes[ch];
                let (index, kind) = match hit {
                    Some(h) => h,
                    None => {
                        let n = Note { key: k as u8, start: snap(t) as u32, len: self.last_note_len, vel: NOTE_VELOCITY };
                        list.push(n);
                        p.previews.push((ch, n.key));
                        (list.len() - 1, DragKind::Move)
                    }
                };
                let orig = p.patterns[pat].notes[ch][index];
                self.piano_drag = Some(PianoDrag { index, kind, grab_ticks: t - orig.start as i64, grab_key: k - orig.key as i32, orig });
            }
        }
        if let (Some(d), Some(pp)) = (self.piano_drag, gr.interact_pointer_pos()) {
            let list = &mut p.patterns[pat].notes[ch];
            if d.index < list.len() {
                let (t, k) = at(pp);
                let n = &mut list[d.index];
                match d.kind {
                    DragKind::Move => {
                        n.start = snap((t - d.grab_ticks).max(0)) as u32;
                        n.key = (k - d.grab_key).clamp(0, keys as i32 - 1) as u8;
                    }
                    DragKind::Resize => {
                        let end = snap(t + TICKS_PER_STEP as i64 / 2).max(d.orig.start as i64 + TICKS_PER_STEP as i64);
                        n.len = (end - d.orig.start as i64) as u32;
                    }
                }
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
                Some(_) => "Drag to move the note, right-click to delete".into(),
                None => "Click to add a note".into(),
            };
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
            .default_size(vec2(900.0, 320.0))
            .resizable(true)
            .show(ctx, |ui| self.mixer_ui(ui, p));
        if !open {
            self.show_mixer = false;
        }
    }

    fn mixer_ui(&mut self, ui: &mut Ui, p: &mut Project) {
        let strip_w = 50.0;
        let strip_h = 270.0;
        egui::SidePanel::right("insert_detail")
            .resizable(false)
            .exact_width(170.0)
            .frame(egui::Frame::none().fill(PANEL_DARK).inner_margin(6.0))
            .show_inside(ui, |ui| {
                ui.label(egui::RichText::new(&p.mixer[self.selected_insert].name).color(ACCENT).strong());
                let routed: Vec<String> = p.channels.iter().filter(|c| c.mixer == self.selected_insert).map(|c| c.name.clone()).collect();
                let from = if routed.is_empty() { "No channels routed".to_string() } else { format!("From: {}", routed.join(", ")) };
                ui.label(egui::RichText::new(from).color(TEXT_DIM).size(10.0));
                ui.separator();
                for s in 1..=10 {
                    egui::Frame::none().fill(PANEL_DEEP).rounding(2.0).inner_margin(3.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(egui::RichText::new(format!("Slot {s}   (empty)")).color(TEXT_DIM).size(10.0));
                    });
                }
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

    fn strip(&mut self, ui: &mut Ui, p: &mut Project, i: usize, r: Rect) {
        let sel = i == self.selected_insert;
        let used = i == 0 || p.channels.iter().any(|c| c.mixer == i);
        let painter = ui.painter();
        painter.rect_filled(r, 2.0, if sel { Color32::from_rgb(0x4E, 0x58, 0x60) } else { PANEL_DARK });
        if sel {
            painter.rect_stroke(r, 2.0, Stroke::new(1.0, ACCENT));
        }
        let label = if i == 0 { "Master".to_string() } else { format!("Insert {i}") };
        painter.text(pos2(r.center().x, r.top() + 9.0), Align2::CENTER_CENTER, label, FontId::proportional(9.5), if used { TEXT } else { TEXT_DIM });

        let enabled = p.mixer[i].enabled;
        let led_r = Rect::from_center_size(pos2(r.center().x, r.top() + 24.0), vec2(12.0, 8.0));
        painter.rect_filled(led_r, 2.0, if enabled { LED_ON } else { LED_OFF });
        if ui.interact(led_r.expand(3.0), ui.id().with(("mute", i)), Sense::click()).clicked() {
            p.mixer[i].enabled = !enabled;
        }

        let kr = Rect::from_center_size(pos2(r.center().x, r.top() + 44.0), vec2(20.0, 20.0));
        let mut pan = p.mixer[i].pan;
        let pr = ui.put(kr, |ui: &mut Ui| knob(ui, &mut pan, -engine::PAN_CENTRE, engine::PAN_CENTRE, 0.0, 18.0));
        if pr.hovered() {
            self.hint = format!("{} pan {:+.0}%", p.mixer[i].name, pan / engine::PAN_CENTRE * 100.0);
        }
        p.mixer[i].pan = pan;

        let top = r.top() + 62.0;
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
        ui.painter().line_segment([handle.left_center(), handle.right_center()], Stroke::new(1.0, PANEL_DEEP));
        if fr.hovered() || fr.dragged() {
            let v = p.mixer[i].volume;
            let db = if v <= 0.0001 { f32::NEG_INFINITY } else { 20.0 * (v / engine::VOL_SCALE).log10() };
            self.hint = format!("{} volume {:.1} dB", p.mixer[i].name, db);
        }
        let num = if i == 0 { "M".to_string() } else { i.to_string() };
        ui.painter().text(pos2(r.center().x, r.bottom() - 10.0), Align2::CENTER_CENTER, num, FontId::monospace(10.0), TEXT_DIM);

        let click = ui.interact(Rect::from_min_max(r.min, pos2(r.right(), r.top() + 16.0)), ui.id().with(("strip", i)), Sense::click());
        if click.clicked() || fr.drag_started() || pr.clicked() {
            self.selected_insert = i;
        }
    }
}
