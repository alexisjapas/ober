# TESTING — checklist de test manuel contrôleur

Pas de test matériel en CI (specs §7) : cette checklist se déroule à la main
avec le **Hercules DJControl Inpulse 200 MK2** branché, avant chaque merge
touchant `engine`, `midi` ou `mapping`. Elle sera étoffée à chaque jalon.

## Pré-requis

- [ ] Contrôleur détecté au lancement (barre d'état / logs)
- [ ] Carte son "DJControl" ouverte en 4 canaux (M2+) ; sinon fallback stéréo
- [ ] Débrancher/rebrancher le contrôleur en cours de session : reconnexion
      automatique, aucun crash (M3+)

## M1 — Moteur audio (clavier)

- [ ] Chargement de 2 pistes (MP3, FLAC, WAV), play/pause/seek au clavier
- [ ] Crossfader et volumes au clavier, aucun underrun signalé
- [ ] Latence mesurée ≤ 10 ms (documenter la méthode)

## M2 — DSP

- [ ] EQ 3 bandes audibles et symétriques sur chaque deck, kill fonctionnel
- [ ] Varispeed ±8 % / ±16 % sans artefacts
- [ ] Pré-écoute casque : cue par deck, potard cue/master, gain casque
- [ ] Limiteur : pas d'écrêtage dur en poussant tous les gains

## M3 — MIDI in

Avant tout : valider les codes MIDI réels du MK2 avec
`cargo run -p midi --bin midi-probe` (le mapping vient de l'Inpulse 200
première génération via Mixxx — corriger `mappings/*.ron` si écart).

- [ ] Contrôleur détecté au lancement (« MIDI <nom> » dans le titre) et
      LEDs pilotables après l'init (`0xB0 0x7F 0x7F`)
- [ ] Débrancher → titre repasse à « MIDI — », aucun crash ; rebrancher →
      reconnexion automatique sous ~2 s, contrôles de nouveau opérants
- [ ] Play A/B (0x91/0x92 note 0x07) : lecture/pause, état titre cohérent
- [ ] Cue A/B (note 0x06) : pose du point à l'arrêt, retour au point en
      lecture, pré-écoute tant que le bouton est tenu depuis le point
- [ ] PFL casque A/B (note 0x0C) : toggle cue casque (indicateur CUE titre)
- [ ] Load A/B (note 0x0D) : message log (file picker M6)
- [ ] Crossfader (0xB0 0x00) : plein gauche = A seul, plein droite = B seul,
      courbe constant power au centre
- [ ] Volumes (0xB1/0xB2 0x00) : plage complète, sans crans audibles
- [ ] EQ basses/aigus (0x02/0x04) : kill −26 dB franc à gauche, +6 dB à
      droite, potard médian ≈ neutre à vérifier à l'oreille
- [ ] Pitch (0x08) : ±8 %, **vérifier le sens** (haut = plus lent attendu ?)
      et l'absence de saut au premier mouvement
- [ ] Jogs : les messages arrivent (log/debug) — le scratch lui-même : M4
- [ ] Latence perçue fader → son : imperceptible (chemin court §5.1)

## M4 — Jogs (à détailler au jalon)

- [ ] Bend : correction de tempo douce, retour progressif
- [ ] Scratch : suivi précis, pas de son "escalier", rampe de relâchement propre
- [ ] Comparaison A/B avec Mixxx sur le même matériel

## M5 — Feedback (à détailler au jalon)

- [ ] LEDs play/cue synchronisées avec l'état réel
- [ ] VU LEDs cohérents avec les niveaux affichés

## M6 — UI (à détailler au jalon)

- [ ] Session de mix complète au contrôleur sans toucher la souris
- [ ] Framerate natif stable (vérifier sur écran 120/144 Hz), frame < 8 ms
- [ ] Mode idle 10 fps après 5 s d'inactivité, réveil instantané
