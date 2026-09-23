# J07 — Compatibilité à forte valeur (1) : MEANS, COMPARE, TRANSPOSE, PRINTTO, informats

Goal: exécuter des programmes SAS réels de préparation et de validation de données : options manquantes de MEANS/SUMMARY, COMPARE outil de validation de migration, TRANSPOSE complet, PRINTTO réellement routé, informats persistés ; chaque incrément validé contre des oracles écrits avant l'implémentation (issue #9 ; ex-M50).
Depends on: J06 · Orchestrator: sonnet/high

## J07-P1 — Oracles de conformité écrits avant l'implémentation

```yaml
id: J07-P1
kind: implement
tier: T4
size: M
depends_on: []
files:
  - conformance/cases/compat-
acceptance:
  - "for p in means compare transpose printto informat; do ls -d conformance/cases/compat-$p-*/ >/dev/null 2>&1 || exit 1; done"
  - "ls -d conformance/cases/compat-*/ | wc -l | grep -qE '^([5-9]|[1-9][0-9])$' && ! grep -L '\"source\": \"http' conformance/cases/compat-*/case.json | grep -q ."
  - "cargo test -p sasrs --test conformance 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- Cas `known-divergence` (valeurs attendues recopiées d'exemples publiés de la doc SAS 9.4, URL en provenance) : MEANS/SUMMARY (NWAY, MISSING, ORDER=, ID, MAXDEC=, CLASS `/ MISSING ORDER=`, FREQ, `OUTPUT / AUTONAME`, valeurs CLASS manquantes exclues par défaut), COMPARE (ID, VAR/WITH, CRITERION=, METHOD=, OUTNOEQUAL/OUTBASE/OUTCOMP/OUTDIF, BY), TRANSPOSE (COPY, IDLABEL, LET, SUFFIX=, LABEL=, DELIMITER=, ID multiples, BY DESCENDING), PRINTTO (contenu des fichiers routés, NEW), informats (CONTENTS après INFORMAT/ATTRIB).
- Ne pas : modifier `src/` ; ces cas sont la référence des parts J07-P2 à P6, qui ne changeront que leur `status`.

### Context
- `conformance/schema.md`, `CONTRIBUTING.md` (oracles).

## J07-P2 — PROC MEANS/SUMMARY : options de production

```yaml
id: J07-P2
kind: implement
tier: T3
size: M
depends_on: [J07-P1]
files:
  - src/procs/means/
  - conformance/cases/compat-means-
  - tests/fixtures/j07/means_
  - tests/snapshots/snapshot__fixtures@j07__means_
acceptance:
  - "ls conformance/cases/compat-means-*/case.json >/dev/null && ! grep -l '\"status\": \"known-divergence\"' conformance/cases/compat-means-*/case.json | grep -q ."
  - "cargo test -p sasrs --test conformance 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "cargo test -p sasrs --lib means_compat 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- `NWAY`, `MISSING` (PROC et CLASS), `ORDER=` (DATA/FORMATTED/FREQ/INTERNAL), `ID`, `MAXDEC=`, `DESCENDTYPES`, `COMPLETETYPES`, `CHARTYPE`, `FREQ`, `VARDEF=`, `EXCLNPWGT`, `OUTPUT` formes `mean=`/`mean(x y)=`/`/ AUTONAME`, plusieurs instructions CLASS/VAR/OUTPUT ; valeurs CLASS manquantes exclues par défaut (changement de sortie : snapshots justifiés) ; ODS `Summary` avec colonnes CLASS/BY.
- Passer les cas `compat-means-*` à `validated` (seul le statut change).
- Tests unitaires préfixés `means_compat`.

### Context
- `means/parse.rs`, `means/types.rs`, `means/output.rs`, `means/report.rs` ; `docs/support-contract.md`.

## J07-P3 — PROC COMPARE : outil de validation de migration

```yaml
id: J07-P3
kind: implement
tier: T3
size: M
depends_on: [J07-P1]
files:
  - src/procs/compare/
  - conformance/cases/compat-compare-
  - tests/fixtures/j07/compare_
  - tests/snapshots/snapshot__fixtures@j07__compare_
acceptance:
  - "ls conformance/cases/compat-compare-*/case.json >/dev/null && ! grep -l '\"status\": \"known-divergence\"' conformance/cases/compat-compare-*/case.json | grep -q ."
  - "cargo test -p sasrs --lib compare_compat 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- `ID` (appariement par clé triée, observations propres à chaque table signalées), `VAR`/`WITH`, `CRITERION=`, `METHOD=ABSOLUTE|RELATIVE|EXACT|PERCENT`, `BRIEF`, `LISTALL`, `MAXPRINT=`, `OUT=` avec `OUTNOEQUAL`/`OUTBASE`/`OUTCOMP`/`OUTDIF`/`OUTPERCENT`, `BY` ; messages et code `&SYSINFO` conformes à la doc.
- Passer les cas `compat-compare-*` à `validated`.
- Tests unitaires préfixés `compare_compat`.

### Context
- `compare/parse.rs`, `compare/analyze.rs`, `compare/output.rs`.

## J07-P4 — PROC TRANSPOSE complet

```yaml
id: J07-P4
kind: implement
tier: T3
size: M
depends_on: [J07-P1]
files:
  - src/procs/transpose/
  - conformance/cases/compat-transpose-
  - tests/fixtures/j07/transpose_
  - tests/snapshots/snapshot__fixtures@j07__transpose_
acceptance:
  - "ls conformance/cases/compat-transpose-*/case.json >/dev/null && ! grep -l '\"status\": \"known-divergence\"' conformance/cases/compat-transpose-*/case.json | grep -q ."
  - "cargo test -p sasrs --lib transpose_compat 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- `COPY`, `IDLABEL` (+ colonne `_LABEL_`), `LET`, `SUFFIX=`, `LABEL=`, `DELIMITER=`, `PREFIX=` chaîne, plusieurs variables `ID`, `BY DESCENDING` + contrôle de tri, longueurs inférées en caractères.
- Passer les cas `compat-transpose-*` à `validated`.
- Tests unitaires préfixés `transpose_compat`.

### Context
- `transpose/mod.rs`, `transpose/naming.rs`.

## J07-P5 — PROC PRINTTO : routage réel

```yaml
id: J07-P5
kind: implement
tier: T3
size: M
depends_on: [J07-P1]
files:
  - src/procs/printto.rs
  - src/session.rs
  - src/log.rs
  - src/lib.rs
  - tests/cli.rs
  - conformance/cases/compat-printto-
acceptance:
  - "ls conformance/cases/compat-printto-*/case.json >/dev/null && ! grep -l '\"status\": \"known-divergence\"' conformance/cases/compat-printto-*/case.json | grep -q ."
  - "cargo test -p sasrs --test cli printto 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- `LOG=`/`PRINT=` routent réellement log et listing vers les fichiers (ajout, ou remplacement avec `NEW`), `PROC PRINTTO;` rétablit les destinations ; interaction avec `--log`/`--print` et les diagnostics structurés documentée ; erreurs d'ouverture → ERROR comptée.
- Remplace la WARNING provisoire de J02-P4.
- Passer les cas `compat-printto-*` à `validated` ; tests CLI préfixés `printto`.

### Context
- `procs/printto.rs`, `session.rs:230-237`, `log.rs`, diagnostics structurés (J06-P2).

## J07-P6 — Informats persistés dans les métadonnées

```yaml
id: J07-P6
kind: implement
tier: T3
size: M
depends_on: [J07-P1]
files:
  - src/dataset.rs
  - src/datastep/compile_decl.rs
  - src/datastep/compile_io.rs
  - src/procs/contents.rs
  - src/procs/datasets/
  - conformance/cases/compat-informat-
  - tests/fixtures/j07/informat_
  - tests/snapshots/snapshot__fixtures@j07__informat_
acceptance:
  - "ls conformance/cases/compat-informat-*/case.json >/dev/null && ! grep -l '\"status\": \"known-divergence\"' conformance/cases/compat-informat-*/case.json | grep -q ."
  - "grep -q 'informat' src/dataset.rs"
  - "cargo test -p sasrs --lib informat_meta 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- `VarMeta.informat` persisté dans le sidecar (protocole ADR 0001, rétrocompatible) ; posé par INFORMAT/ATTRIB en étape DATA, conservé par SET/MERGE ; colonne Informat de PROC CONTENTS et `OUT=` ; `MODIFY … INFORMAT` dans PROC DATASETS.
- Passer les cas `compat-informat-*` à `validated` ; tests unitaires préfixés `informat_meta`.

### Context
- `dataset.rs:24-190`, README ligne DATA step « Not supported » (informats non persistés), ADR 0001.

## J07-P7 — Documentation de couverture (J07)

```yaml
id: J07-P7
kind: implement
tier: T5
size: S
depends_on: [J07-P2, J07-P3, J07-P4, J07-P5, J07-P6]
files:
  - README.md
  - docs/support-contract.md
  - conformance/STATUS.md
acceptance:
  - "scripts/conformance-report.sh --check"
  - "! grep -qF '`MAXDEC=`, `NWAY`, `MISSING`, `ORDER=`, `ID`' README.md && ! grep -qF '`IDLABEL`, `COPY`, `LET`, `SUFFIX=`' README.md && ! grep -qF '`CRITERION=`, `ID`, `VAR`/`WITH`' README.md"
  - "grep -q 'compat-means' conformance/STATUS.md"
```

### Scope
- `README.md` : lignes MEANS/SUMMARY, COMPARE, TRANSPOSE, PRINTTO, CONTENTS, DATA step (informats) mises à jour avec l'état « validé » issu du corpus ; `conformance/STATUS.md` régénéré ; `docs/support-contract.md` à jour.

### Context
- `scripts/conformance-report.sh` (J05-P6), `CONTRIBUTING.md`.

## J07-P8 — Review J07

```yaml
id: J07-P8
kind: review
tier: T2
size: S
depends_on: [J07-P1, J07-P2, J07-P3, J07-P4, J07-P5, J07-P6, J07-P7]
files: []
acceptance:
  - "for p in means compare transpose printto informat; do ls -d conformance/cases/compat-$p-*/ >/dev/null 2>&1 || exit 1; done"
  - "ls -d conformance/cases/compat-*/ | wc -l | grep -qE '^([5-9]|[1-9][0-9])$' && ! grep -L '\"source\": \"http' conformance/cases/compat-*/case.json | grep -q ."
  - "cargo test -p sasrs --test conformance 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "ls conformance/cases/compat-means-*/case.json >/dev/null && ! grep -l '\"status\": \"known-divergence\"' conformance/cases/compat-means-*/case.json | grep -q ."
  - "cargo test -p sasrs --lib means_compat 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "ls conformance/cases/compat-compare-*/case.json >/dev/null && ! grep -l '\"status\": \"known-divergence\"' conformance/cases/compat-compare-*/case.json | grep -q ."
  - "cargo test -p sasrs --lib compare_compat 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "ls conformance/cases/compat-transpose-*/case.json >/dev/null && ! grep -l '\"status\": \"known-divergence\"' conformance/cases/compat-transpose-*/case.json | grep -q ."
  - "cargo test -p sasrs --lib transpose_compat 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "ls conformance/cases/compat-printto-*/case.json >/dev/null && ! grep -l '\"status\": \"known-divergence\"' conformance/cases/compat-printto-*/case.json | grep -q ."
  - "cargo test -p sasrs --test cli printto 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "ls conformance/cases/compat-informat-*/case.json >/dev/null && ! grep -l '\"status\": \"known-divergence\"' conformance/cases/compat-informat-*/case.json | grep -q ."
  - "grep -q 'informat' src/dataset.rs"
  - "cargo test -p sasrs --lib informat_meta 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "scripts/conformance-report.sh --check"
  - "! grep -qF '`MAXDEC=`, `NWAY`, `MISSING`, `ORDER=`, `ID`' README.md && ! grep -qF '`IDLABEL`, `COPY`, `LET`, `SUFFIX=`' README.md && ! grep -qF '`CRITERION=`, `ID`, `VAR`/`WITH`' README.md"
  - "grep -q 'compat-means' conformance/STATUS.md"
  - "scripts/ci-status.sh consolidation"
  - "cargo test -p sasrs"
  - "cargo build --features graphics && cargo build --features s3"
  - "cargo fmt --check && cargo clippy --all-targets -- -D warnings"
```

### Scope
Review the merged milestone diff with verify-before-done, code-review, test-design. Report; change nothing.
