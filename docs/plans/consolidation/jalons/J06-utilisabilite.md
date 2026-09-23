# J06 — Utilisabilité : API Session, Python, distribution, documentation

Goal: une personne externe installe sasrs depuis un environnement vierge, exécute un programme, récupère une table produite et comprend une erreur sans lire PLAN.md ni PROGRESS.md ; API Rust stable autour d'une session, wrapper Python durci, releases versionnées immuables (issue #8).
Depends on: J05 · Orchestrator: sonnet/high

## J06-P1 — Façade publique `sasrs::api` (ADR)

```yaml
id: J06-P1
kind: implement
tier: T2
size: M
depends_on: []
files:
  - docs/adr/0002-api-publique.md
  - src/api.rs
  - src/api/
  - src/lib.rs
  - tests/api.rs
acceptance:
  - "grep -q '^## Décision' docs/adr/0002-api-publique.md"
  - "cargo test -p sasrs --test api 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "grep -q 'pub mod api' src/lib.rs"
```

### Scope
- ADR : surface stable (module `sasrs::api`), politique de compatibilité (semver, modules internes `#[doc(hidden)]` plutôt que cassés), types exposés.
- `api::Session` : `new(Options)`, `submit(&str) -> Submission` (log du programme, compteurs, code retour) plusieurs fois sur la même session, `register_dataset(libref, name, DataFrame, métadonnées optionnelles)`, `dataset(libref, name) -> (DataFrame, Vec<VarMeta>)`, `close() -> CloseReport` (finalise les destinations ODS, supprime WORK) ; erreurs typées.
- `run()` et `RunOutcome` conservés (réimplémentés sur la façade), sortie inchangée : zéro snapshot modifié.
- `tests/api.rs` : deux soumissions partageant WORK et macros, injection puis relecture avec métadonnées, `close`.

### Context
- `src/lib.rs`, `src/session.rs:109-306`, `src/executor/mod.rs:82`, `src/library/mod.rs:36-148`, `src/dataset.rs:25-117`.

## J06-P2 — Diagnostics structurés et fichiers produits

```yaml
id: J06-P2
kind: implement
tier: T3
size: M
depends_on: [J06-P1]
files:
  - src/log.rs
  - src/session.rs
  - src/api.rs
  - src/api/
  - src/ods_graphics.rs
  - src/graphics/
  - tests/api.rs
acceptance:
  - "cargo test -p sasrs --test api diagnostics 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "cargo test -p sasrs --test api produced_files 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- `LogWriter` conserve une liste de diagnostics `{severity: Note|Warning|Error, line: Option<u32>, step, message}` en plus du texte (texte inchangé octet pour octet) ; exposée par `Submission` et `RunOutcome` (champ ajouté).
- Registre des fichiers produits (ODS HTML/RTF/PDF/Excel, images) : chemin, type, proc ; exposé par `Submission`/`CloseReport`.
- Tests préfixés `diagnostics` et `produced_files` dans `tests/api.rs`.
- Zéro snapshot modifié.

### Context
- `src/log.rs:12-96`, `src/session.rs:160-437`, sites d'écriture d'images (`graphics/render.rs`, procs graphiques), ADR 0002.

## J06-P3 — Exemples autonomes testés

```yaml
id: J06-P3
kind: implement
tier: T4
size: M
depends_on: [J06-P2]
files:
  - examples/
  - tests/examples.rs
acceptance:
  - "cargo run --example quickstart 2>&1 | grep -q 'OK'"
  - "cargo test -p sasrs --test examples 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- `examples/data/` (CSV d'exemple), `examples/cli/analysis.sas` + sortie attendue documentée, `examples/quickstart.rs` (façade `api` : soumettre, relire une table, afficher un diagnostic ; imprime `OK` à la fin).
- `tests/examples.rs` exécute l'exemple CLI via le binaire et vérifie la table produite et le code retour.

### Context
- ADR 0002 ; `tests/cli.rs` (helper binaire).

## J06-P4 — Wrapper Python durci et testé

```yaml
id: J06-P4
kind: implement
tier: T3
size: M
depends_on: []
files:
  - python/
  - .github/workflows/ci.yml
acceptance:
  - "cd python && python3 -m unittest discover -s tests -v 2>&1 | grep -q '^OK'"
  - "grep -q 'unittest' .github/workflows/ci.yml"
  - "grep -qi 'lance un binaire\\|launches a binary' python/README.md"
```

### Scope
- `cli.py` : timeout réseau, message clair sans traceback (réseau absent, HTTP 404, plateforme non supportée, SHA invalide), cache indexé par SHA (plus de re-hachage à chaque run si un marqueur vérifié existe), verrou de fichier pour les premiers lancements concurrents, nettoyage des temporaires orphelins, repli sûr si `os.replace` échoue sous Windows et que le binaire en place est valide.
- `python/tests/` (unittest, bibliothèque standard, `urlopen` et plateforme simulés) : cache absent, cache corrompu, réseau absent, plateforme non supportée, deux lancements concurrents, SHA incorrect.
- `python/README.md` : explicite que le wrapper télécharge et lance un binaire ; limites.
- CI : job Python (3.9 et dernière) dans `ci-ok`.

### Context
- `python/src/sasrs_py/cli.py` (98 l.), watchdog Hermes (Risks) : les lignes SHA de `cli.py` sont réécrites de l'extérieur jusqu'à J06-P5.

## J06-P5 — Distribution versionnée et manifeste de build

```yaml
id: J06-P5
kind: implement
tier: T3
size: M
depends_on: [J06-P4]
files:
  - .github/workflows/release.yml
  - build.rs
  - Cargo.toml
  - src/main.rs
  - python/src/sasrs_py/cli.py
  - docs/release.md
acceptance:
  - "grep -q \"tags:\" .github/workflows/release.yml && grep -q 'SHA256SUMS' .github/workflows/release.yml && grep -q 'manifest.json' .github/workflows/release.yml"
  - "cargo run -- --version 2>&1 | grep -qE 'commit [0-9a-f]{7}|commit unknown'"
  - "grep -q 'build = \"build.rs\"' Cargo.toml || test -f build.rs"
```

### Scope
- `release.yml` sur tag `v*` : binaires linux x86_64, windows x86_64, macOS arm64 ; `SHA256SUMS` ; `manifest.json` (version, commit, features, rustc, cible) ; assets jamais remplacés (échec si le tag existe déjà).
- `build.rs` : commit et features embarqués ; `sasrs --version` → `sasrs <version> (commit <sha>, features: …)`.
- Wrapper Python épinglé sur une version (`v<version>`) avec SHA issus de `SHA256SUMS` embarqués dans le paquet ; plus de réécriture externe nécessaire.
- `docs/release.md` : procédure de release, immutabilité, arrêt du watchdog Hermes (R-001).
- Ne pas : publier une release réelle (le tag est posé par l'utilisateur).

### Context
- `python/src/sasrs_py/cli.py`, release actuelle `python-v0.1.0` (asset remplacé en place), `Cargo.toml` version 0.1.0.

## J06-P6 — Plan d'une API Python native

```yaml
id: J06-P6
kind: implement
tier: T4
size: S
depends_on: []
files:
  - docs/adr/0003-api-python.md
acceptance:
  - "grep -q '^## Décision' docs/adr/0003-api-python.md && grep -qi 'pyo3' docs/adr/0003-api-python.md && grep -qi 'DataFrame' docs/adr/0003-api-python.md"
```

### Scope
- ADR : faisabilité pyo3/maturin au-dessus de `sasrs::api`, échange de DataFrames (Arrow/Polars, pandas), roues par plateforme, coût de build Polars, coexistence avec la voie CLI simple ; recommandation et découpage en items pour la feuille de route (J08-P5).
- Ne pas : écrire de code.

### Context
- ADR 0002 (J06-P1, lire s'il est fusionné ; sinon `src/lib.rs`), `python/`.

## J06-P7 — Parcours nouvel utilisateur

```yaml
id: J06-P7
kind: implement
tier: T5
size: M
depends_on: [J06-P3, J06-P5, J06-P6]
files:
  - README.md
  - docs/getting-started.md
acceptance:
  - "grep -q 'docs/getting-started.md' README.md && grep -q 'examples/quickstart.rs' docs/getting-started.md"
  - "grep -qi 'retrieve\\|récupérer' docs/getting-started.md && grep -q 'Exit codes' README.md"
```

### Scope
- `docs/getting-started.md` : installation depuis un environnement vierge (binaire de release, `cargo install`, Python), premier programme, récupérer une table (CLI `--work` + parquet, Rust `api`, Python), lire un diagnostic, codes retour.
- `README.md` : installation réécrite (voies réelles uniquement), bibliothèque (exemple compilable renvoyant vers `examples/quickstart.rs`), features `graphics`/`s3`, lien getting-started.

### Context
- `examples/`, `docs/release.md`, ADR 0002/0003.

## J06-P8 — Review J06

```yaml
id: J06-P8
kind: review
tier: T2
size: S
depends_on: [J06-P1, J06-P2, J06-P3, J06-P4, J06-P5, J06-P6, J06-P7]
files: []
acceptance:
  - "grep -q '^## Décision' docs/adr/0002-api-publique.md"
  - "cargo test -p sasrs --test api 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "grep -q 'pub mod api' src/lib.rs"
  - "cargo test -p sasrs --test api diagnostics 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "cargo test -p sasrs --test api produced_files 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "cargo run --example quickstart 2>&1 | grep -q 'OK'"
  - "cargo test -p sasrs --test examples 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "cd python && python3 -m unittest discover -s tests -v 2>&1 | grep -q '^OK'"
  - "grep -q 'unittest' .github/workflows/ci.yml"
  - "grep -qi 'lance un binaire\\|launches a binary' python/README.md"
  - "grep -q \"tags:\" .github/workflows/release.yml && grep -q 'SHA256SUMS' .github/workflows/release.yml && grep -q 'manifest.json' .github/workflows/release.yml"
  - "cargo run -- --version 2>&1 | grep -qE 'commit [0-9a-f]{7}|commit unknown'"
  - "grep -q 'build = \"build.rs\"' Cargo.toml || test -f build.rs"
  - "grep -q '^## Décision' docs/adr/0003-api-python.md && grep -qi 'pyo3' docs/adr/0003-api-python.md && grep -qi 'DataFrame' docs/adr/0003-api-python.md"
  - "grep -q 'docs/getting-started.md' README.md && grep -q 'examples/quickstart.rs' docs/getting-started.md"
  - "grep -qi 'retrieve\\|récupérer' docs/getting-started.md && grep -q 'Exit codes' README.md"
  - "scripts/ci-status.sh consolidation"
  - "cargo test -p sasrs"
  - "cargo build --features graphics && cargo build --features s3"
  - "cargo fmt --check && cargo clippy --all-targets -- -D warnings"
```

### Scope
Review the merged milestone diff with verify-before-done, code-review, test-design. Report; change nothing.
