# Latence audio — méthode de mesure et état

Critère M1 (specs §3.6) : latence de sortie ≤ 10 ms (buffer + périphérique)
sur Linux/ALSA avec le contrôleur.

## Décomposition

```
latence totale ≈ buffer logiciel (cpal) + buffer(s) du périphérique + DAC
```

- **Buffer logiciel** : `TARGET_BUFFER_FRAMES = 256` frames à 48 kHz
  = **5,33 ms** (clampé à la plage annoncée par la configuration retenue ;
  la taille effective est loggée au démarrage et visible dans `StreamInfo`).
- **Périphérique** : dépend du backend. Sous PipeWire (couche ALSA), le
  quantum ajoute typiquement 1 période.

**Mesuré sur le DJControl Inpulse 200 Mk2 (ALSA brut, 2026-07)** : la carte
n'accepte que des buffers de **1114–1115 frames** en 4 canaux @ 48 kHz, soit
≈ 23,2 ms de buffer logiciel — au-dessus de la cible de 10 ms des specs.
Pistes pour descendre :
- vérifier si PipeWire expose la carte en 4 canaux avec un quantum plus
  court (alors pointer `device_match` vers ce nœud) ;
- tester d'autres fréquences (44,1 kHz) au cas où la plage diffère ;
- sinon, contrainte matérielle à documenter comme telle (l'objectif ≤ 10 ms
  des specs visait « buffer + périphérique » sur du matériel qui le permet).

## Charge du callback (mesurée, bench criterion)

`cargo bench -p engine --bench callback` — 2 decks actifs, bloc de
128 frames, machine de dev (Ryzen 7 7800X3D, 2026-07) :

- **M1** (mix simple, stéréo) : ≈ 665 ns / bloc, ~0,03 % du budget de 2,67 ms.
- **M2** (chaîne complète : varispeed Hermite, EQ 3 bandes, cue, limiteur,
  sortie 4 canaux) : ≈ 6,6 µs / bloc, ~0,25 % du budget.
- **M4** (+ modèle de jog par frame) : **≈ 7,9 µs / bloc**, soit ~0,3 % du
  budget (budget specs : < 20 %). Marge très large pour la suite.

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
