# Où on en est — 5 novembre 2026

## Fait

- Base : Channel rack, Piano roll, Playlist, Mixer, tempo 10–522 BPM, valeurs tirées de FL Studio (voir `MECHANICS.md`).
- Projets : enregistrer, ouvrir, sauvegarde automatique, modèles, deux démos.
- Exports : WAV, MP3 (bibliothèque LAME du système), MIDI, un WAV par piste du Mixer. Import MIDI.
- Jeu en direct : métronome, tempo au tap, clavier d'ordinateur en piano, clavier MIDI, enregistrement des notes.
- Annuler / rétablir (Ctrl+Z / Ctrl+Y).
- Piano roll : sélection, copier / couper / coller / dupliquer, flèches, couloir de vélocité, menu Tools (Quantize, Legato, Chop, Reverse, octave, Flip keys, Arpeggiate, Humanize, Snap to scale, Chords, Crescendo, Decrescendo, Stretch, Remove duplicates, Trim overlaps, Riff), notes fantômes (case Ghosts).
- Motifs : supprimer un motif (PATTERNS) ; Playlist : Ctrl+glisser copie un bloc ; clic droit sur la règle : insérer ou supprimer une mesure.
- Channel rack : sous-menu Steps (remplir un pas sur 1, 2, 4 ou 8, décaler, pas au hasard).
- Mixer : solo, envois entre pistes, 10 emplacements d'effets, 8 effets intégrés (Reverb, Delay, EQ, Compressor, Filter, Distortion, Chorus, Phaser).
- Plugins CLAP (instruments et effets) : 239 trouvés sur cette machine, réglages en curseurs, fenêtre propre du plugin (X11), état gardé dans le projet.
- Sons du disque (WAV, MP3, FLAC, OGG), fichiers déposés sur la fenêtre.
- Automation (tempo, volumes, panoramiques, réglages des effets) en mode SONG, et en mode PAT à l'endroit où le motif est posé ; points ou dessin à main levée (Maj+glisser).
- Enregistrement audio depuis le micro.
- Browser : tous les dossiers fonctionnent.
- 68 tests passent, plus 3 avec les vrais plugins et leurs fenêtres (`cargo test -- --include-ignored`), `cargo clippy` sans avertissement.

## Vérifié par clics rejoués (BEATSTUDIO_INPUT, captures dans `shots/`)

Menu Tools du Piano roll jusqu'au riff (A minor), sous-menu Steps du Channel rack (Fill each 2 steps),
clic droit sur la règle (insérer une mesure : blocs et automation suivent), Ctrl+glisser un bloc (copie),
Maj+glisser sur un couloir d'automation (dessin), ajout d'un Chorus dans le Mixer,
Piano roll au clavier : Ctrl+A (tout choisir), Ctrl+B (dupliquer), Maj+↑ (une octave),
plugin instrument choisi au clic droit (Citrus) et sa fenêtre ouverte par « Plugin window ».

## Pas encore vérifié

Ces fonctions compilent et leur logique est testée, mais personne ne les a essayées :

- Ctrl+glisser (cadre de sélection), Ctrl+C / Ctrl+V et couloir de vélocité dans le Piano roll ;
- automation en mode PAT (à l'oreille) ;
- fenêtre des paramètres d'un plugin ;
- son du Chorus et du Phaser (à l'oreille), export par piste depuis le menu ;
- prise réelle au micro.

## Limites connues

- Pas de VST, seulement CLAP. La fenêtre propre d'un plugin passe par X11 (XWayland sous Wayland) ; vérifiée sur 12 plugins (Dexed, Lemon…) par `cargo test real_plugin_windows -- --include-ignored`.
- Un son du disque est joué en entier pour une note d'un pas, coupé à la fin pour une note plus longue.
- L'automation est sur des couloirs en haut de la Playlist, pas sur les pistes. En mode PAT, un motif absent de la Playlist n'a pas d'automation.

## Pour reprendre

- Dépôt GitHub privé : `devfrp/beat-studio`, à jour au 1er novembre 2026.
- Le dossier `target` est un lien vers `/tmp/ca_target`, vidé au redémarrage : refaire `mkdir -p /tmp/ca_target` avant de compiler, et la première compilation est plus longue.
- Prochaine étape conseillée : essayer l'application (`cargo run`) et noter ce qui ne marche pas.
