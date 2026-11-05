# Où on en est — 5 novembre 2026

## Fait

- Base : Channel rack, Piano roll, Playlist, Mixer, tempo 10–522 BPM, valeurs tirées de FL Studio (voir `MECHANICS.md`).
- Projets : enregistrer, ouvrir, sauvegarde automatique, modèles, deux démos.
- Exports : WAV, MP3 (bibliothèque LAME du système), MIDI. Import MIDI.
- Jeu en direct : métronome, tempo au tap, clavier d'ordinateur en piano, clavier MIDI, enregistrement des notes.
- Annuler / rétablir (Ctrl+Z / Ctrl+Y).
- Piano roll : sélection, copier / couper / coller / dupliquer, flèches, couloir de vélocité, menu Tools (Quantize, Legato, Chop, Reverse, octave, Flip keys, Arpeggiate, Humanize, Snap to scale, Chords, Crescendo, Decrescendo).
- Motifs : supprimer un motif (PATTERNS) ; Playlist : Ctrl+glisser copie un bloc.
- Channel rack : sous-menu Steps (remplir un pas sur 1, 2, 4 ou 8, décaler, pas au hasard).
- Mixer : solo, envois entre pistes, 10 emplacements d'effets, 8 effets intégrés (Reverb, Delay, EQ, Compressor, Filter, Distortion, Chorus, Phaser).
- Plugins CLAP (instruments et effets) : 239 trouvés sur cette machine, réglages en curseurs, fenêtre propre du plugin (X11), état gardé dans le projet.
- Sons du disque (WAV, MP3, FLAC, OGG), fichiers déposés sur la fenêtre.
- Automation (tempo, volumes, panoramiques, réglages des effets) en mode SONG, et en mode PAT à l'endroit où le motif est posé ; points ou dessin à main levée (Maj+glisser).
- Enregistrement audio depuis le micro.
- Browser : tous les dossiers fonctionnent.
- 53 tests passent, plus 3 avec les vrais plugins et leurs fenêtres (`cargo test -- --include-ignored`), `cargo clippy` sans avertissement.

## Pas encore vérifié à la main

Ces fonctions compilent et leur logique est testée, mais personne n'a encore cliqué dedans :

- sélection et copier-coller dans le Piano roll, couloir de vélocité, menu Tools ;
- points d'automation et dessin à main levée dans la Playlist, automation en mode PAT ;
- fenêtre des paramètres d'un plugin, et le bouton « Open plugin window » depuis l'application (la fenêtre elle-même est vérifiée par le test) ;
- prise réelle au micro.

## Limites connues

- Pas de VST, seulement CLAP. La fenêtre propre d'un plugin passe par X11 (XWayland sous Wayland) ; vérifiée sur 12 plugins (Dexed, Lemon…) par `cargo test real_plugin_windows -- --include-ignored`.
- Un son du disque est joué en entier pour une note d'un pas, coupé à la fin pour une note plus longue.
- L'automation est sur des couloirs en haut de la Playlist, pas sur les pistes. En mode PAT, un motif absent de la Playlist n'a pas d'automation.

## Pour reprendre

- Dépôt GitHub privé : `devfrp/beat-studio`, à jour au 1er novembre 2026.
- Le dossier `target` est un lien vers `/tmp/ca_target`, vidé au redémarrage : refaire `mkdir -p /tmp/ca_target` avant de compiler, et la première compilation est plus longue.
- Prochaine étape conseillée : essayer l'application (`cargo run`) et noter ce qui ne marche pas.
