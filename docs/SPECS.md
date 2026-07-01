# Spécifications techniques — Logiciel de mix DJ open-source

**Version :** 0.2 (POC)
**Cibles :** Linux (priorité), macOS, Windows
**Langage :** Rust (stable)
**Framework applicatif :** Bevy (choix acté — voir §1.4)
**Licence :** GPL-3.0 (à confirmer — compatibilité avec les mappings Mixxx réutilisés comme référence)

**Priorités produit (par ordre) :** performance runtime, stabilité, réactivité, extensibilité vers des systèmes d'analyse sonore visuelle complexes, finition visuelle. La taille du binaire et la durée de compilation sont explicitement **non prioritaires**.

---

## 1. Objectif et périmètre

### 1.1 Objectif du POC

Démontrer un moteur de mix 2 decks à faible latence, contrôlable intégralement via le **Hercules DJControl Inpulse 200 MK2** (MIDI in/out + carte son intégrée), avec pré-écoute casque et affichage waveform temps réel.

### 1.2 Dans le périmètre (v0.1)

- 2 decks : chargement de fichiers audio locaux, play/pause, cue point, seek
- Mixage : crossfader, volume par deck, EQ 3 bandes par deck, gain master
- Pitch : varispeed ±8 % / ±16 % (sans keylock)
- Routage audio : sortie master (canaux 1/2) + pré-écoute casque (canaux 3/4) sur la carte son du contrôleur, avec cue mix
- Analyse offline au chargement : BPM, beatgrid, waveform pré-calculée
- MIDI bidirectionnel : contrôles entrants + feedback LED
- UI Bevy : 2 waveforms scrollables, VU-mètres, état des decks
- Système de mapping contrôleur déclaratif (RON)

### 1.3 Hors périmètre (v0.1)

- Keylock / time-stretch temps réel
- Sync automatique / master tempo
- Effets (reverb, delay, filter…)
- Bibliothèque musicale avec base de données (un simple file picker suffit)
- Enregistrement du mix
- Streaming (interdit par les CGU des plateformes de toute façon)
- Support d'autres contrôleurs (mais l'architecture doit le permettre)

### 1.4 Choix de Bevy — justification et implications

Bevy est retenu comme framework applicatif pour les raisons suivantes :

- **Rendu GPU retenu (retained)** : les visualisations cibles à moyen terme (spectrogrammes temps réel, waveforms multi-bandes zoomables, analyses harmoniques, rendus réactifs au signal) exigent meshes persistants, instancing, textures streamées et shaders custom. Le render graph wgpu de Bevy fournit cette infrastructure nativement ; un framework immediate-mode (egui seul) obligerait à la reconstruire à la main.
- **Évolutivité analyse** : possibilité à terme de déporter des analyses sur GPU (compute shaders : FFT, corrélation de phase) dans le même pipeline.
- **Design system custom** : le niveau de finition visé (référence : Serato/Traktor/Rekordbox, UI entièrement dessinées sur mesure) se construit sur des primitives de rendu, pas sur des widgets système. Bevy fournit ces primitives.
- La familiarité de l'équipe avec Bevy et la mutualisation avec les autres projets internes sont des bénéfices secondaires assumés.

**Implications contractuelles pour le développement :**

1. **Épinglage de version** : la version de Bevy est fixée dans `Cargo.toml` (version exacte, pas de `^`). Les migrations de version majeure sont des tâches planifiées (~1×/an), jamais subies. Aucune dépendance à des crates Bevy tierces non maintenues activement.
2. **Frontière moteur/UI** : `engine`, `decode`, `analysis`, `midi`, `mapping` ne dépendent jamais de Bevy (garanti par la structure du workspace et vérifié en CI via `cargo tree`). Le churn de version Bevy n'expose que la crate `app`.
3. **Bevy UI (`bevy_ui`) n'est pas utilisé pour le cœur de l'interface** : decks, waveforms, VU, contrôles sont des entités à rendu custom (meshes/materials/shaders). `bevy_egui` est réservé aux panneaux secondaires (préférences, futur éditeur de mapping, debug).
4. **Gestion de l'énergie** : la game loop continue est le comportement voulu en lecture (animations permanentes). Un mode basse consommation est requis à l'idle (aucun deck en lecture) : réduction du framerate cible (10 fps) via `bevy::winit::WinitSettings`, retour immédiat à 60+ fps sur interaction ou lecture.

---

## 2. Architecture générale

### 2.1 Principe directeur

**Séparation stricte temps-réel / non temps-réel.** Le processus est découpé en trois domaines :

```
┌─────────────────────────────────────────────────────────────┐
│  Thread audio temps-réel (callback cpal)                     │
│  - Aucune allocation, aucun lock, aucun syscall              │
│  - Mixage, EQ, varispeed, routage master/cue                 │
└──────────────▲──────────────────────────┬────────────────────┘
               │ commandes (SPSC lock-free)│ état + audio taps
┌──────────────┴──────────────────────────▼────────────────────┐
│  Threads workers                                             │
│  - Décodage symphonia (streaming vers ring buffers)          │
│  - Analyse offline : BPM, beatgrid, waveform, rustfft        │
│  - I/O MIDI (midir) : parsing entrant, feedback LED sortant  │
└──────────────▲──────────────────────────┬────────────────────┘
               │ events                    │ state snapshots
┌──────────────┴──────────────────────────▼────────────────────┐
│  Bevy (thread principal)                                     │
│  - ECS : état applicatif, UI, waveforms, VU-mètres           │
│  - Input clavier/souris (fallback sans contrôleur)           │
└──────────────────────────────────────────────────────────────┘
```

### 2.2 Règles absolues du thread audio

Le callback `cpal` **ne doit jamais** :
- allouer (`Box`, `Vec::push`, `String`, `format!`…)
- prendre un `Mutex`/`RwLock` (y compris implicitement via `log!`)
- faire d'I/O (fichier, réseau, stdout)
- bloquer sur un channel (utiliser `try_recv`/`pop` non bloquants uniquement)

Toute la mémoire nécessaire (buffers de decks, états DSP) est pré-allouée à l'initialisation ou lors du chargement d'une piste (côté worker), puis transférée au thread audio par échange de pointeur via channel lock-free.

### 2.3 Communication inter-threads

| Canal | Type | Direction | Contenu |
|---|---|---|---|
| Commandes audio | SPSC lock-free (`rtrb` ou `ringbuf`) | UI/MIDI → audio | `Play`, `SetVolume(deck, f32)`, `SeekSamples(deck, u64)`, `SwapTrackBuffer(deck, ptr)`… |
| État audio | Triple buffer ou SPSC | audio → UI | position de lecture, niveaux RMS/peak, état play/cue |
| Audio tap (visualisation) | SPSC | audio → UI | blocs de samples post-mix pour FFT/VU temps réel |
| Récupération mémoire | SPSC | audio → worker | anciens buffers de piste à désallouer (jamais de `drop` dans le callback) |

Crates recommandées : `rtrb` (ring buffer temps réel), `triple_buffer` (snapshots d'état), `crossbeam-channel` (côté non temps-réel uniquement).

### 2.4 Organisation en crates (workspace Cargo)

```
dj-mix/
├── crates/
│   ├── engine/        # moteur audio : DSP, decks, mixer, cpal — AUCUNE dépendance Bevy
│   ├── decode/        # symphonia + rubato : décodage, resampling vers f32 interleaved
│   ├── analysis/      # BPM, beatgrid, waveform summary, rustfft
│   ├── midi/          # midir, parsing MIDI, moteur de mapping, feedback LED
│   ├── mapping/       # format RON, types de mapping, chargement/validation
│   └── app/           # binaire Bevy : UI, orchestration, plugins
├── mappings/
│   └── hercules_inpulse_200_mk2.ron
└── assets/
```

`engine` doit compiler et être testable sans Bevy (tests unitaires DSP, bench criterion). C'est une exigence, pas une suggestion : elle garantit la frontière architecturale.

---

## 3. Moteur audio (`engine`)

### 3.1 Format interne

- **f32, 48 kHz, stéréo entrelacé** en interne. Tout fichier est resamplé vers 48 kHz au décodage (`rubato`, offline).
- Taille de buffer cible : 128–256 samples (2,7–5,3 ms). Configurable ; fallback 512 si le périphérique l'exige.

### 3.2 Périphérique de sortie

- Le contrôleur expose une carte son USB **4 canaux de sortie** (1/2 = master, 3/4 = casque). Ouvrir ce périphérique en un seul stream 4 canaux quand il est disponible.
- Fallback : périphérique par défaut du système en stéréo (master uniquement, pas de pré-écoute) — l'application doit rester utilisable sans le contrôleur.
- Hôtes : ALSA (Linux — documenter la configuration ; PipeWire fonctionne via la couche ALSA), CoreAudio (macOS), WASAPI (Windows). `cpal` abstrait les trois.
- Sélection du périphérique dans un fichier de config + détection automatique par nom (match sur "DJControl").

### 3.3 Chaîne de traitement par deck

```
buffer piste (f32 48k) → varispeed (interpolation) → EQ 3 bandes → gain deck → ┐
                                                                              ├→ crossfader → gain master → out 1/2
                                                       [tap cue si activé] → ─┘→ cue mix → out 3/4
```

- **Varispeed** : lecture à position fractionnaire avec interpolation **Hermite cubique 4 points** (bon compromis qualité/CPU ; linéaire acceptable pour un premier jet, à remplacer avant la v0.1). Le pitch modifie la hauteur — comportement vinyle assumé.
- **EQ 3 bandes** : biquads (low-shelf ~250 Hz, peak ~1 kHz, high-shelf ~2,5 kHz), gains −26 dB → +6 dB, kill à −∞ optionnel. Implémentation maison (RBJ cookbook) ou crate `biquad`. Recalcul des coefficients hors callback (les commandes portent les coefficients, pas les fréquences).
- **Crossfader** : courbe configurable (constant power par défaut, sharp cut pour le scratch).
- **Cue mix** : `out_casque = cue_gain * mix(decks cue actifs) + master_gain_cue * master`, contrôlé par le potard "cue/master" du contrôleur.
- **Limiteur soft-clip** sur le master (protection oreilles/enceintes) : `tanh` ou clipping avec knee, simple mais obligatoire.

### 3.4 Chargement de piste

1. UI demande le chargement → worker `decode` décode **l'intégralité** du fichier en mémoire (f32 48 kHz). Pour le POC, on assume des pistes ≤ 15 min (~330 Mo max en f32 stéréo — acceptable ; le streaming par chunks est une évolution v0.2).
2. Le worker `analysis` calcule BPM/beatgrid/waveform sur ce buffer.
3. Le buffer est transféré au thread audio via commande `SwapTrackBuffer` (échange de `*mut`/`Arc` pré-construit, pas de copie).
4. L'ancien buffer repart vers le worker pour désallocation.

### 3.5 Jog wheels — modèle de scratch

Le point le plus délicat du projet. Deux modes selon le message MIDI reçu :

- **Bord du jog (pas de touch)** : pitch bend temporaire — offset de vitesse proportionnel à la vélocité de rotation, avec retour progressif à la vitesse nominale.
- **Surface touchée (scratch)** : la position de lecture suit la position du jog. Implémenter un modèle à inertie : le jog envoie des ticks relatifs (±1 par cran, taux variable) ; convertir en vélocité cible via une fenêtre glissante (~10–20 ms), puis asservir la vitesse de lecture avec un filtre passe-bas (constante de temps ~5 ms) pour éviter le son "escalier". Au relâchement : rampe de retour à la vitesse nominale (~50–200 ms, configurable).

Les paramètres (sensibilité, ticks/tour, courbes) vivent dans le fichier de mapping, pas dans le code.

### 3.6 Critères de performance

- Latence de sortie ≤ 10 ms (buffer + périphérique) sur Linux/ALSA avec le contrôleur.
- **Zéro underrun** en usage normal sur une machine moderne ; instrumenter le callback (compteur d'underruns exposé à l'UI, mesure du temps de callback en debug).
- Charge CPU du callback < 20 % du budget temps (marge pour les évolutions).

---

## 4. Décodage et analyse

### 4.1 Décodage (`decode`)

- `symphonia` : MP3, FLAC, WAV, OGG Vorbis, AAC/M4A (activer les features correspondantes).
- Sortie normalisée : `Vec<f32>` entrelacé stéréo 48 kHz (`rubato` `SincFixedIn` pour le resampling, qualité haute — c'est offline).
- Mono → duplication stéréo. Gestion des erreurs de décodage partielles (fichier tronqué → garder ce qui est décodé, signaler à l'UI).

### 4.2 Analyse (`analysis`)

- **BPM + beatgrid** : détection d'onsets par flux d'énergie spectrale (`rustfft`, fenêtres 1024/hop 512), puis autocorrélation/histogramme d'intervalles pour le tempo (plage 60–200 BPM, résolution 0,01 BPM), et phase du premier beat par maximisation d'alignement. Grille fixe (tempo constant) suffisante pour le POC. Alternative acceptable : porter l'approche de `aubio` (ne pas binder la lib C, rester pur Rust).
- **Waveform summary** : min/max/RMS par bande (basses/médiums/aigus via 3 filtres) à ~1000 points/s → structure compacte envoyée à l'UI, uploadée en textures/buffers GPU (mipmaps : 1×, 4×, 16×) pour un rendu shader sans régénération (voir §6.1).
- **Pipeline d'analyse temps réel extensible** : le tap audio (§2.3) alimente un bus d'analyseurs côté worker — chaque analyseur est un trait `Analyzer { fn process(&mut self, block: &[f32]) -> Option<AnalysisFrame> }` enregistré dynamiquement. v0.1 n'en implémente qu'un (niveaux RMS/peak pour les VU), mais le bus est la fondation des visualisations futures (spectrogramme, chroma/harmonie, corrélation de phase entre decks, détection de structure). Les `AnalysisFrame` transitent vers Bevy par un canal dédié, typés par analyseur.
- L'analyse tourne en tâche de fond ; la piste est jouable dès le décodage terminé, le beatgrid arrive ensuite.

---

## 5. MIDI et mapping (`midi`, `mapping`)

### 5.1 I/O MIDI

- `midir` pour input/output. Thread dédié au MIDI in (callback midir → parsing → events vers Bevy et commandes directes vers le thread audio pour les contrôles critiques : jogs, faders, crossfader).
- **Chemin court pour les contrôles critiques** : jog/fader → commande audio directe sans passer par le scheduler Bevy (latence de frame inacceptable pour le scratch). Bevy reçoit une copie pour l'affichage.
- Hot-plug : détection de connexion/déconnexion du contrôleur, reconnexion automatique, l'application ne crashe jamais sur un débranchement.

### 5.2 Format de mapping (RON)

Un contrôleur = un fichier RON déclaratif. Structure cible :

```ron
Mapping(
    name: "Hercules DJControl Inpulse 200 MK2",
    device_match: ["DJControl Inpulse 200 MK2"],  // substring sur le nom du port MIDI
    controls: [
        // Bouton simple
        (input: NoteOn(ch: 1, note: 0x07), action: Play(deck: A), mode: Toggle),
        // Fader 7 bits
        (input: CC(ch: 0, cc: 0x00), action: CrossFader, mode: Absolute),
        // Potard EQ
        (input: CC(ch: 1, cc: 0x02), action: EqMid(deck: A), mode: Absolute(curve: DbLinear(-26, 6))),
        // Jog : ticks relatifs signés
        (input: CC(ch: 1, cc: 0x0A), action: JogTick(deck: A), mode: Relative(encoding: SignedBit)),
        // Touch du jog
        (input: NoteOn(ch: 1, note: 0x08), action: JogTouch(deck: A), mode: Gate),
        // Shift layer
        (input: NoteOn(ch: 1, note: 0x03), action: Shift, mode: Gate),
        (input: NoteOn(ch: 1, note: 0x07), shift: true, action: Cue(deck: A), mode: Momentary),
    ],
    feedback: [
        (state: Playing(deck: A), output: NoteOn(ch: 1, note: 0x07), on: 0x7F, off: 0x00),
        (state: CueSet(deck: A),  output: NoteOn(ch: 1, note: 0x06), on: 0x7F, off: 0x00),
        (state: VuMaster,         output: CC(ch: 0, cc: 0x30), scale: Linear(0, 127)),
    ],
    jog: (ticks_per_rev: 720, touch_scratch: true, bend_sensitivity: 0.3, release_ramp_ms: 100),
)
```

Le moteur de mapping est **générique** : il traduit `événement MIDI → Action` (enum exhaustive du domaine : `Play`, `Cue`, `Seek`, `EqLow`…) et `StateChange → message MIDI`. Aucun code spécifique Hercules dans le moteur.

- Trait `ControllerBackend` pour les cas non exprimables en déclaratif (séquences d'init propriétaires, SysEx). Le backend générique piloté par RON est l'implémentation par défaut ; l'Inpulse 200 MK2 ne devrait pas nécessiter plus (à valider — Hercules utilise parfois un message d'init pour activer le mode "full MIDI" des LEDs).
- Validation du mapping au chargement : erreurs lisibles (contrôle dupliqué, action inconnue, canal hors plage).

### 5.3 Référence Inpulse 200 MK2

Pas de spec MIDI publique complète chez Hercules. Sources à exploiter :
1. **Mapping Mixxx** (XML + JS, dépôt `mixxxdj/mixxx`) — référence principale pour les notes/CC des pads, jogs, faders, LEDs, et le message d'init éventuel. Attention à la licence si du contenu est traduit (d'où GPL envisagée).
2. Rétro-ingénierie directe : `aseqdump`/`midisnoop` sur le contrôleur physique pour valider chaque contrôle. **Livrer un outil `midi-probe`** (binaire du workspace) qui logge les messages entrants — utile pour tous les futurs contrôleurs.
3. Le "beatmatch guide" (LEDs de guidage tempo/phase) fonctionne par messages MIDI sortants — feature v0.2, mais réserver les états dans l'enum de feedback.

---

## 6. UI et rendu Bevy (`app`)

### 6.1 Stratégie de rendu

L'interface est découpée en deux couches, avec une règle nette :

- **Couche performance (rendu custom)** : waveforms, spectrogramme (v0.2+), VU-mètres, beatgrid, tête de lecture, jogs virtuels. Entités Bevy avec meshes/materials/shaders WGSL dédiés. C'est la couche qui justifie Bevy et où se joue la qualité perçue.
- **Couche utilitaire (`bevy_egui`)** : panneaux de préférences, sélection de périphérique, debug/diagnostics, futur éditeur de mapping. Jamais visible pendant une session de mix normale.

Règles de rendu de la couche performance :

- **Aucune régénération de mesh par frame.** Waveform : les mipmaps min/max/RMS (§4.2) sont uploadées une fois en textures/buffers GPU au chargement de la piste ; le défilement et le zoom se font dans le shader (offset/scale d'UV en uniform). Le CPU n'écrit par frame que quelques uniforms (position de lecture interpolée, zoom, gains).
- **VU-mètres et indicateurs de niveau** : instancing d'un quad + uniforms, jamais de reconstruction de géométrie.
- **Spectrogramme temps réel (préparé dès v0.1, activé v0.2)** : texture en anneau (ring texture) mise à jour par bandes via `write_texture` depuis les données du tap audio, rendue par un shader avec offset circulaire. La FFT reste sur CPU (`rustfft`) en v0.x ; le portage en compute shader est une évolution possible sans changement d'architecture.
- **Interpolation temporelle** : la position de lecture affichée est extrapolée entre snapshots audio (`position + vitesse × Δt`) pour un défilement parfaitement fluide, avec correction douce (pas de snap) quand un snapshot arrive.
- Budget : frame CPU+GPU < 8 ms à 60 fps minimum ; l'application doit supporter les écrans 120/144 Hz (framerate non plafonné à 60, animations basées sur le temps réel, jamais sur le compteur de frames).

### 6.2 Design system

- L'UI de session est **entièrement dessinée sur mesure** (référence de finition : Serato/Traktor). Aucun widget `bevy_ui` pour le cœur.
- Définir dès M6 un module `theme` : palette (tokens de couleur sémantiques), échelle typographique, rayons, espacements — consommé par les materials et par le style egui (les panneaux utilitaires doivent rester cohérents visuellement).
- Fonts : **Inter** (texte) + **Phosphor Icons** — réutiliser le module `fonts.rs` existant.
- Toute animation (transitions d'état, pulsations beat, VU decay) est paramétrée par des courbes d'easing centralisées dans `theme`, pas codée en dur.

### 6.3 Écran unique (POC)

- 2 waveforms horizontales superposées, tête de lecture fixe au centre, défilement synchronisé à la position audio interpolée, beatgrid en surimpression, zoom molette (niveaux mipmap 1×/4×/16×).
- Par deck : titre/BPM/temps restant, boutons play/cue cliquables, sliders volume/EQ/pitch (fallback souris), indicateur cue casque.
- Section centrale : crossfader, VU master, gain casque + cue/master mix.
- Barre d'état : périphérique audio actif, contrôleur détecté, underruns, charge CPU audio, fps.
- File picker natif (`rfd`) pour charger les pistes.

### 6.4 Synchronisation état

- Un système Bevy draine les snapshots du triple buffer audio chaque frame → met à jour les composants ECS.
- Les interactions UI émettent les mêmes `Action` que le MIDI (un seul chemin de traitement des intentions).

### 6.5 Gestion de l'énergie

- **Idle** (aucun deck en lecture, pas d'interaction depuis > 5 s) : framerate cible réduit à 10 fps via `WinitSettings` (mode réactif avec timeout).
- **Actif** : framerate natif de l'écran dès qu'un deck joue ou qu'une interaction survient (transition immédiate, sans frame perdue perceptible).
- Le thread audio n'est **jamais** affecté par ces modes.

---

## 7. Qualité, tests, CI

- **Tests unitaires** : DSP (réponse des biquads vs référence, courbes de crossfader), moteur de mapping (événement → action, table exhaustive Inpulse), parsing RON, détection BPM sur un corpus de fichiers de test à tempo connu (clicks générés + extraits réels, tolérance ±0,1 BPM).
- **Benchmarks** (`criterion`) : coût du callback pour 2 decks actifs à 128 samples ; budget < 20 % du temps réel.
- **Test d'intégration audio** : rendu offline du graphe (mêmes structs, appelées hors cpal) → fichiers WAV de non-régression.
- **CI** (GitHub Actions) : build + tests Linux/macOS/Windows, `cargo clippy -D warnings`, `cargo fmt --check`, **vérification de la frontière Bevy** (script CI : `cargo tree -p engine -p decode -p analysis -p midi -p mapping | grep -q bevy` doit échouer). Pas de test matériel en CI ; prévoir une checklist de test manuel contrôleur (document `TESTING.md`).
- **Instrumentation** : feature flag `rt-checks` qui panique sur allocation dans le callback en debug (via allocateur traqué type `assert_no_alloc`).

---

## 8. Jalons

| Jalon | Contenu | Critère de sortie |
|---|---|---|
| **M1 — Moteur audio** | Workspace, engine + decode, 2 decks au clavier, volume/crossfader, sortie stéréo | Mix 2 pistes sans underrun, latence mesurée ≤ 10 ms |
| **M2 — DSP** | EQ 3 bandes, varispeed Hermite, limiteur, cue routing 4 canaux sur la carte son du contrôleur | Pré-écoute casque fonctionnelle sur l'Inpulse |
| **M3 — MIDI in** | midir, moteur de mapping RON, mapping Inpulse complet (hors jogs), outil `midi-probe` | Tous faders/potards/boutons opérants |
| **M4 — Jogs** | Modèle scratch/bend avec inertie | Scratch propre à l'oreille, pas d'artefacts |
| **M5 — Feedback + analyse** | LEDs (play/cue/VU), BPM/beatgrid offline | LEDs synchronisées, BPM ±0,1 sur corpus |
| **M6 — UI** | Waveforms Bevy (rendu shader), design system `theme`, panneau complet, mode idle basse conso, file picker | Session de mix complète au contrôleur, framerate natif de l'écran stable, frame < 8 ms |

Estimation indicative pour un sénior Rust avec expérience audio : M1–M2 sont le cœur du risque ; M3–M5 sont du travail méthodique ; M4 demandera des itérations à l'oreille avec le matériel physique. M6 inclut désormais la mise en place du design system et du pipeline de rendu shader — prévoir une marge en conséquence.

---

## 9. Risques identifiés

| Risque | Impact | Mitigation |
|---|---|---|
| Codes MIDI Inpulse incomplets/erronés | M3–M5 bloqués | Mapping Mixxx + `midi-probe` + matériel physique dès le début |
| Carte son 4 canaux mal exposée par cpal/ALSA | Pas de pré-écoute | Tester tôt (M2) ; fallback 2 périphériques séparés si nécessaire |
| Scratch de qualité insuffisante | Produit non crédible | Modèle à inertie paramétrable, itérations à l'oreille, comparer à Mixxx |
| Latence Bevy → audio sur contrôles critiques | Scratch mou | Chemin MIDI → audio direct (spécifié §5.1), à respecter strictement |
| GC de buffers dans le callback | Glitches | Canal de récupération mémoire (§2.3), `assert_no_alloc` en debug |
| Breaking changes Bevy | Coût de maintenance, régressions UI | Version épinglée, migrations planifiées (§1.4), moteur isolé de Bevy |
| Consommation CPU/GPU excessive à l'idle | Mauvaise expérience laptop/batterie | Mode basse consommation (§6.5), mesuré et testé sur laptop |
| Complexité du rendu shader custom sous-estimée | M6 en dérive | Prototyper la waveform shader tôt (spike pendant M3–M4, en parallèle) |
