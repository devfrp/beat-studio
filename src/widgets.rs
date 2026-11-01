// Colours and small drawing helpers shared by the windows.

use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Painter, Rect, Rounding, Sense, Stroke, Ui};

// Background gradient from the default skin; the rest of the palette is estimated.
pub const BG_TOP: Color32 = Color32::from_rgb(0x6B, 0x74, 0x7C);
pub const BG_BOTTOM: Color32 = Color32::from_rgb(0x57, 0x5C, 0x64);
pub const PANEL: Color32 = Color32::from_rgb(0x3B, 0x42, 0x48);
pub const PANEL_ALT: Color32 = Color32::from_rgb(0x37, 0x3E, 0x44);
pub const PANEL_DARK: Color32 = Color32::from_rgb(0x2D, 0x33, 0x38);
pub const PANEL_DEEP: Color32 = Color32::from_rgb(0x23, 0x28, 0x2C);
pub const HEADER: Color32 = Color32::from_rgb(0x4B, 0x53, 0x5A);
pub const HOVER: Color32 = Color32::from_rgb(0x5C, 0x66, 0x6E);
pub const LINE: Color32 = Color32::from_rgb(0x1E, 0x22, 0x26);
pub const LINE_FAINT: Color32 = Color32::from_rgb(0x33, 0x39, 0x3E);
pub const TEXT: Color32 = Color32::from_rgb(0xD2, 0xD8, 0xDC);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8E, 0x98, 0x9F);
pub const ACCENT: Color32 = Color32::from_rgb(0xF4, 0x9E, 0x3C);
pub const LED_ON: Color32 = Color32::from_rgb(0x8C, 0xE0, 0x52);
pub const LED_OFF: Color32 = Color32::from_rgb(0x46, 0x4E, 0x46);
pub const LCD: Color32 = Color32::from_rgb(0x1A, 0x1F, 0x22);
pub const LCD_TEXT: Color32 = Color32::from_rgb(0xB8, 0xF0, 0x8A);
pub const STEP_A: Color32 = Color32::from_rgb(0x5C, 0x63, 0x6A);
pub const STEP_B: Color32 = Color32::from_rgb(0x6E, 0x52, 0x55);
pub const STEP_ON: Color32 = Color32::from_rgb(0xE6, 0xEA, 0xEC);
pub const NOTE: Color32 = Color32::from_rgb(0x7F, 0xD0, 0x8A);
pub const NOTE_EDGE: Color32 = Color32::from_rgb(0x2E, 0x5A, 0x36);
pub const RED: Color32 = Color32::from_rgb(0xF0, 0x50, 0x48);

pub fn apply_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.override_text_color = Some(TEXT);
    v.panel_fill = PANEL_DARK;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_rounding = Rounding::same(4.0);
    v.window_shadow = egui::epaint::Shadow { offset: vec2(0.0, 4.0), blur: 12.0, spread: 0.0, color: Color32::from_black_alpha(90) };
    v.extreme_bg_color = LCD;
    v.faint_bg_color = PANEL_DARK;
    v.selection.bg_fill = ACCENT.gamma_multiply(0.6);
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.inactive.bg_fill = HEADER;
    v.widgets.inactive.weak_bg_fill = HEADER;
    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.active.bg_fill = ACCENT.gamma_multiply(0.7);
    v.widgets.active.weak_bg_fill = ACCENT.gamma_multiply(0.7);
    style.spacing.item_spacing = vec2(6.0, 3.0);
    style.spacing.button_padding = vec2(6.0, 2.0);
    ctx.set_style(style);
}

pub fn gradient(painter: &Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(egui::Shape::mesh(mesh));
}

pub fn pattern_color(i: usize) -> Color32 {
    let h = (0.36 + i as f32 * 0.137).fract();
    egui::ecolor::Hsva::new(h, 0.42, 0.80, 1.0).into()
}

pub fn key_name(k: u8) -> String {
    const N: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", N[(k % 12) as usize], k / 12)
}

/// A round knob: drag up/down to change, double-click to reset.
pub fn knob(ui: &mut Ui, value: &mut f32, min: f32, max: f32, default: f32, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click_and_drag());
    if resp.dragged() {
        let d = -resp.drag_delta().y + resp.drag_delta().x * 0.5;
        *value = (*value + d * (max - min) / 140.0).clamp(min, max);
    }
    if resp.double_clicked() {
        *value = default;
    }
    let painter = ui.painter();
    let c = rect.center();
    let rad = size * 0.5 - 1.0;
    painter.circle_filled(c, rad, PANEL_DEEP);
    painter.circle_stroke(c, rad, Stroke::new(1.0, Color32::from_rgb(0x60, 0x6A, 0x72)));
    let frac = (*value - min) / (max - min);
    let ang = (-135.0f32 + 270.0 * frac).to_radians();
    let dir = vec2(ang.sin(), -ang.cos());
    let col = if resp.hovered() || resp.dragged() { ACCENT } else { Color32::from_rgb(0xC8, 0xD0, 0xD4) };
    painter.line_segment([c + dir * rad * 0.25, c + dir * (rad - 1.5)], Stroke::new(2.0, col));
    resp
}

pub fn led(ui: &mut Ui, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(12.0, 22.0), Sense::click());
    let r = Rect::from_center_size(rect.center(), vec2(8.0, 8.0));
    ui.painter().rect_filled(r, 2.0, if on { LED_ON } else { LED_OFF });
    resp
}

pub fn led_button(ui: &mut Ui, label: &str, on: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(11.0), TEXT);
    let size = vec2(galley.size().x + 22.0, 20.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() { HOVER } else { PANEL_DARK };
    ui.painter().rect_filled(rect, 3.0, fill);
    ui.painter().circle_filled(pos2(rect.left() + 8.0, rect.center().y), 3.5, if on { LED_ON } else { LED_OFF });
    ui.painter().galley(pos2(rect.left() + 15.0, rect.center().y - galley.size().y / 2.0), galley, TEXT);
    resp
}

pub fn big_button(ui: &mut Ui, label: &str, lit: Option<Color32>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 26.0), Sense::click());
    let fill = if resp.hovered() { HOVER } else { PANEL_DARK };
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, fill);
    let col = lit.unwrap_or(TEXT);
    let c = rect.center();
    match label {
        "play" => {
            let pts = vec![c + vec2(-5.0, -7.0), c + vec2(7.0, 0.0), c + vec2(-5.0, 7.0)];
            painter.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
        }
        "pause" => {
            painter.rect_filled(Rect::from_center_size(c - vec2(3.5, 0.0), vec2(4.0, 13.0)), 1.0, col);
            painter.rect_filled(Rect::from_center_size(c + vec2(3.5, 0.0), vec2(4.0, 13.0)), 1.0, col);
        }
        "stop" => {
            painter.rect_filled(Rect::from_center_size(c, vec2(12.0, 12.0)), 1.0, col);
        }
        "record" => {
            painter.circle_filled(c, 6.5, col);
        }
        _ => {
            painter.text(c, Align2::CENTER_CENTER, label, FontId::proportional(15.0), col);
        }
    }
    resp
}

pub fn lcd(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::none().fill(LCD).rounding(3.0).inner_margin(egui::Margin::symmetric(6.0, 2.0)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.visuals_mut().override_text_color = Some(LCD_TEXT);
            ui.style_mut().override_font_id = Some(FontId::monospace(16.0));
            ui.visuals_mut().widgets.inactive.weak_bg_fill = LCD;
            ui.visuals_mut().widgets.inactive.bg_fill = LCD;
            add(ui);
        });
    });
}

pub fn lcd_small(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::none().fill(LCD).rounding(2.0).inner_margin(egui::Margin::symmetric(2.0, 1.0)).show(ui, |ui| {
        ui.set_width(24.0);
        ui.visuals_mut().override_text_color = Some(LCD_TEXT);
        ui.style_mut().override_font_id = Some(FontId::monospace(11.0));
        ui.visuals_mut().widgets.inactive.weak_bg_fill = LCD;
        add(ui);
    });
}

pub fn folder(ui: &mut Ui, name: &str, open: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 2.0, PANEL);
    }
    let icon = Rect::from_min_size(pos2(rect.left() + 4.0, rect.center().y - 5.0), vec2(13.0, 10.0));
    ui.painter().rect_filled(icon, 1.5, Color32::from_rgb(0xC9, 0xA4, 0x5E));
    let col = if open { TEXT } else { Color32::from_rgb(0xB4, 0xBC, 0xC2) };
    ui.painter().text(pos2(rect.left() + 22.0, rect.center().y), Align2::LEFT_CENTER, name, FontId::proportional(12.0), col);
    resp
}

pub fn file_item(ui: &mut Ui, name: &str, sel: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::click());
    if sel || resp.hovered() {
        ui.painter().rect_filled(rect, 2.0, if sel { HEADER } else { PANEL });
    }
    ui.painter().circle_filled(pos2(rect.left() + 6.0, rect.center().y), 2.5, if sel { ACCENT } else { TEXT_DIM });
    ui.painter().text(pos2(rect.left() + 14.0, rect.center().y), Align2::LEFT_CENTER, name, FontId::proportional(11.0), TEXT);
    resp
}

fn meter_level(x: f32) -> f32 {
    if x <= 0.0001 {
        0.0
    } else {
        ((20.0 * x.log10() + 48.0) / 54.0).clamp(0.0, 1.0)
    }
}

fn meter_color(f: f32) -> Color32 {
    if f > 0.9 {
        RED
    } else if f > 0.75 {
        Color32::from_rgb(0xF0, 0xC0, 0x40)
    } else {
        LED_ON
    }
}

pub fn meter_h(painter: &Painter, rect: Rect, peak: [f32; 2]) {
    painter.rect_filled(rect, 2.0, LCD);
    let h = (rect.height() - 4.0) / 2.0;
    for (k, pk) in peak.iter().enumerate() {
        let f = meter_level(*pk);
        let r = Rect::from_min_size(pos2(rect.left() + 2.0, rect.top() + 2.0 + k as f32 * (h + 1.0)), vec2((rect.width() - 4.0) * f, h - 1.0));
        painter.rect_filled(r, 1.0, meter_color(f));
    }
}

pub fn meter_v(painter: &Painter, rect: Rect, peak: [f32; 2]) {
    painter.rect_filled(rect, 2.0, LCD);
    let w = (rect.width() - 3.0) / 2.0;
    for (k, pk) in peak.iter().enumerate() {
        let f = meter_level(*pk);
        let hh = (rect.height() - 4.0) * f;
        let x = rect.left() + 1.0 + k as f32 * (w + 1.0);
        let r = Rect::from_min_max(pos2(x, rect.bottom() - 2.0 - hh), pos2(x + w, rect.bottom() - 2.0));
        painter.rect_filled(r, 1.0, meter_color(f));
    }
}

pub fn save_png(path: &std::path::Path, image: &egui::ColorImage) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let file = std::fs::File::create(path)?;
    let w = std::io::BufWriter::new(file);
    let mut enc = png::Encoder::new(w, image.size[0] as u32, image.size[1] as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()?;
    let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    writer.write_image_data(&bytes)?;
    Ok(())
}
