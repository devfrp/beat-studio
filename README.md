# Beat Studio

Un petit studio de musique pour Linux, inspiré de FL Studio : on compose des rythmes pas à pas, on écrit des mélodies, on assemble le tout en morceau, on mixe avec des effets et des plugins CLAP, et on exporte en WAV, MP3 ou MIDI.

Écrit en Rust avec [egui](https://github.com/emilk/egui) pour l'interface et [cpal](https://github.com/RustAudio/cpal) pour le son.

> Beat Studio est un projet indépendant. Il n'est ni affilié à Image-Line ni approuvé par cette société. « FL Studio » est une marque d'Image-Line. Aucun code, son ou image de FL Studio n'est inclus : les sons sont synthétisés par l'application, et seules quelques valeurs de réglage (tempo de départ, résolution des notes, échelle des volumes) reprennent celles de l'original, comme l'explique [MECHANICS.md](MECHANICS.md).

**Version 0.1.0** : première version. Tout est testé automatiquement, mais le programme a encore été peu essayé à la main. Les retours sont les bienvenus.

## Ce qu'on peut faire

- **Channel rack** : une grille de 16 pas par mesure et par instrument (808 Kick, Clap, HiHat, Snare et un synthé au départ). Le clic droit sur un instrument permet aussi de remplir un pas sur 2, 4 ou 8, de décaler les pas ou d'en tirer au hasard.
- **Piano roll** : écrire des notes avec leur vélocité, les choisir, les copier, les coller, les dupliquer. Le menu Tools permet de :
  - quantifier, mettre en legato, découper, inverser, étirer ;
  - transposer d'une octave, caler sur une gamme, faire des accords, des arpèges ou un riff au hasard ;
  - varier les vélocités.

  Les notes des autres instruments s'affichent en pâle, et l'accord des notes choisies est nommé.
- **Playlist** : assembler les motifs en morceau, copier un bloc avec Ctrl+glisser, insérer ou supprimer des mesures. Des courbes d'automation (tempo, volumes, réglages des effets) se placent point par point ou se dessinent à main levée.
- **Mixer** : un Master et 125 pistes, avec coupure, solo, envois entre pistes et dix emplacements d'effets par piste. Huit effets sont intégrés : Reverb, Delay, EQ, Compressor, Filter, Distortion, Chorus et Phaser.
- **Plugins CLAP** : instruments et effets, avec leur propre fenêtre ou des curseurs génériques.
- **Jeu en direct** : clavier d'ordinateur en piano, clavier MIDI, métronome, tempo au tap, enregistrement des notes et du micro.
- **Fichiers** :
  - projets `.bsp`, sauvegarde automatique, modèles ;
  - export en WAV, en MP3, en MIDI, ou un WAV par piste du Mixer ;
  - import de fichiers MIDI et de sons (WAV, MP3, FLAC, OGG) ;
  - glisser-déposer sur la fenêtre.
- **Annuler / rétablir** partout.

## Installation

### Ce qu'il faut sur la machine

| Pour | Paquet (Arch) | Paquet (Debian / Ubuntu) |
|---|---|---|
| Compiler | `rust` (1.87 ou plus récent), `pkgconf` | `cargo` (Rust 1.87 ou plus récent, par exemple via rustup), `pkg-config` |
| Le son et le MIDI | `alsa-lib` (marche aussi avec PipeWire ou PulseAudio par leur couche ALSA) | `libasound2-dev` |
| La fenêtre | `libxkbcommon`, `wayland`, `libgl` (Wayland ou X11) | `libxkbcommon-dev`, `libwayland-dev`, `libgl1-mesa-dev` |
| La fenêtre propre des plugins | `libx11` (X11, ou XWayland sous Wayland) | `libx11-6` |
| L'export MP3 (facultatif) | `lame` | `libmp3lame0` |

Les plugins CLAP sont cherchés dans `~/.clap` et `/usr/lib/clap`.

### Compiler et lancer

```sh
git clone https://github.com/devfrp/beat-studio.git
cd beat-studio
cargo run --release
```

Le binaire se trouve ensuite dans `target/release/beatstudio`. Les [releases](https://github.com/devfrp/beat-studio/releases) proposent aussi un binaire Linux x86_64 déjà compilé.

Au premier lancement, Beat Studio cherche les plugins CLAP installés. Chaque plugin est ouvert dans un processus à part, pour qu'un plugin qui plante ne fasse pas tomber le studio. La liste est gardée dans `~/.config/beatstudio/plugins.txt`.

## Les fenêtres

| Fenêtre | Touche | À quoi elle sert |
|---|---|---|
| Playlist | F5 | Assembler les motifs en morceau ; courbes d'automation en haut |
| Channel rack | F6 | Grille de pas : un rang par instrument |
| Piano roll | F7 | Écrire des notes pour l'instrument choisi |
| Browser | F8 | Motifs, sons, plugins, projets, modèles, démos, enregistrements, exports |
| Mixer | F9 | Volume, panoramique, solo, envois et effets de chaque piste |

## Raccourcis

| Touche | Action |
|---|---|
| Espace | Lecture / pause |
| L | Mode motif (PAT) ou morceau (SONG) |
| + ou = / - | Motif suivant (le crée s'il n'existe pas) / précédent |
| Ctrl+Z, Ctrl+Y (ou Ctrl+Maj+Z) | Annuler, rétablir |
| Ctrl+S, Ctrl+Maj+S, Ctrl+O | Enregistrer, enregistrer sous, ouvrir |
| Ctrl+R | Exporter en WAV |
| Ctrl+M | Métronome |
| Ctrl+A / C / X / V / B, Suppr, flèches | Piano roll (après un clic dedans) : tout choisir, copier, couper, coller, dupliquer, supprimer, déplacer (Maj+↑↓ : une octave) |
| Lettres et chiffres | Jouer des notes sur l'instrument choisi |

À la souris :
- **clic droit** : menus d'un instrument, d'un effet ou de la règle des mesures ; supprimer une note, un bloc ou un point ;
- **Ctrl+glisser** : choisir des notes dans le Piano roll, ou copier un bloc dans la Playlist ;
- **Maj+glisser** : dessiner une courbe d'automation ;
- **double-clic** sur un bouton rond : le remettre à sa valeur de départ.

## Fichiers

Les projets vont dans `~/Documents/Beat Studio`, avec des sous-dossiers :
- `Templates` pour les modèles ;
- `Samples` pour les sons ;
- `Recorded` pour les prises au micro ;
- `Rendered` pour les exports WAV et MP3 ;
- `Scores` pour les fichiers MIDI.

## Limites connues

- Les sons intégrés sont synthétisés, et les effets sont ceux de l'application.
- Plugins CLAP seulement (pas de VST). Leur fenêtre passe par X11 (XWayland sous Wayland).
- L'automation est sur des couloirs en haut de la Playlist, pas sur les pistes.
- Un son du disque joue en entier pour une note d'un pas, et il est coupé à la fin d'une note plus longue.

## Développement

```sh
cargo test                          # les tests
cargo test -- --include-ignored     # avec les vrais plugins CLAP installés
cargo clippy                        # doit rester sans avertissement
cargo run -- --screenshot shots/now.png   # ouvre, capture l'écran et ferme
```

[ETAT.md](ETAT.md) dit ce qui est fait et ce qui reste à vérifier.

## Licence

[MIT](LICENSE).
