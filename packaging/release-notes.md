Beat Studio is a small music studio inspired by FL Studio. It is an independent project, not affiliated with Image-Line.

## Install

| System | File |
|---|---|
| **Linux** (x86_64) | `BeatStudio-x86_64.AppImage`, or in one command: `curl -fsSL https://raw.githubusercontent.com/devfrp/beat-studio/main/install.sh \| sh` |
| **Windows** (x86_64) | `BeatStudio-windows-x86_64.exe` (the program alone), or `BeatStudio-windows-x86_64.zip` (with the README) |
| **macOS** 11 or later (Apple Silicon and Intel) | `BeatStudio-macos-universal.zip`: unzip it and move "Beat Studio" to Applications |

The programs are not signed, so you will see a warning the first time:

- **Windows**: SmartScreen warns you. Click "More info", then "Run anyway".
- **macOS**: right-click the app and choose "Open". If macOS says it is damaged, run `xattr -dr com.apple.quarantine "/Applications/Beat Studio.app"`.
- **Linux**: the AppImage needs FUSE 2 (`libfuse2` or `fuse2`). Without it, start it with `APPIMAGE_EXTRACT_AND_RUN=1`.

`SHA256SUMS.txt` lists the checksums of the files.

## What's inside

- **Channel rack**: a step grid per instrument, which you can fill or shift.
- **Piano roll**:
  - notes with velocity, selection, copy and paste;
  - Tools menu: quantize, legato, arpeggiate, scales, chords, random riff, crescendo…;
  - ghost notes of the other instruments, and the name of the chord the selected notes form.
- **Playlist**:
  - arrange patterns into a song, copy clips, insert or delete bars;
  - automation point by point or drawn freehand.
- **Mixer**:
  - 125 tracks with solo, sends and ten effect slots;
  - 8 built-in effects: Reverb, Delay, EQ, Compressor, Filter, Distortion, Chorus, Phaser.
- **CLAP plugins** (instruments and effects) with their own window:
  - on Linux, in an X11 window;
  - on Windows and macOS, the plugin's floating window, when it has one.
- **VST3 plugins** (instruments and effects), found in the usual VST3 folders, with generic sliders — Dexed opens and sounds.
- **Live playing**:
  - computer keyboard or MIDI keyboard, metronome, tap tempo;
  - recording of notes and of the microphone.
- **Files**:
  - projects, templates, autosave;
  - export to WAV, MP3 (when the LAME library is installed), MIDI, or one WAV per track;
  - import of MIDI files and sounds.

## Good to know

This is an early release. The automated tests pass, and the interface has been checked on Linux with a handful of real plugins. The Windows and macOS builds come straight from CI and nobody has tried them yet. Bug reports are welcome.

MIT license.
