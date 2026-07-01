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

## M3 — MIDI in (à détailler au jalon)

- [ ] Table exhaustive : chaque fader/potard/bouton du mapping → action attendue
- [ ] Couche Shift correcte sur tous les contrôles concernés

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
