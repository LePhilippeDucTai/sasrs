# J04 — Intégrité du stockage et des métadonnées

Goal: le couple `.parquet` + `.sasmeta.json` reste cohérent face aux interruptions, suppressions, renommages et corruptions ; toute incohérence est diagnostiquée ; les métadonnées survivent aux transformations SQL/CSV ; stratégie de récupération documentée et testée par un agent distinct de l'implémenteur (issue #6).
Depends on: J03 · Orchestrator: sonnet/high

## J04-P1 — Protocole d'écriture atomique parquet + sidecar

```yaml
id: J04-P1
kind: implement
tier: T2
size: M
depends_on: []
files:
  - docs/adr/0001-stockage-parquet-sidecar.md
  - src/dataset.rs
  - src/library/mod.rs
  - src/library/dir.rs
  - src/dataset/tests.rs
  - Cargo.toml
acceptance:
  - "grep -q '^## Décision' docs/adr/0001-stockage-parquet-sidecar.md && grep -qi 'récupération' docs/adr/0001-stockage-parquet-sidecar.md"
  - "grep -q 'fault-injection' Cargo.toml"
  - "cargo test -p sasrs --lib atomic_write 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "cargo build --features fault-injection"
```

### Scope
- ADR (contexte, options, décision, conséquences, stratégie de récupération) ; option recommandée : fichiers temporaires dans le même dossier + fsync + `rename` ; empreinte du parquet (taille, nombre de lignes, noms de colonnes) enregistrée dans le sidecar ; sidecar dont l'empreinte ne correspond pas = périmé → ignoré avec diagnostic ; ordre d'écriture choisi pour qu'une interruption ne publie jamais de nouvelles données avec des métadonnées fausses.
- Implémentation dans `SasDataset::write_parquet`/`write_sidecar` et `DirLibrary::write` ; fichiers temporaires orphelins nettoyés ou ignorés par `list`.
- Feature cargo `fault-injection` (hors défaut) : points d'arrêt nommés (`after_parquet_tmp`, `after_parquet_rename`, `after_sidecar_tmp`…) pilotés par la variable `SASRS_FAULT_INJECT` ; aucun coût hors feature.
- Tests unitaires préfixés `atomic_write`.
- Ne pas : changer le format Parquet ni la décision « sidecar JSON » de M4.

### Context
- `src/dataset.rs:55-190`, `src/library/dir.rs:13-98`, `src/library/mod.rs:32-61` (doc du trait : rename déplace le sidecar).
- `PLAN.md` racine § Décisions actées (Stockage), M4 persistance VarMeta.

## J04-P2 — Suppression, renommage et échange sans orphelins

```yaml
id: J04-P2
kind: implement
tier: T3
size: M
depends_on: [J04-P1]
files:
  - src/library/dir.rs
  - src/library/csv.rs
  - src/library/tests.rs
  - src/procs/datasets/mod.rs
  - src/procs/datasets/tests.rs
acceptance:
  - "cargo test -p sasrs --lib orphan_sidecar 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- `delete` supprime aussi le sidecar ; `rename` refuse une destination existante (ERROR, comme PROC DATASETS CHANGE en SAS), supprime tout sidecar orphelin de la destination, et annule le déplacement du parquet si celui du sidecar échoue.
- EXCHANGE : rollback en cas d'échec intermédiaire ; `unique_temp_name` vérifie aussi les sidecars.
- `list`/`exists`/`read` cohérents sur la casse des noms de fichiers.
- CSV : `scan` ne jette plus les notes de coercition (WARNING 2^53 transmis).
- Tests unitaires préfixés `orphan_sidecar` (delete, rename vers une cible avec sidecar orphelin, échec simulé via `fault-injection`, EXCHANGE).

### Context
- `library/dir.rs:19-98`, `library/csv.rs:67-116`, `procs/datasets/mod.rs:111-200`, test existant `procs/datasets/tests.rs:223`.

## J04-P3 — Sidecar corrompu ou invalide : diagnostic explicite

```yaml
id: J04-P3
kind: implement
tier: T3
size: S
depends_on: [J04-P1]
files:
  - src/dataset.rs
  - src/dataset/tests.rs
acceptance:
  - "cargo test -p sasrs --lib sidecar_invalid 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- Sidecar illisible, JSON invalide, type de champ faux, longueur incompatible avec le type ou inférieure à la plus longue valeur, empreinte périmée : WARNING (compté) nommant le fichier et la cause ; métadonnées fautives ignorées, données lues ; plus aucun `.ok()` silencieux.
- Entrées de variables absentes du parquet : NOTE.
- Tests unitaires préfixés `sidecar_invalid` (un par cause).

### Context
- `dataset.rs:75-91,131-190` ; ADR 0001 (J04-P1).

## J04-P4 — Métadonnées après transformations SQL

```yaml
id: J04-P4
kind: implement
tier: T3
size: M
depends_on: [J04-P1]
files:
  - src/sql/select.rs
  - src/sql/plan/
  - src/sql/dictionary.rs
  - src/sql/tests/
  - tests/fixtures/j04/sqlmeta_
  - tests/snapshots/snapshot__fixtures@j04__sqlmeta_
acceptance:
  - "cargo test -p sasrs --lib sql_metadata 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- `CREATE TABLE AS SELECT` conserve format, label et longueur des colonnes sources reprises telles quelles (règles SAS : colonne calculée sans métadonnées sauf `FORMAT=`/`LABEL=`/`LENGTH=` dans le SELECT) ; les lectures SQL passent par le chemin qui applique le sidecar.
- `sql/dictionary.rs` : notes de lecture transmises au log.
- Tests unitaires préfixés `sql_metadata` ; fixture `tests/fixtures/j04/sqlmeta_*.sas` (CONTENTS après CREATE TABLE AS).

### Context
- `sql/select.rs:104-118`, `sql/plan/source.rs:17`, `sql/dictionary.rs:106-141`, `dataset.rs:102-117` (`from_dataframe`).

## J04-P5 — Tests d'interruption et de corruption simulées, doc de récupération

```yaml
id: J04-P5
kind: implement
tier: T4
size: M
depends_on: [J04-P2, J04-P3, J04-P4]
files:
  - tests/storage_integrity.rs
  - .github/workflows/ci.yml
  - scripts/check.sh
  - README.md
acceptance:
  - "cargo test -p sasrs --features fault-injection --test storage_integrity 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "grep -q 'fault-injection' .github/workflows/ci.yml && grep -q 'fault-injection' scripts/check.sh"
  - "grep -qi 'recovery' README.md"
```

### Scope
- Suite adversariale écrite sans modifier `src/` : pour chaque point d'injection, lancer un programme SAS réel (binaire ou `run`) qui écrit/remplace/supprime/renomme une table, interrompre, puis vérifier qu'un run suivant lit soit l'ancien état cohérent soit le nouveau, jamais données nouvelles + métadonnées fausses sans diagnostic ; round-trip données + métadonnées (formats, labels, longueurs, missings spéciaux) ; sidecars corrompus générés.
- Un défaut trouvé est rapporté (`blocked` avec reproducer), pas corrigé ici.
- CI et `scripts/check.sh` : job/commande `cargo test --features fault-injection --test storage_integrity`.
- `README.md` : section « Storage and recovery » (protocole, diagnostic, que faire après une interruption), lien vers l'ADR.

### Context
- ADR 0001 ; points d'injection `SASRS_FAULT_INJECT` (J04-P1) ; `tests/common/mod.rs`.

## J04-P6 — Review J04

```yaml
id: J04-P6
kind: review
tier: T2
size: S
depends_on: [J04-P1, J04-P2, J04-P3, J04-P4, J04-P5]
files: []
acceptance:
  - "grep -q '^## Décision' docs/adr/0001-stockage-parquet-sidecar.md && grep -qi 'récupération' docs/adr/0001-stockage-parquet-sidecar.md"
  - "grep -q 'fault-injection' Cargo.toml"
  - "cargo test -p sasrs --lib atomic_write 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "cargo build --features fault-injection"
  - "cargo test -p sasrs --lib orphan_sidecar 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "cargo test -p sasrs --lib sidecar_invalid 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "cargo test -p sasrs --lib sql_metadata 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "cargo test -p sasrs --features fault-injection --test storage_integrity 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "grep -q 'fault-injection' .github/workflows/ci.yml && grep -q 'fault-injection' scripts/check.sh"
  - "grep -qi 'recovery' README.md"
  - "scripts/ci-status.sh consolidation"
  - "cargo test -p sasrs"
  - "cargo build --features graphics && cargo build --features s3"
  - "cargo fmt --check && cargo clippy --all-targets -- -D warnings"
```

### Scope
Review the merged milestone diff with verify-before-done, code-review, test-design. Report; change nothing.
