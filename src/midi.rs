// MIDI keyboard input. midir calls back on its own thread; the notes go
// through a channel and the interface reads them once per frame.

use std::sync::mpsc::{channel, Receiver};

use midir::{Ignore, MidiInput, MidiInputConnection};

const CLIENT: &str = "Beat Studio";

/// Settings value: connect to the first keyboard found.
pub const AUTO: &str = "auto";
/// Settings value: no MIDI input.
pub const OFF: &str = "off";

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MidiEvent {
    On { key: u8, vel: u8 },
    Off { key: u8 },
}

/// Reads a note message; a note-on with velocity 0 is a note-off.
pub fn parse(msg: &[u8]) -> Option<MidiEvent> {
    let (&status, data) = msg.split_first()?;
    let (key, vel) = (*data.first()?, *data.get(1)?);
    match status & 0xF0 {
        0x90 if vel > 0 => Some(MidiEvent::On { key, vel }),
        0x80 | 0x90 => Some(MidiEvent::Off { key }),
        _ => None,
    }
}

/// Names of the MIDI inputs, without the loopback port that every Linux system has.
pub fn input_ports() -> Vec<String> {
    let Ok(input) = MidiInput::new(CLIENT) else { return Vec::new() };
    input
        .ports()
        .iter()
        .filter_map(|p| input.port_name(p).ok())
        .filter(|n| !n.starts_with("Midi Through"))
        .collect()
}

pub struct MidiIn {
    _conn: MidiInputConnection<()>,
    pub name: String,
    events: Receiver<MidiEvent>,
}

impl MidiIn {
    /// Opens the input named `want`, or the first one for `AUTO`.
    pub fn open(want: &str) -> Option<MidiIn> {
        let mut input = MidiInput::new(CLIENT).ok()?;
        input.ignore(Ignore::All);
        let port = input.ports().into_iter().find(|p| {
            input.port_name(p).is_ok_and(|n| if want == AUTO { !n.starts_with("Midi Through") } else { n == want })
        })?;
        let name = input.port_name(&port).ok()?;
        let (tx, rx) = channel();
        let conn = input
            .connect(&port, "input", move |_, msg, _| {
                if let Some(e) = parse(msg) {
                    let _ = tx.send(e);
                }
            }, ())
            .ok()?;
        Some(MidiIn { _conn: conn, name, events: rx })
    }

    pub fn events(&self) -> Vec<MidiEvent> {
        self.events.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_note_messages() {
        assert_eq!(parse(&[0x90, 60, 100]), Some(MidiEvent::On { key: 60, vel: 100 }));
        assert_eq!(parse(&[0x93, 61, 0]), Some(MidiEvent::Off { key: 61 }));
        assert_eq!(parse(&[0x80, 62, 64]), Some(MidiEvent::Off { key: 62 }));
        assert_eq!(parse(&[0xB0, 7, 100]), None); // control change
        assert_eq!(parse(&[0x90, 60]), None);
    }
}
