# Latence audio — méthode de mesure et état

Critère M1 (specs §3.6) : latence de sortie ≤ 10 ms (buffer + périphérique)
sur Linux/ALSA avec le contrôleur.

## Décomposition

```
latence totale ≈ buffer logiciel (cpal) + buffer(s) du périphérique + DAC
```

- **Buffer logiciel** : `TARGET_BUFFER_FRAMES = 256` frames à 48 kHz
  = **5,33 ms** (clampé à la plage annoncée par le périphérique ; la taille
  effective est loggée au démarrage et visible dans `StreamInfo`).
- **Périphérique** : dépend du backend. Sous PipeWire (couche ALSA), le
  quantum ajoute typiquement 1 période ; sur l'ALSA brut de la carte
  DJControl, viser 2 périodes de 128–256 frames.

## Charge du callback (mesurée, bench criterion)

`cargo bench -p engine --bench callback` — mix 2 decks actifs, bloc de
128 frames, machine de dev (2026-07) :

- **≈ 665 ns / bloc**, soit ~0,03 % du budget temps réel de 2,67 ms
  (budget specs : < 20 %). Marge très large pour l'EQ/varispeed/limiteur du M2.

Le snapshot expose `callback_load` (lissé) et `underruns` en continu dans la
barre d'état (titre de fenêtre au M1).

## Mesure de la latence réelle (à faire avec le matériel — M2)

Méthode recommandée, boucle physique :

1. Sortie master de la carte DJControl → entrée ligne/micro d'une carte de
   capture (ou la même carte si full-duplex).
2. Jouer un click généré (piste WAV d'impulsions), enregistrer la boucle,
   mesurer l'écart émission→retour dans un éditeur (Audacity), puis
   soustraire la latence d'entrée de la carte de capture.
3. Alternative logicielle : `pw-top` (quantum effectif par nœud) ou
   `cat /proc/asound/cardX/pcm0p/sub0/hw_params` + `status` (taille de
   période et de buffer réellement négociées par ALSA).

| Configuration | Buffer logiciel | Latence mesurée | Date |
|---|---|---|---|
| PipeWire (périph. par défaut) | 256 frames (5,33 ms) | _à mesurer_ | — |
| ALSA direct DJControl | _cible 128–256_ | _à mesurer (M2)_ | — |
