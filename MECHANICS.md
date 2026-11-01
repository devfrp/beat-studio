# Mechanics sheet

Where the values come from: the target `flstudio_win64_26.1.7.5654.exe` is an NSIS installer
(stub code in `sub_40355d` / `entry` 0x4039c5 only verifies and unpacks). Its compressed script
header (LZMA, at file offset 0x10a24) was decoded to find the real files and their data offsets:

| file inside the installer | data offset | what it is |
|---|---|---|
| `FLEngine_x64.dll` | 2565779 | the program itself (22 MB, x86-64, Delphi) — decompiled below |
| `FL64.exe` | 9206554 | small launcher |
| `Basic with limiter.flp` | 507967520 | the default "new project" template |
| `Artwork\Skins\Default.txt` | 31090867 | the default skin colours |

Ghidra could not run (disk full), so functions were read with the built-in decompiler
(`native_function_pseudocode`) and, where its function list stopped (50 000 functions),
with a plain disassembly. All addresses are virtual addresses in `FLEngine_x64.dll`.

## Tempo (decompiled)

| rule | value | function |
|---|---|---|
| tempo is stored as an integer, BPM × 1000 | multiplier 1000.0f at 0xd62aa4 | `sub_d62a00` |
| stored tempo must be 10000 … 522000, otherwise error −3 | `cmp 0x2710` / `cmp 0x7f710` | `sub_d62a00` |
| accepted tempo is sent as setting 0x40000005 | `sub_1046a70(0x40000005, tempo, …)` | `sub_d62a00` → `sub_1046a70` |
| setting 0x40000005 writes the global tempo (0x15e28c8) and re-times playback | `sub_13114f0`, `sub_1311510` | `sub_1046a70` case 0x40000005 |
| read tempo: BPM = stored / 1000.0 | double 1000.0 at 0x1311508 | `sub_13114f0` |
| BPM clamp | min 10.0 (0x1311990), max 522.0 (0x1311998) | `sub_1311510` |
| ticks per second = BPM × PPQ / 4.0, then samples per tick = sample rate × 15 / that | 4.0f at 0x13119a8, `×15` (`shl 4; sub`) | `sub_1311510` |
| MIDI tempo = 60 000 000 / BPM (µs per beat) | 60000000.0f at 0x13119ac | `sub_1311510` |

Rebuild: `quantize_tempo` rounds to 1/1000 BPM and clamps to 10000…522000; the sequencer
advances `ticks_per_sample = (BPM × PPQ / 4) / (rate × 15)`.

## Volume and pan (decompiled)

| rule | value | function |
|---|---|---|
| channel/mixer volume and pan settings use a 12800-step scale | `0x3200` passed to the setters `sub_11f2330`, `sub_11f1b60`, `sub_11f1e20` | `sub_1046a70` |
| gain = value / 12800.0 | double 12800.0 at 0x104cd48 | `sub_1046a70` (insert volume branch) |
| pan is signed; +6400 (0x1900) moves it to the 0…12800 knob range, centre 6400 | `r8d += 0x1900` | `sub_1046a70` |
| 0x104cd10 constant table | 1000.0, 2⁻³⁰, 2400.0, 1.0, 192.0, 2⁻¹⁴, 12800.0, 20.0, 64.0, 100.0 | used by `sub_1046a70` |
| mixer events pack slot in 6 bits and track above it: id = 0x70000000 + ((track << 6 \| slot) << 16) + param | `>> 16`, `>> 6`, `& 0x3f` | `sub_1046a70` |

## Project defaults (template `Basic with limiter.flp`)

| value | number | source |
|---|---|---|
| saved by version | 26.1.0.5528 | event 199 |
| pulses per quarter note | 96 | FLhd header |
| tempo | 130.000 BPM | event 156 = 130000 (BPM × 1000, same unit as `sub_d62a00`) |
| main swing | 0 | event 11 = 0 |
| main pitch | 0 cents | event 80 = 0 |
| time signature | 4 / 4 | events 17 = 4, 18 = 4 |
| current pattern | 1 | event 67 = 1 |
| channels | 808 Kick, 808 Clap, 808 HiHat, 808 Snare (samplers), 808 Astronomic (FLEX synth) | events 64 / 203 / 201 |
| channel volume | 10000 (of 12800) | event 219 bytes 4–7 |
| channel pan | 6400 (of 12800) = centre | event 219 bytes 0–3 |
| mixer insert volume | 12800 = gain 1.0 = 0 dB | event 225 param 0xC0 (18 saved tracks) |
| mixer insert pan | 0 (signed) = centre | event 225 param 0xC1 |
| notes in pattern 1 | none (empty) | no event 224 in the file |

## Timing rules

* One step = a sixteenth note = 96 / 4 = 24 ticks.
* Steps per pattern bar = 4 beats × 4 steps = 16.
* Swing (estimated formula, looked for but not found in code): every second step is delayed by `swing × 0.5 × step length`.
* A pattern lasts at least one bar; it grows by whole bars to fit its last note (estimated).

## Look (skin `Default.txt`)

* Background gradient `$FF6B747C` → `$FF575C64`, read as RGB #6B747C → #575C64 (byte order estimated).
* Panel, step-button and text colours are estimated: the engine's colour table was not found.

## Still estimated (looked for, not found)

* Mixer size: Master + 125 inserts. The template saves 18 tracks; no count constant found in `sub_1046a70`.
* Fader top: 16000 (= +1.9 dB).
* New-note velocity 100 / 128.
* Instrument sounds: synthesised stand-ins (kick, clap, hi-hat, snare, simple synth), not the original samples.
