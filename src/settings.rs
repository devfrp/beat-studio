// User settings kept between sessions (~/.config/beatstudio/settings.txt, key = value).

use std::fs;
use std::path::PathBuf;

pub const UI_SCALES: [f32; 6] = [0.8, 0.9, 1.0, 1.1, 1.25, 1.5];
pub const BUFFER_SIZES: [u32; 6] = [128, 256, 512, 1024, 2048, 4096];
pub const AUTOSAVE_CHOICES: [u32; 5] = [0, 1, 5, 10, 30];

#[derive(Clone, PartialEq, Debug)]
pub struct Settings {
    /// Output device name; None = the system default.
    pub device: Option<String>,
    /// Buffer size in frames; None = let the system choose.
    pub buffer: Option<u32>,
    pub ui_scale: f32,
    /// Minutes between automatic backups; 0 = off.
    pub autosave_minutes: u32,
    pub metronome: bool,
    pub typing_piano: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { device: None, buffer: None, ui_scale: 1.0, autosave_minutes: 5, metronome: false, typing_piano: true }
    }
}

fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("beatstudio").join("settings.txt")
}

impl Settings {
    pub fn load() -> Settings {
        fs::read_to_string(path()).map(|t| Settings::from_text(&t)).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, self.to_text())
    }

    fn to_text(&self) -> String {
        format!(
            "device = {}\nbuffer = {}\nui_scale = {}\nautosave_minutes = {}\nmetronome = {}\ntyping_piano = {}\n",
            self.device.as_deref().unwrap_or(""),
            self.buffer.map(|b| b.to_string()).unwrap_or_default(),
            self.ui_scale,
            self.autosave_minutes,
            self.metronome,
            self.typing_piano,
        )
    }

    fn from_text(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "device" => s.device = (!v.is_empty()).then(|| v.to_string()),
                "buffer" => s.buffer = v.parse().ok(),
                "ui_scale" => s.ui_scale = v.parse::<f32>().unwrap_or(1.0).clamp(0.5, 2.0),
                "autosave_minutes" => s.autosave_minutes = v.parse().unwrap_or(5),
                "metronome" => s.metronome = v == "true",
                "typing_piano" => s.typing_piano = v != "false",
                _ => {}
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip() {
        let s = Settings {
            device: Some("USB Audio".into()),
            buffer: Some(256),
            ui_scale: 1.25,
            autosave_minutes: 10,
            metronome: true,
            typing_piano: false,
        };
        assert_eq!(Settings::from_text(&s.to_text()), s);
        assert_eq!(Settings::from_text(&Settings::default().to_text()), Settings::default());
    }
}
