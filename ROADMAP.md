# Roadmap — ober (POC v0.1)

Référence : [docs/SPECS.md](docs/SPECS.md) (specs v0.2). Chaque jalon a un
objectif démontrable et un **critère de sortie mesurable** ; on n'entame pas un
jalon tant que le critère du précédent n'est pas tenu. Exception voulue : le
*spike waveform shader* se mène en parallèle de M3–M4 (dérisquage de M6, §9).

## Vue d'ensemble

| Jalon | Contenu | Critère de sortie | Statut |
|---|---|---|---|
| **M0** | Scaffolding : workspace, flake nix, CI, squelettes de types | `cargo test` vert dans `nix develop`, CI verte | 🟡 quasi fait |
| **M1** | Moteur audio : engine + decode, 2 decks au clavier, volume/crossfader, sortie stéréo | Mix 2 pistes sans underrun, latence mesurée ≤ 10 ms | ⬜ |
| **M2** | DSP : EQ 3 bandes, varispeed Hermite, limiteur, cue 4 canaux | Pré-écoute casque fonctionnelle sur l'Inpulse | ⬜ |
| **M3** | MIDI in : midir, moteur de mapping RON, mapping Inpulse (hors jogs) | Tous faders/potards/boutons opérants | ⬜ |
| **M4** | Jogs : modèle scratch/bend à inertie | Scratch propre à l'oreille, pas d'artefacts | ⬜ |
| **M5** | Feedback LED + analyse offline (BPM/beatgrid/waveform) | LEDs synchronisées, BPM ±0,1 sur corpus | ⬜ |
| **M6** | UI : waveforms shader, design system, mode idle, file picker | Session de mix complète au contrôleur, frame < 8 ms | ⬜ |

M1–M2 concentrent le risque technique (temps réel, carte son 4 canaux).
M4 demande des itérations à l'oreille avec le matériel physique.

---

## M0 — Scaffolding

- [x] Workspace 6 crates (§2.4) ; seule `app` dépend de Bevy
- [x] Flake nix : toolchain stable (`rust-toolchain.toml`), ALSA/Vulkan/Wayland/X11/udev, `aseqdump`
- [x] Versions épinglées dans `[workspace.dependencies]` — Bevy en version **exacte** `=0.19.0` (§1.4)
- [x] CI GitHub Actions : fmt, `clippy -D warnings`, tests Linux/macOS/Windows, **vérification de la frontière Bevy** (`scripts/check-bevy-boundary.sh`)
- [x] Squelettes de types : `EngineCommand`/`EngineSnapshot`, `DecodedTrack`, `Analyzer`/`AnalysisFrame`, `Action`/`Mapping` RON
- [x] `midi-probe` opérationnel (log hex de tous les ports d'entrée)
- [x] `cargo test --workspace`, `clippy -D warnings`, `fmt` et frontière Bevy verts dans `nix develop` (Rust 1.96.1, Linux)
- [ ] Confirmer la licence GPL-3.0 (compat mappings Mixxx) et ajouter `LICENSE`
- [ ] Pousser sur un remote et vérifier que la CI passe sur les 3 OS

## M1 — Moteur audio

Objectif : mixer 2 pistes au clavier, sortie stéréo sur le périphérique par défaut.

- [x] `decode` : symphonia 0.6 (probe → packets) → f32 entrelacé ; rubato 3 (`Async` sinc = ex-`SincFixedIn`) → 48 kHz ; mono→stéréo ; fichiers tronqués tolérés et signalés (§4.1)
- [x] `engine` : état de deck (buffer, position f64, gain), mixer (volumes, crossfader constant power), gain master (§3.3)
- [x] Stream cpal stéréo, buffer cible 256 frames clampé à la plage du périphérique (fallback 512 inclus) (§3.1)
- [x] Canaux inter-threads (§2.3) : commandes `rtrb` UI→audio ; snapshots `triple_buffer` audio→UI ; tap audio (bloc entier ou rien) ; **canal de récupération mémoire** (jamais de désallocation dans le callback, l'UI garde un clone de chaque Arc)
- [x] `SwapTrackBuffer` par échange d'`Arc<TrackBuffer>` pré-construit, sans copie (§3.4)
- [x] Feature `rt-checks` : allocateur traqué `assert_no_alloc` + panique sur allocation dans le callback en debug (§7)
- [x] Instrumentation : underruns (dépassements de budget + erreurs de stream) et charge du callback lissée, dans le snapshot (§3.6)
- [x] `app` : chargement CLI (2 pistes), workers de décodage, play/pause/seek/volumes/crossfader/master au clavier, état dans le titre de fenêtre
- [x] Tests : rendu offline du graphe (mêmes structs, hors cpal) — 5 tests d'intégration + WAV d'écoute optionnel (`OBER_WRITE_WAV=1`) ; les WAV de non-régression « golden » attendront un DSP stabilisé (M2)
- [x] Bench criterion : ~665 ns pour 2 decks à 128 frames, soit ~0,03 % du budget (cible < 20 %) (§7)
- [x] Méthode de mesure de latence documentée (`docs/latence.md`) ; buffer logiciel 256 frames = 5,33 ms
- [ ] **Validation matérielle** : session d'écoute réelle (2 pistes, aucun underrun affiché) et mesure de latence physique ≤ 10 ms — à faire sur la machine avec sortie audio active

**Sortie** : mix 2 pistes sans underrun, latence mesurée ≤ 10 ms.

## M2 — DSP

- [x] EQ 3 bandes biquad RBJ maison (low-shelf 250 Hz, peak 1 kHz, high-shelf 2,5 kHz), gains −26 → +6 dB ; **coefficients calculés hors callback** (`dsp::eq_coeffs`), les commandes portent les coefficients ; kill −∞ reporté au mapping M3 (§3.3)
- [x] Varispeed ±16 % (clavier limité à ±8 %), interpolation Hermite cubique 4 points directement (pas de premier jet linéaire) (§3.3)
- [x] Limiteur soft-clip master `tanh` (aussi appliqué au bus casque) (§3.3)
- [x] Stream 4 canaux (1/2 master, 3/4 casque) sur périphérique matché par nom ("DJControl" auto ou `device_match` de `ober.config.ron`) ; fallback stéréo périphérique par défaut ; le 4 canaux n'est jamais tenté sur le périphérique par défaut (cartes 5.1) (§3.2)
- [x] Cue mix casque : balance cue↔master + gain casque, tap cue post-gain deck / pré-crossfader (§3.3)
- [x] Tests : réponse des biquads vs formule théorique (y compris filtrage temporel), Hermite, soft-clip, varispeed (fréquence transposée), kill EQ vs réponse théorique, limiteur borné, routage cue 4 canaux indépendant du crossfader (§7)
- [x] Bench chaîne complète : ~6,6 µs / bloc de 128 frames en 4 canaux (~0,25 % du budget, cible < 20 %)
- [ ] **Validation matérielle sur l'Inpulse** : carte détectée, stream 4 canaux ouvert (risque cpal/ALSA §9 — si échec : plan B 2 périphériques), pré-écoute au casque à l'oreille, mesure de latence physique — checklist TESTING.md

**Sortie** : pré-écoute casque fonctionnelle sur l'Inpulse.

## M3 — MIDI in

- [x] Thread MIDI dédié (midir) ; hot-plug par polling (~1,5 s) : déconnexion détectée, reconnexion auto, jamais de crash au débranchement ; état shift/toggles conservé entre connexions (§5.1)
- [x] **Chemin court** : événement traduit → `EngineCommand` poussé dans un **ring SPSC dédié** du moteur, directement depuis le callback midir ; copie de chaque événement vers Bevy pour l'affichage (§5.1)
- [x] Schéma de mapping complet : courbes (`Linear`, `DbLinear` — sortie en dB pour l'EQ), encodages relatifs (`SignedBit`, `TwosComplement`), couche Shift avec repli sur la couche de base, champ `init` (messages bruts à la connexion) (§5.2)
- [x] Moteur de mapping générique `InputSpec → Action` (`midi::MappingEngine`) — aucun code Hercules dans le moteur ; l'init LEDs passe par le champ déclaratif `init` (le trait `ControllerBackend` attendra un contrôleur qui exige du SysEx) (§5.2–5.3)
- [x] Validation au chargement, erreurs lisibles cumulées : doublons (input, shift), canaux/notes hors plage, courbes invalides, `device_match` vide (§5.2)
- [x] `mappings/hercules_inpulse_200_mk2.ron` rempli depuis le mapping Mixxx de l'Inpulse 200 : transport, cue, PFL, load, crossfader, volumes, EQ 2 bandes (pas de médium sur ce contrôleur), pitch (MSB), jogs déclarés (exploités au M4), init `0xB0 0x7F 0x7F` — **codes à confirmer sur le MK2 avec midi-probe** (§5.3)
- [x] Cue point à sémantique vinyle dans le moteur (`CuePress`/`CueRelease` : pose/retour/pré-écoute tenue) + tests offline
- [x] Tests : table événement→action sur le mapping Hercules livré (toggle, momentary, NoteOff/vel 0, courbes dB aux butées, jogs relatifs, messages inconnus/tronqués), couche shift générique, encodages, routage chemin court (§7)
- [x] Checklist manuelle contrôleur détaillée dans `TESTING.md` (§7)
- [ ] **Validation matérielle sur le MK2** : chaque contrôle de la checklist TESTING.md, correction des codes RON si écart avec l'Inpulse 200 v1
- [ ] **Spike parallèle (M3–M4)** : prototype waveform shader — texture min/max/RMS uploadée une fois, scroll/zoom par uniforms (dérisquage M6, §9)

**Sortie** : tous faders/potards/boutons opérants.

## M4 — Jogs

- [ ] Bend (bord du jog) : offset de vitesse proportionnel à la vélocité de rotation, retour progressif à la vitesse nominale (§3.5)
- [ ] Scratch (surface touchée) : ticks relatifs → vélocité cible (fenêtre glissante 10–20 ms) → asservissement de la vitesse par passe-bas (τ ≈ 5 ms) ; rampe de relâchement 50–200 ms configurable (§3.5)
- [ ] Tous les paramètres (sensibilité, ticks/tour, courbes) dans le mapping RON — rien en dur (§3.5)
- [ ] Itérations à l'oreille sur le matériel, comparaison avec Mixxx (§9)

**Sortie** : scratch propre à l'oreille, pas d'artefacts (pas de son "escalier").

## M5 — Feedback + analyse

- [ ] Schéma RON `feedback` + moteur `StateChange → MIDI out` : LEDs play/cue, VU ; réserver les états du beatmatch guide (v0.2) dans l'enum (§5.2–5.3)
- [ ] BPM + beatgrid offline : onsets par flux d'énergie spectrale (rustfft, fenêtres 1024/hop 512) → autocorrélation/histogramme 60–200 BPM (résolution 0,01) → phase du premier beat ; grille fixe (§4.2)
- [ ] Waveform summary 3 bandes, ~1000 points/s, min/max/RMS (préparation du rendu M6) (§4.2)
- [ ] Bus d'analyseurs temps réel branché sur le tap audio ; v0.1 : niveaux RMS/peak pour les VU ; canal `AnalysisFrame` → Bevy (§4.2)
- [ ] Piste jouable dès la fin du décodage, beatgrid livré ensuite (asynchrone) (§4.2)
- [ ] Corpus de test BPM : clicks générés + extraits réels à tempo connu, tolérance ±0,1 BPM (§7)

**Sortie** : LEDs synchronisées, BPM ±0,1 sur le corpus.

## M6 — UI

- [ ] Module `theme` : tokens de couleur sémantiques, échelle typo, rayons, espacements, courbes d'easing centralisées ; consommé par les materials **et** le style egui (§6.2)
- [ ] Fonts : Inter + Phosphor Icons — récupérer le module `fonts.rs` des projets internes et les assets dans `assets/fonts/` (§6.2)
- [ ] Waveforms en shader WGSL : mipmaps min/max/RMS (1×/4×/16×) uploadées une fois au chargement, scroll/zoom par uniforms — **aucune régénération de mesh par frame** (§6.1)
- [ ] Position affichée extrapolée (`position + vitesse × Δt`), correction douce sans snap (§6.1)
- [ ] VU-mètres par instancing (quad + uniforms), beatgrid en surimpression, tête de lecture fixe centrée, zoom molette (§6.1/§6.3)
- [ ] Écran unique complet : 2 waveforms, panneaux deck (titre/BPM/temps restant, play/cue, sliders fallback souris, indicateur cue), section centrale (crossfader, VU master, gains casque), barre d'état (périph audio, contrôleur, underruns, charge CPU audio, fps) (§6.3)
- [ ] Les interactions UI émettent les mêmes `Action` que le MIDI — un seul chemin (§6.4)
- [ ] `bevy_egui` pour les panneaux secondaires uniquement (préférences, debug) — **valider la compat bevy_egui 0.41 ↔ bevy 0.19 avant usage** ; file picker `rfd` (§6.1/§6.3)
- [ ] Ring texture du spectrogramme préparée (structure en place, activation v0.2) (§6.1)
- [ ] Mode idle : 10 fps via `WinitSettings` après > 5 s sans lecture ni interaction, retour immédiat au framerate natif ; le thread audio n'est jamais affecté ; mesurer sur laptop (§6.5)
- [ ] Support écrans 120/144 Hz : animations basées sur le temps réel, jamais sur le compteur de frames (§6.1)

**Sortie** : session de mix complète au contrôleur, framerate natif stable, frame CPU+GPU < 8 ms.

---

## Chantiers transverses (valables à chaque jalon)

- Frontière Bevy : `engine`/`decode`/`analysis`/`midi`/`mapping` sans dépendance Bevy — vérifiée en CI à chaque push (§1.4/§2.4)
- Règles du callback audio (§2.2) : aucune allocation, aucun lock, aucune I/O, aucun blocage — revue systématique + `rt-checks` en debug
- `cargo clippy -D warnings` + `cargo fmt --check` + tests 3 OS verts avant merge
- Épinglage Bevy : toute montée de version est une tâche planifiée dédiée (~1×/an), jamais au fil de l'eau (§1.4)
- Pas de crate Bevy tierce non activement maintenue (§1.4)

## Après le POC (v0.2+, hors périmètre v0.1)

- Streaming par chunks (pistes > 15 min)
- Spectrogramme temps réel activé (infrastructure posée en M6) ; FFT en compute shader
- Beatmatch guide (LEDs tempo/phase)
- Keylock / time-stretch, sync/master tempo, effets, bibliothèque musicale, enregistrement du mix
- Autres contrôleurs (l'architecture mapping générique le permet déjà)
