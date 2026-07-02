# Constitution of Development — the rules of work

How we work on **ober**. These rules are stable and binding; the
[ROADMAP](ROADMAP.md) holds the evolving plan and status, the
[README](README.md) holds setup and commands, and
[docs/SPECS.md](docs/SPECS.md) holds the product contract (a verbatim copy —
never edited). The specs say *what* to build; this document says *how* to
work on the code.

Working language of **everything written for the project** is **English** —
not only code, comments and docs, but also commit messages, tag and release
notes, branch names, PR titles and descriptions, and issues: every artifact
that lands in the repo or its infrastructure. French is fine only in *live
discussion*, never in anything persisted. Two legacy exceptions:
`docs/SPECS.md` (a verbatim French contract copy, excepted by design) and
pre-existing French comments in the code, which migrate to English whenever
the code they annotate is touched. Cite a rule by number ("cf. Rule 2") and
a spec by section ("§2.2").

---

## Rule 1 — The audio callback is pure (the cardinal invariant)

Everything that runs inside the cpal callback —
`engine::graph::AudioGraph::process` and everything it calls — obeys:
**no allocation, no lock, no I/O, no blocking call, no deallocation**
(`Arc`s leave through the reclaim channel). Exchanges with the rest of the
world go exclusively through lock-free channels (`rtrb` rings,
`triple_buffer`, the audio-tap ring). The `rt-checks` feature
(`assert_no_alloc`) enforces this in debug; the `callback` bench measures
the budget.

**Why.** An xrun is audible. The audio thread has a hard budget on the
order of a millisecond, and the only way to hold it *always* is to bound
the worst case — an allocation, a mutex or an I/O has no bounded worst
case.

**Anchored in.** `crates/engine/src/graph.rs` (`AudioGraph::process`), the
`rt-checks` feature, `crates/engine/benches/callback.rs`; specs §2.2.

---

## Rule 2 — The Bevy boundary

`engine`, `decode`, `analysis`, `midi`, `mapping` **never** depend on Bevy,
directly or transitively; only `crates/app` touches it. The boundary is
enforced in CI (`scripts/check-bevy-boundary.sh`).

**Why.** The engine stays testable and benchable without a window or a
GPU, and the Bevy migration (Rule 4) touches a single crate. The opposite —
Bevy leaked into the engine — is the coupling you never undo.

**Anchored in.** `scripts/check-bevy-boundary.sh`,
`.github/workflows/ci.yml` (job `bevy-boundary`); specs §1.4, §2.4.

---

## Rule 3 — `cargo fmt` is authoritative; the tree stays clippy-clean

There is **no `rustfmt.toml`** — the default formatter decides, and layout
is not a review battleground. Format *before* committing: every commit
leaves `cargo fmt --all --check` clean and
`cargo clippy --workspace --all-targets -- -D warnings` warning-free.

**Why.** Formatting and lint debates are pure friction; delegating them to
the tools keeps reviews about substance.

**Anchored in.** `.github/workflows/ci.yml` (jobs `fmt`, `clippy`);
`Cargo.toml` (edition 2024).

---

## Rule 4 — Bevy is pinned to an exact version

`bevy = "=0.19.0"`: any version bump is a dedicated, planned task
(~1×/year), never done in passing. More broadly, workspace versions live in
the single `[workspace.dependencies]` registry; a crate pins nothing
locally.

**Why.** Bevy's API churn is the project's main maintenance risk (specs
§1.4); an unplanned bump in the middle of a milestone turns a fix into a
migration.

**Anchored in.** `Cargo.toml` (`[workspace.dependencies]`); CLAUDE.md
("Known pitfalls"); specs §1.4.

---

## Rule 5 — DSP coefficients are computed outside the callback

Commands sent to the engine carry **ready-to-use values** (biquad
coefficients, linear gains, ratios); the callback only applies them. No
trigonometry, no costly conversion on the real-time side.

**Why.** This is the operational corollary of Rule 1: coefficient
computation has neither a bounded budget nor a reason to live on the audio
thread. And the intent/execution split keeps the engine drivable by any
control surface.

**Anchored in.** `crates/engine/src/command.rs`, `crates/engine/src/dsp/`;
specs §2.2.

---

## Rule 6 — One single path for intents

Controller, keyboard and mouse all emit `mapping::Action`s, routed to the
engine by `midi::to_engine_command` (`app::emit_control`). No UI → engine
shortcut.

**Why.** Three paths diverge silently; a single path makes the keyboard a
test bench for the full mapping, with no controller plugged in.

**Anchored in.** `crates/midi/src/route.rs` (`to_engine_command`),
`crates/app/src/main.rs` (`emit_control`), `crates/mapping`; specs §6.4.

---

## Rule 7 — The design system is a single source

Colors, spacings, easings: only through `app/src/theme.rs`. Layout is
expressed in **window fractions** (`theme::layout::bands()`), never in
pixels hard-coded inside a UI module.

**Why.** Visual consistency and fluid resizing do not survive scattered
local constants; a theme adjustment must happen in one place.

**Anchored in.** `crates/app/src/theme.rs`; `waveform.rs`, `vu.rs`,
`hud.rs`, `widgets.rs`, `browser.rs` (consumers).

---

## Rule 8 — Every feature ships with a test; hardware has its checklist

Unit tests per module for pure logic; the offline render
(`offline_render`) and the `callback` bench cover the engine end to end.
Whatever requires the physical controller (real MIDI codes, headphone
cueing, latency, LEDs, scratching by ear) follows the **manual checklist**
in [TESTING.md](TESTING.md) — no hardware test in CI.

**Why.** What is not tested regresses; and pretending to test hardware in
CI produces tests that test nothing. The honest boundary is explicit:
automated up to the edge of the hardware, checklist beyond it.

**Anchored in.** crate tests (`cargo test --workspace`),
`crates/engine/tests/offline_render.rs`, TESTING.md;
`.github/workflows/ci.yml`.

---

## Rule 9 — Document the *why*, cite the spec

Doc-comments justify non-obvious decisions and cross-reference the
relevant spec section ("§3.3") or rule — they explain *why*, not *what*
the code plainly says. A new invariant is added to the constitution and
cited, not buried in a comment. Operational knowledge goes into
ROADMAP/README/TESTING/docs/, never into chat messages.

**Why.** The reasoning is the part that rots silently; recording it (and
linking it to the binding rule) is what keeps the next change honest.

**Anchored in.** Pervasive across `crates/` (the doc-comment style is the
norm); CLAUDE.md ("Conventions").

---

## Rule 10 — Commit hygiene

Conventional Commits (`feat:`, `fix:`, `chore:`, `docs:`, `test:`, `ci:`,
`refactor:`, `perf:`). Never commit a tree that does not pass
fmt/clippy/tests/boundary (Rules 2–3). Every milestone or fix ends
**committed, pushed, CI green** (`gh run list`), with ROADMAP/TESTING
updated.

**Why.** A readable, typed history is what makes a regression bisectable
and a changelog writable; a green CI on every commit is what makes any
commit a resumable starting point.

**Anchored in.** `git log` (conventional style from the quality baseline
onward); CLAUDE.md ("Conventions").

---

## Rule 11 — Version is semver of the shipped artifact; tag on request or before a minor bump

`[workspace.package].version` (Cargo.toml) is the single source of truth,
and it is **semver of the shipped artifact** (the `ober` binary) — not a
commit counter:

- `fix:` → **patch** (`x.y.Z`): a backwards-compatible bug fix.
- `feat:` → **minor** (`x.Y.0`): new backwards-compatible capability.
- a breaking change (config/mapping format, CLI) → **major** (`X.0.0`).
- `chore:` / `docs:` / `test:` / `ci:` / `refactor:` that do **not** change
  the shipped binary → **no bump** (dev-only tooling — benches, CI, the
  devShell — is not part of what is versioned).

Every release **tag** is `v<that exact version>` (e.g.
`Cargo.toml = 0.2.1` → tag `v0.2.1`); the release CI **fails** the run if
they disagree. The `-dev` suffix of the current version (`0.1.0-dev`) marks
a line that never shipped: it drops at the first release, and the
`v[0-9]+.[0-9]+.[0-9]+` trigger mechanically cannot match a pre-release.

A tag is cut in two cases: **(a) on explicit request** — any version, a
`fix:` patch included, can be released when you decide it ships; and
**(b) before a minor/major bump** — when rolling `Cargo.toml` onto a new
minor or major line, first tag the *outgoing* version if it is not already
tagged, so the last state of the closing line is captured. A patch you
don't ask to release simply lands in `Cargo.toml` untagged and rides along
under whichever tag captures its line. Pushing a `vX.Y.Z` tag is the
**only** trigger for a release — build (Linux/Windows/macOS, `dist`
profile) → archives → GitHub Release. To pin an arbitrary build, the
**git SHA** is enough (optionally as semver build metadata,
`0.1.0+a1b2c3d`) — the version field tracks *behavior*, not every commit.

The tag is **annotated** (`git tag -a`), and its message **is the
changelog**: a hand-written description of the evolutions since the
previous tag. The release CI lifts that message into the release notes
(and appends GitHub's auto "Full Changelog" link), so a lightweight tag —
or an annotated tag with an empty message — is a defect, not a shortcut.

**Why.** Semver keyed to the *artifact's behavior* — not the repo's
activity — is what lets a reader map a release to what actually changed;
bumping the patch on every chore turns the version into a meaningless
commit counter. And an auto-generated commit list is not a changelog — the
hand-written "what changed and why" is the part a reader actually needs.

**Anchored in.** `Cargo.toml` (`[workspace.package].version`);
`.github/workflows/release.yml` (`version-check` guard, the `dist` build
matrix); `Cargo.toml` (`[profile.dist]`).
