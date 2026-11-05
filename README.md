# Beat Studio

A small music studio inspired by FL Studio. Build beats step by step, write melodies, arrange them into a song, mix with effects and CLAP plugins, and export to WAV, MP3 or MIDI.

Written in Rust, with [egui](https://github.com/emilk/egui) for the interface and [cpal](https://github.com/RustAudio/cpal) for sound. Runs on Linux, Windows and macOS.

> Beat Studio is an independent project. It is not affiliated with or endorsed by Image-Line. "FL Studio" is a trademark of Image-Line. No code, sound or artwork from FL Studio is included. The built-in sounds are synthesized by the app, and only a few settings (default tempo, note resolution, volume scale) match the original, as [MECHANICS.md](MECHANICS.md) explains.

**Version 0.1.0**: the first release. The automated tests pass, but the app has seen little hands-on use so far. Feedback and bug reports are welcome.

## Install

### Linux (x86_64)

```sh
curl -fsSL https://raw.githubusercontent.com/devfrp/beat-studio/main/install.sh | sh
```

This puts the AppImage in `~/.local/bin/beatstudio` and adds Beat Studio to your applications menu. Run the same command again to update. To remove it, add `-s -- --uninstall` after `sh`.

You can also download `BeatStudio-x86_64.AppImage` from the [latest release](https://github.com/devfrp/beat-studio/releases/latest), make it executable and run it. AppImages need FUSE 2 (`libfuse2` on Debian/Ubuntu, `fuse2` on Arch). Without it, start the AppImage with `APPIMAGE_EXTRACT_AND_RUN=1`.

### Windows (x86_64)

Download `BeatStudio-windows-x86_64.exe` from the [latest release](https://github.com/devfrp/beat-studio/releases/latest) and run it. The program is not signed, so SmartScreen will warn you. Click "More info", then "Run anyway".

### macOS 11 or later (Apple Silicon and Intel)

Download `BeatStudio-macos-universal.zip` from the [latest release](https://github.com/devfrp/beat-studio/releases/latest), unzip it and move "Beat Studio" to Applications. The app is not signed, so the first time, right-click it and choose "Open". If macOS says the app is damaged, run:

```sh
xattr -dr com.apple.quarantine "/Applications/Beat Studio.app"
```

### Optional extras

- **MP3 export** needs the LAME library: `lame` on Arch, `libmp3lame0` on Debian/Ubuntu, `brew install lame` on macOS, `libmp3lame.dll` next to the program on Windows.
- **CLAP plugins** are found in these folders:
  - Linux: `~/.clap` and `/usr/lib/clap`;
  - macOS: `~/Library/Audio/Plug-Ins/CLAP` and `/Library/Audio/Plug-Ins/CLAP`;
  - Windows: `%COMMONPROGRAMFILES%\CLAP`;
  - plus any folder listed in `CLAP_PATH`.

## Build from source

| You need | Arch | Debian / Ubuntu |
|---|---|---|
| Rust 1.87 or later | `rust` (or rustup) | rustup |
| Sound and MIDI | `alsa-lib` | `libasound2-dev`, `pkg-config` |
| The window (Wayland or X11, OpenGL) | `libxkbcommon`, `wayland`, `libgl` | `libxkbcommon-dev`, `libwayland-dev`, `libgl1-mesa-dev` |
| Plugin windows | `libx11` (X11, or XWayland on Wayland) | `libx11-6` |

On Windows and macOS, Rust alone is enough.

```sh
git clone https://github.com/devfrp/beat-studio.git
cd beat-studio
cargo run --release
```

The first time it starts, Beat Studio looks for installed CLAP plugins. Each plugin is opened in a separate process, so a plugin that crashes cannot take the studio down. The list is kept in the settings folder:
- Linux: `~/.config/beatstudio`;
- macOS: `~/Library/Application Support/beatstudio`;
- Windows: `%APPDATA%\beatstudio`.

## What you can do

- **Channel rack**: a 16-step grid per instrument. You start with an 808 kick, clap, hi-hat and snare, plus a synth. Right-click an instrument to fill every 2, 4 or 8 steps, shift the steps, or get random ones.
- **Piano roll**: write notes with their velocity; select, copy, paste and duplicate them. The Tools menu can:
  - quantize, make legato, chop, reverse or stretch;
  - shift by an octave, snap to a scale, build chords, arpeggiate or generate a random riff;
  - vary velocities, crescendo and decrescendo.

  Other instruments' notes show as faint "ghost" notes, and the chord the selected notes form is named.
- **Playlist**: arrange patterns into a song, copy clips with Ctrl+drag, insert or delete bars. Automation curves (tempo, volumes, effect settings) can be set point by point or drawn freehand.
- **Mixer**: a master track and 125 tracks, with mute, solo, sends between tracks and ten effect slots per track. Eight effects are built in: Reverb, Delay, EQ, Compressor, Filter, Distortion, Chorus and Phaser.
- **CLAP plugins**, both instruments and effects, with their own window:
  - on Linux, embedded in an X11 window;
  - on Windows and macOS, the plugin's own floating window, when it offers one.

  Every plugin also gets generic sliders.
- **Live playing**: computer keyboard as a piano, MIDI keyboard, metronome, tap tempo, recording of notes and of the microphone.
- **Files**:
  - `.bsp` projects, autosave and templates;
  - export to WAV, MP3, MIDI, or one WAV per mixer track;
  - import of MIDI files and sounds (WAV, MP3, FLAC, OGG);
  - drag and drop onto the window.
- **Undo and redo** everywhere.

## Windows

| Window | Key | What it is for |
|---|---|---|
| Playlist | F5 | Arrange patterns into a song; automation lanes at the top |
| Channel rack | F6 | Step grid, one row per instrument |
| Piano roll | F7 | Notes for the selected instrument |
| Browser | F8 | Patterns, sounds, plugins, projects, templates, demos, recordings, exports |
| Mixer | F9 | Volume, pan, solo, sends and effects of each track |

## Shortcuts

| Key | Action |
|---|---|
| Space | Play / pause |
| L | Switch between pattern (PAT) and song (SONG) mode |
| + or = / - | Next pattern (created if needed) / previous pattern |
| Ctrl+Z, Ctrl+Y (or Ctrl+Shift+Z) | Undo, redo |
| Ctrl+S, Ctrl+Shift+S, Ctrl+O | Save, save as, open |
| Ctrl+R | Export to WAV |
| Ctrl+M | Metronome |
| Ctrl+A / C / X / V / B, Delete, arrows | Piano roll (after clicking in it): select all, copy, cut, paste, duplicate, delete, move (Shift+Up/Down: one octave) |
| Letters and digits | Play notes on the selected instrument |

With the mouse:
- **right-click**: menus for an instrument, an effect or the bar ruler; delete a note, a clip or a point;
- **Ctrl+drag**: select notes in the piano roll, or copy a clip in the playlist;
- **Shift+drag**: draw an automation curve;
- **double-click** a knob: reset it.

## Files

Projects go in `~/Documents/Beat Studio`, with these subfolders:
- `Templates` for templates;
- `Samples` for sounds;
- `Recorded` for microphone takes;
- `Rendered` for WAV and MP3 exports;
- `Scores` for MIDI files.

## Known limits

- The built-in sounds are synthesized, and the effects are the app's own.
- CLAP plugins only, no VST. On Windows and macOS, a plugin without a floating window can only be set with sliders.
- Automation lives in lanes at the top of the playlist, not on the tracks.
- A sound file plays in full for a one-step note, and is cut at the end of a longer note.
- The Windows and macOS builds come straight from CI and have not been tried by hand yet.

## Development

```sh
cargo test                                # tests
cargo test -- --include-ignored           # also with the CLAP plugins installed here
cargo clippy                              # must stay free of warnings
cargo run -- --screenshot shots/now.png   # open, take a screenshot, close
```

[ETAT.md](ETAT.md) (in French) lists what is done and what is left to check. Releases are built by GitHub Actions ([release.yml](.github/workflows/release.yml)) when a `v*` tag is pushed.

## License

[MIT](LICENSE).
