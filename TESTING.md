# TESTING — checklist de test manuel contrôleur

Pas de test matériel en CI (specs §7) : cette checklist se déroule à la main
avec le **Hercules DJControl Inpulse 200 MK2** branché, avant chaque merge
touchant `engine`, `midi` ou `mapping`.

État : tout le code M0→M6 est implémenté et vert en CI — ces checklists
sont la **validation matérielle restante** du POC. Déjà vérifié sur le
MK2 : détection de la carte et ouverture du stream 4 canaux @ 48 kHz
(buffer imposé 1114 frames ≈ 23 ms, cf. docs/latence.md).

## Pré-requis

- [ ] Contrôleur détecté au lancement (barre d'état / logs)
- [ ] Carte son "DJControl" ouverte en 4 canaux (M2+) ; sinon fallback stéréo
- [ ] Débrancher/rebrancher le contrôleur en cours de session : reconnexion
      automatique, aucun crash (M3+)

## M1 — Moteur audio (clavier)

- [ ] Chargement de 2 pistes (MP3, FLAC, WAV), play/pause/seek au clavier
- [ ] Crossfader et volumes au clavier, aucun underrun signalé
- [ ] Latence mesurée ≤ 10 ms — méthode dans docs/latence.md ; attention :
      le MK2 impose ≈ 23 ms de buffer en 4 canaux ALSA brut (pistes
      d'amélioration documentées au même endroit)

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
- [ ] Bibliothèque au contrôleur : encodeur BROWSER (CC 0xB0 0x01) fait
      défiler, poussoir (0x90 0x00) entre dans les dossiers (ligne « .. »
      pour remonter), boutons Load chargent la piste sélectionnée
- [ ] Latence perçue fader → son : imperceptible (chemin court §5.1)

## M4 — Jogs

Les paramètres du modèle vivent dans `mappings/*.ron` (section `jog:`) —
itérer à l'oreille sans recompiler (le fichier local prime sur l'embarqué).

- [ ] `ticks_per_rev` réel du MK2 confirmé au midi-probe (un tour complet
      de jog = combien de ticks 0x0A ?)
- [ ] Scratch : la piste suit le doigt sans traîner ni osciller ; aucun son
      « escalier » à rotation lente ; aller-retour rapide propre
- [ ] Prise en main d'un deck en lecture : freinage naturel (pas de coupure)
- [ ] Relâchement : reprise de la lecture en ~100 ms sans à-coup ;
      sur deck à l'arrêt : glissement qui s'éteint en douceur
- [ ] Scratch arrière jusqu'au début de piste : butée propre, pas de crash
- [ ] Bend (bord, deck en lecture) : correction de tempo douce dans les deux
      sens, retour progressif à l'arrêt de la rotation
- [ ] Comparaison A/B avec Mixxx sur le même matériel ; ajuster
      `bend_sensitivity`, `velocity_window_ms`, `scratch_smoothing_ms`,
      `release_ramp_ms` puis reporter les valeurs retenues dans le RON embarqué

## M5 — Feedback + analyse

- [ ] À la connexion : LEDs play/cue/PFL reflètent immédiatement l'état
      courant (y compris après un débranchement/rebranchement)
- [ ] Play : LED play (note 0x07) suit lecture/pause, y compris via clavier
- [ ] Cue : LED cue (note 0x06) allumée dès qu'un point est posé
- [ ] PFL : LED casque (note 0x0C) suit le toggle (bouton ou touche 1/2)
- [ ] Fin de piste : LED (note 0x1C) s'allume sous 30 s restantes
- [ ] Aucun flood MIDI : LEDs stables = aucun message (vérifier au midi-probe
      sur le port de sortie ou via aseqdump)
- [ ] BPM sur pistes réelles : valeur stable et plausible (comparer à Mixxx),
      affichée dans le titre peu après le chargement

## M6 — UI

- [ ] Session de mix complète au contrôleur sans toucher la souris
- [ ] Waveforms : défilement parfaitement fluide en lecture (position
      extrapolée), beatgrid alignée à l'oreille, zoom molette sans à-coup
      (bascule de mipmap invisible)
- [ ] Widgets souris : chaque bouton/slider agit et reste synchronisé avec
      le contrôleur et le clavier (même état affiché)
- [ ] File picker (`F`/`L`, bouton LOAD, bouton MIDI) : chargement pendant
      la lecture de l'autre deck sans glitch audio
- [ ] Framerate natif stable (vérifier sur écran 120/144 Hz), frame < 8 ms
- [ ] Mode idle 10 fps après 5 s d'inactivité (vérifier avec un moniteur de
      fréquence), réveil instantané, thread audio insensible ; consommation
      mesurée sur laptop
- [ ] Panneau F12 : valeurs cohérentes, jamais affiché par défaut
