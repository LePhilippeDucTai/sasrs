# J01 — Base verte, CI bloquante, harnais CLI, roadmap réconciliée

Goal: repartir d'une base réellement verte (fmt, clippy, tests par défaut et graphics), poser la CI GitHub Actions bloquante qui arbitre toutes les parts suivantes, un harnais de tests CLI réels, et une roadmap unique (ex-M46–M66 remappés, règles agents écrites).
Depends on: — · Orchestrator: sonnet/high

## J01-P1 — Baseline lint et graphics verte

```yaml
id: J01-P1
kind: implement
tier: T5
size: L
depends_on: []
files:
  - src/
  - tests/snapshot.rs
acceptance:
  - "cargo fmt --check"
  - "cargo clippy --all-targets -- -D warnings"
  - "cargo clippy --all-targets --features graphics -- -D warnings"
  - "cargo test -p sasrs --features graphics --test snapshot"
  - "test -z \"$(git status --porcelain -- tests/snapshots)\""
```

### Scope
- Supprimer les 52 `use super::super::*;` inutilisés des modules de test (`cargo clippy` les liste ; `cargo fix --lib -p sasrs --tests` possible) ; aucune autre modification de code de production.
- Corriger toute autre alerte `clippy -D warnings` sous `--features graphics` si elle apparaît (champs `cfg_attr(not(feature="graphics"), allow(dead_code))` réellement lus ou retirés).
- `tests/snapshot.rs` : exclure sous `graphics` les fixtures qui écrivent des images, dont `m36/plots.sas` ; remplacer la liste de préfixes de noms par une liste explicite de chemins relatifs commentée.
- Ne pas : régénérer ou modifier un `.snap` ; toucher à `Cargo.toml`.

### Context
- Baseline : `PLAN.md` § Checks ; journaux `cargo clippy` (52 erreurs `unused import`).
- Snapshot en échec sous graphics : `tests/snapshots/snapshot__fixtures@m36__plots.sas.snap` (NOTEs « deferred » du build par défaut).

## J01-P2 — CI GitHub Actions bloquante et scripts de vérification

```yaml
id: J01-P2
kind: implement
tier: T4
size: M
depends_on: [J01-P1]
files:
  - .github/workflows/ci.yml
  - scripts/check.sh
  - scripts/ci-status.sh
  - rust-toolchain.toml
acceptance:
  - "test -x scripts/check.sh && test -x scripts/ci-status.sh && bash -n scripts/check.sh && bash -n scripts/ci-status.sh"
  - "grep -q 'ci-ok:' .github/workflows/ci.yml && grep -q -- '--features graphics' .github/workflows/ci.yml && grep -q -- '--features s3' .github/workflows/ci.yml"
  - "grep -q 'INSTA_UPDATE' .github/workflows/ci.yml && grep -q 'git status --porcelain' .github/workflows/ci.yml && grep -q 'workflow_dispatch' .github/workflows/ci.yml"
  - "grep -q 'channel = \"1.98.1\"' rust-toolchain.toml && grep -q 'clippy' rust-toolchain.toml"
  - "scripts/check.sh lint"
  - "scripts/ci-status.sh --help | grep -q 'consolidation'"
```

### Scope
- `ci.yml` : déclencheurs push (toutes branches), pull_request, workflow_dispatch ; jobs `fmt`, `clippy` (défaut, graphics, s3 ; `--all-targets -D warnings`), `test` (`cargo test -p sasrs`), `test-graphics`, `test-s3` (`cargo test --features s3 --lib`, sans secrets) ; env `CI=true`, `INSTA_UPDATE=no` ; après chaque job de test, `git status --porcelain` vide sinon échec (aucun `.snap.new`) ; cache `Swatinem/rust-cache` ; job agrégé `ci-ok` (`needs:` tous les jobs, échoue si l'un échoue) = seul check à rendre obligatoire.
- `rust-toolchain.toml` : `channel = "1.98.1"`, components rustfmt + clippy (les lints ne changent plus avec la toolchain).
- `scripts/check.sh lint|test|build|all` : mêmes commandes que la CI, même env ; code retour non nul au premier échec.
- `scripts/ci-status.sh [branche]` (défaut consolidation) : via `gh run list --workflow ci.yml --branch <b>`, exit 0 seulement si le dernier run porte `headSha == git rev-parse origin/<b>` et `conclusion == success` ; affiche l'état ; `--help` décrit l'usage.
- Ne pas : ajouter de job de release (J06-P5) ni de test Python (J06-P4).

### Context
- Commandes et état : `PLAN.md` § Checks ; features `s3`/`graphics` dans `Cargo.toml`.
- insta : avec `CI=true` aucun `.snap.new` n'est écrit, un écart échoue.

## J01-P3 — Harnais de tests CLI réels

```yaml
id: J01-P3
kind: implement
tier: T4
size: S
depends_on: []
files:
  - tests/cli.rs
acceptance:
  - "grep -q 'CARGO_BIN_EXE_sasrs' tests/cli.rs"
  - "cargo test -p sasrs --test cli 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- `tests/cli.rs` lance le binaire (`env!("CARGO_BIN_EXE_sasrs")`) dans un tempdir, avec un helper réutilisable (écrit le `.sas`, exécute, rend stdout/stderr/code).
- Cas (comportement actuel correct uniquement) : programme propre → 0 ; WARNING → 1 ; ERROR → 2 ; script illisible → 2 et message stderr ; `--log`/`--print` écrivent les fichiers attendus ; défauts log→stderr, listing→stdout ; `--version` contient la version de `Cargo.toml` ; chemins LIBNAME relatifs résolus depuis le dossier du script.
- Ne pas : tester les échecs d'écriture (J02-P1 corrige puis teste) ; modifier `src/`.

### Context
- `src/main.rs` (84 l.), `src/lib.rs` (`run`, `RunOutcome`, calcul du code retour).
- Données : `tests/common/mod.rs::write_class_parquet`.

## J01-P4 — Roadmap réconciliée et règles de développement par agents

```yaml
id: J01-P4
kind: implement
tier: T5
size: M
depends_on: []
files:
  - PLAN.md
  - PROGRESS.md
  - README.md
  - CONTRIBUTING.md
  - .github/pull_request_template.md
  - .claude/skills/sasrs-impl/SKILL.md
acceptance:
  - "grep -q 'docs/plans/consolidation' PLAN.md && grep -q 'docs/plans/consolidation' PROGRESS.md && grep -q 'docs/plans/consolidation' .claude/skills/sasrs-impl/SKILL.md"
  - "grep -q '^## Correspondance M46–M66 → consolidation' PLAN.md"
  - "grep -q 'validé contre référence' CONTRIBUTING.md && grep -q 'approximation documentée' CONTRIBUTING.md && grep -q 'known-divergence' CONTRIBUTING.md"
  - "grep -q 'Snapshot:' CONTRIBUTING.md && grep -q 'Snapshot' .github/pull_request_template.md"
  - "! grep -q 'todo!()' README.md && grep -q 'docs/plans/consolidation' README.md"
```

### Scope
- `PLAN.md` et `PROGRESS.md` racine : bandeau en tête « feuille de route gelée le 2026-09-23 ; suite : `docs/plans/consolidation/` » ; section `## Correspondance M46–M66 → consolidation` (M50 → J07-P5 ; M46–M49, M51–M66 → `docs/roadmap/avancee.md`, J08-P5) ; rien d'autre n'est réécrit.
- `SKILL.md` de sasrs-impl : marquée remplacée par `/milestone-continue consolidation` ; garder le paragraphe du watchdog.
- `CONTRIBUTING.md` : règles de l'issue #11 (la CI arbitre, jamais la parole d'un agent ; l'implémenteur n'écrit pas le seul oracle ; reproducer + test de non-régression par changement de sémantique ; snapshot ≠ preuve de conformité ; changement de snapshot justifié `Snapshot: <fixture> — <raison>` ; contrat « reconnu mais ignoré » ; statut `known-divergence`) ; définitions des quatre états : implémenté · validé contre référence · approximation documentée · non supporté ; commandes `scripts/check.sh`.
- `.github/pull_request_template.md` : reproducer, tests, oracle et provenance, snapshots modifiés et justification, CI.
- `README.md` : paragraphe Status pointant vers `docs/plans/consolidation/`, légende sans `todo!()` (🔴 = reconnu, non supporté, diagnostic explicite), phrase périmée sur TTEST/NPAR1WAY retirée ; pas d'audit des tableaux (J02-P7, J03-P7).

### Context
- `PLAN.md` racine § Phase G (M46–M66), `PROGRESS.md` en-tête, `.claude/skills/sasrs-impl/SKILL.md`.
- Issue #11 § « Exigences spécifiques » (`gh issue view 11`).

## J01-P5 — Protection de main par le check CI

```yaml
id: J01-P5
kind: implement
tier: T5
size: S
depends_on: [J01-P2, J01-P4]
files:
  - CONTRIBUTING.md
acceptance:
  - "gh api repos/LePhilippeDucTai/sasrs/branches/main/protection --jq '.required_status_checks.contexts[]' | grep -qx 'ci-ok'"
  - "gh api repos/LePhilippeDucTai/sasrs/branches/main/protection --jq '.allow_force_pushes.enabled' | grep -qx false"
  - "grep -q 'ci-ok' CONTRIBUTING.md"
```

### Scope
- `gh api -X PUT repos/LePhilippeDucTai/sasrs/branches/main/protection` : `required_status_checks` {strict: false, contexts: ["ci-ok"]}, `enforce_admins` false (le watchdog Hermes pousse encore sur main), `required_pull_request_reviews` null, `restrictions` null, `allow_force_pushes` false, `allow_deletions` false (autorisé par l'utilisateur, D-001).
- `CONTRIBUTING.md` : main protégé, fusion consolidation → main par PR avec `ci-ok` vert.
- Ne pas : protéger `consolidation` ; modifier d'autres réglages du dépôt.

### Context
- Job `ci-ok` : `.github/workflows/ci.yml` (J01-P2). D-001 dans `DECISIONS_LOG.md`.

## J01-P6 — Review J01

```yaml
id: J01-P6
kind: review
tier: T2
size: S
depends_on: [J01-P1, J01-P2, J01-P3, J01-P4, J01-P5, J01-P7]
files: []
acceptance:
  - "cargo fmt --check"
  - "cargo clippy --all-targets -- -D warnings"
  - "cargo clippy --all-targets --features graphics -- -D warnings"
  - "cargo test -p sasrs --features graphics --test snapshot"
  - "test -z \"$(git status --porcelain -- tests/snapshots)\""
  - "test -x scripts/check.sh && test -x scripts/ci-status.sh && bash -n scripts/check.sh && bash -n scripts/ci-status.sh"
  - "grep -q 'ci-ok:' .github/workflows/ci.yml && grep -q -- '--features graphics' .github/workflows/ci.yml && grep -q -- '--features s3' .github/workflows/ci.yml"
  - "grep -q 'INSTA_UPDATE' .github/workflows/ci.yml && grep -q 'git status --porcelain' .github/workflows/ci.yml && grep -q 'workflow_dispatch' .github/workflows/ci.yml"
  - "grep -q 'channel = \"1.98.1\"' rust-toolchain.toml && grep -q 'clippy' rust-toolchain.toml"
  - "scripts/check.sh lint"
  - "scripts/ci-status.sh --help | grep -q 'consolidation'"
  - "grep -q 'CARGO_BIN_EXE_sasrs' tests/cli.rs"
  - "cargo test -p sasrs --test cli 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "grep -q 'docs/plans/consolidation' PLAN.md && grep -q 'docs/plans/consolidation' PROGRESS.md && grep -q 'docs/plans/consolidation' .claude/skills/sasrs-impl/SKILL.md"
  - "grep -q '^## Correspondance M46–M66 → consolidation' PLAN.md"
  - "grep -q 'validé contre référence' CONTRIBUTING.md && grep -q 'approximation documentée' CONTRIBUTING.md && grep -q 'known-divergence' CONTRIBUTING.md"
  - "grep -q 'Snapshot:' CONTRIBUTING.md && grep -q 'Snapshot' .github/pull_request_template.md"
  - "! grep -q 'todo!()' README.md && grep -q 'docs/plans/consolidation' README.md"
  - "gh api repos/LePhilippeDucTai/sasrs/branches/main/protection --jq '.required_status_checks.contexts[]' | grep -qx 'ci-ok'"
  - "gh api repos/LePhilippeDucTai/sasrs/branches/main/protection --jq '.allow_force_pushes.enabled' | grep -qx false"
  - "grep -q 'ci-ok' CONTRIBUTING.md"
  - "cargo test -p sasrs"
  - "cargo build --features graphics && cargo build --features s3"
  - "cargo fmt --check && cargo clippy --all-targets -- -D warnings"
```

### Scope
Review the merged milestone diff with verify-before-done, code-review, test-design. Report; change nothing.

## J01-P7 — CI : dépendances système fontconfig pour le profil graphics

```yaml
id: J01-P7
kind: implement
tier: T5
size: S
depends_on: [J01-P2, J01-P2b]
files:
  - .github/workflows/ci.yml
acceptance:
  - "python3 -c \"import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))\""
  - "test \"$(grep -c 'pkg-config' .github/workflows/ci.yml)\" -ge 2 && test \"$(grep -c 'libfontconfig1-dev' .github/workflows/ci.yml)\" -ge 2 && test \"$(grep -c 'libfreetype-dev' .github/workflows/ci.yml)\" -ge 2 && test \"$(grep -c 'fonts-liberation' .github/workflows/ci.yml)\" -ge 2"
  - "CI=true INSTA_UPDATE=no cargo clippy --all-targets --features graphics -- -D warnings"
  - "CI=true INSTA_UPDATE=no cargo test -p sasrs --features graphics --test snapshot"
  - "test -z \"$(git status --porcelain)\""
```

### Scope
- Les 2 jobs `clippy (graphics)` et `test (graphics)` reçoivent, avant toute commande cargo, un step d'installation des dépendances système qu'exige `yeslogic-fontconfig-sys` sur le runner : `pkg-config`, `libfontconfig1-dev`, `libfreetype-dev`, `fonts-liberation` (rendu texte). Via `sudo apt-get update && sudo apt-get install -y …` (mécanisme officiel d'ubuntu-latest), un step par job (aucun partage de steps possible entre jobs GitHub Actions).
- Interdits : toucher toute autre ligne de `ci.yml` (autres jobs, commandes cargo, env, `ci-ok`, déclencheurs, cache) ; modifier `Cargo.toml`/`Cargo.lock` (la dépendance graphics reste réelle et bloquante) ; affaiblir un job ou retirer un job.
- Les acceptations 3 et 4 se rejouent dans le conteneur de développement (l'hôte n'a pas pkg-config, cf. scripts/check.sh §8) : elles prouvent que la liste de paquets couvre bien les besoins du build.rs de la crate.
- Arbitrage final (orchestrateur, après push) : `scripts/ci-status.sh consolidation` vert sur le sha mergé — la CI arbitre (CONTRIBUTING §1).

### Context
- Runs CI échouées : 35882867341, 35883283504 (`failed to run custom build command for yeslogic-fontconfig-sys v6.0.1`, panic build.rs:8 — pkg-config absent du runner). Tous les autres jobs sont verts. D-002 dans `DECISIONS_LOG.md`.

