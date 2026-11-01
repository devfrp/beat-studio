// User settings kept between sessions (~/.config/beatstudio/settings.txt, key = value).

use std::fs;
use std::path::PathBuf;

use crate::midi;

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
    /// MIDI input name, or midi::AUTO / midi::OFF.
    pub midi_input: String,
    /// Sound input for audio recording; None = the system default.
    pub input_device: Option<String>,
    /// The record button also records sound from the input.
    pub record_audio: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            device: None,
            buffer: None,
            ui_scale: 1.0,
            autosave_minutes: 5,
            metronome: false,
            typing_piano: true,
            midi_input: midi::AUTO.into(),
            input_device: None,
            record_audio: false,
        }
    }
}

fn path() -> PathBuf {
    config_dir().join("settings.txt")
}

pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("beatstudio")
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
            "device = {}\nbuffer = {}\nui_scale = {}\nautosave_minutes = {}\nmetronome = {}\ntyping_piano = {}\nmidi_input = {}\ninput_device = {}\nrecord_audio = {}\n",
            self.device.as_deref().unwrap_or(""),
            self.buffer.map(|b| b.to_string()).unwrap_or_default(),
            self.ui_scale,
            self.autosave_minutes,
            self.metronome,
            self.typing_piano,
            self.midi_input,
            self.input_device.as_deref().unwrap_or(""),
            self.record_audio,
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
                "midi_input" if !v.is_empty() => s.midi_input = v.to_string(),
                "input_device" => s.input_device = (!v.is_empty()).then(|| v.to_string()),
                "record_audio" => s.record_audio = v == "true",
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
            midi_input: "USB Keys:USB Keys MIDI 1 20:0".into(),
            input_device: Some("Mic".into()),
            record_audio: true,
        };
        assert_eq!(Settings::from_text(&s.to_text()), s);
        assert_eq!(Settings::from_text(&Settings::default().to_text()), Settings::default());
    }
}
