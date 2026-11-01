# Où on en est — 1er novembre 2026

## Fait

- Base : Channel rack, Piano roll, Playlist, Mixer, tempo 10–522 BPM, valeurs tirées de FL Studio (voir `MECHANICS.md`).
- Projets : enregistrer, ouvrir, sauvegarde automatique, modèles, deux démos.
- Exports : WAV, MP3 (bibliothèque LAME du système), MIDI. Import MIDI.
- Jeu en direct : métronome, tempo au tap, clavier d'ordinateur en piano, clavier MIDI, enregistrement des notes.
- Annuler / rétablir (Ctrl+Z / Ctrl+Y).
- Piano roll : sélection, copier / couper / coller / dupliquer, flèches, couloir de vélocité.
- Mixer : solo, envois entre pistes, 10 emplacements d'effets, 6 effets intégrés (Reverb, Delay, EQ, Compressor, Filter, Distortion).
- Plugins CLAP (instruments et effets) : 239 trouvés sur cette machine, réglages en curseurs, état gardé dans le projet.
- Sons du disque (WAV, MP3, FLAC, OGG), fichiers déposés sur la fenêtre.
- Automation en mode SONG (tempo, volumes, panoramiques, réglages des effets).
- Enregistrement audio depuis le micro.
- Browser : tous les dossiers fonctionnent.
- 26 tests passent (`cargo test -- --include-ignored` pour ceux avec les vrais plugins), `cargo clippy` sans avertissement.

## Pas encore vérifié à la main

Ces fonctions compilent et leur logique est testée, mais personne n'a encore cliqué dedans :

- sélection et copier-coller dans le Piano roll, couloir de vélocité ;
- points d'automation dans la Playlist ;
- fenêtre des paramètres d'un plugin ;
- prise réelle au micro.

## Limites connues

- La fenêtre propre des plugins ne s'affiche pas (seulement des curseurs). Pas de VST, seulement CLAP.
- Un son du disque est joué en entier pour une note d'un pas, coupé à la fin pour une note plus longue.
- L'automation n'agit qu'en mode SONG, sur des couloirs en haut de la Playlist.

## Pour reprendre

- Dépôt GitHub privé : `devfrp/beat-studio`, à jour au 1er novembre 2026.
- Le dossier `target` est un lien vers `/tmp/ca_target`, vidé au redémarrage : la première compilation suivante est plus longue.
- Prochaine étape conseillée : essayer l'application (`cargo run`) et noter ce qui ne marche pas.
