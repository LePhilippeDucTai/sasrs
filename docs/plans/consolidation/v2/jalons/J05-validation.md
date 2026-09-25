# J05 — Validation industrielle : conformité, propriétés, différentiel

Goal: un corpus de conformité SAS indépendant des snapshots (provenance, tolérances, nature math/SAS, divergences connues), des tests de propriétés et différentiels, tous exécutés par la CI ; la couverture publique distingue implémenté et validé contre référence (issue #7).
Depends on: J04 · Orchestrator: sonnet/high

## J05-P1 — Structure du corpus de conformité et exécuteur

```yaml
id: J05-P1
kind: implement
tier: T3
size: M
depends_on: []
files:
  - conformance/README.md
  - conformance/schema.md
  - conformance/cases/example-
  - tests/conformance.rs
acceptance:
  - "test -f conformance/README.md && grep -q 'provenance' conformance/schema.md && grep -q 'known-divergence' conformance/schema.md && grep -q 'tolerance' conformance/schema.md"
  - "cargo test -p sasrs --test conformance 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "ls -d conformance/cases/example-*/ | wc -l | grep -qE '^[2-9]'"
```

### Scope
- Format d'un cas `conformance/cases/<id>/` : `program.sas`, `data/*.csv` (entrées), `expected/<dataset>.csv` (sorties structurées), `case.json` : `id`, `title`, `provenance` {`kind`: sas-doc | sas-run | independent-oracle, `source` (URL ou référence), `sas_version`, `options`}, `validates`: math | sas-behaviour, `tolerance` {abs, rel} par défaut et par colonne, `log` {required, forbidden (regex)}, `exit_code`, `status`: validated | known-divergence (+ `issue`).
- `tests/conformance.rs` : pour chaque cas, tempdir, CSV → parquet (types déclarés dans `case.json`), `run` avec `work_dir` fixé, relecture des datasets attendus, comparaison avec tolérances (missings SAS comparés comme tels) ; `known-divergence` doit échouer, sinon le test signale « à promouvoir » et échoue ; rapport par cas sur stdout.
- Deux cas d'exemple (`example-math-*`, `example-sasdoc-*`).
- Ne pas : dépendre des snapshots insta.

### Context
- Issue #7 § « corpus de conformité » ; `CONTRIBUTING.md` (oracles, `known-divergence`) ; `tests/common/mod.rs`.

## J05-P2 — Cas de conformité Base issus de la documentation SAS

```yaml
id: J05-P2
kind: implement
tier: T4
size: M
depends_on: [J05-P1]
files:
  - conformance/cases/base-
acceptance:
  - "ls -d conformance/cases/base-*/ | wc -l | grep -qE '^([1-9][0-9])$'"
  - "cargo test -p sasrs --test conformance 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- ≥ 10 cas reprenant des exemples publiés de la doc SAS 9.4 avec leur sortie (URL exacte dans `provenance`) : étape DATA (RETAIN, FIRST./LAST., MERGE BY, UPDATE, tableaux), SORT NODUPKEY, FORMAT, MEANS CLASS/OUTPUT, FREQ CHISQ, TRANSPOSE, SQL jointures et remerge, fonctions caractère/date.
- Valeurs attendues recopiées de la doc, jamais produites par sasrs ; cas en échec → `known-divergence` avec description et lien d'issue à créer (`gh issue create`) ; ne pas modifier `src/`.

### Context
- `conformance/schema.md` (J05-P1).

## J05-P3 — Cas de conformité statistiques

```yaml
id: J05-P3
kind: implement
tier: T4
size: M
depends_on: [J05-P1]
files:
  - conformance/cases/stat-
acceptance:
  - "ls -d conformance/cases/stat-*/ | wc -l | grep -qE '^([8-9]|[1-9][0-9])$'"
  - "cargo test -p sasrs --test conformance 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- ≥ 8 cas depuis les exemples « Getting Started »/« Examples » publiés de SAS/STAT et Base 9.4 : REG, TTEST, CORR (Pearson, Spearman), UNIVARIATE (pondéré inclus), FREQ Fisher, NPAR1WAY, GLM, LOGISTIC binaire ; tolérances justifiées par la précision imprimée dans la doc.
- `validates: math` quand la référence est un résultat mathématique (oracle indépendant), `sas-behaviour` quand c'est une sortie SAS.
- Cas en échec → `known-divergence` + issue ; ne pas modifier `src/`.

### Context
- `conformance/schema.md` ; oracle pondéré `tests/oracles/weighted_stats.json` (J03).

## J05-P4 — Tests de propriétés

```yaml
id: J05-P4
kind: implement
tier: T3
size: M
depends_on: []
files:
  - Cargo.toml
  - Cargo.lock
  - tests/properties.rs
acceptance:
  - "grep -q 'proptest' Cargo.toml"
  - "cargo test -p sasrs --test properties 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- `proptest` en dev-dependency ; propriétés : `Value::sas_cmp` est un ordre total cohérent avec l'ordre SAS des missings (`._` < `.` < `.A` … `.Z` < nombres) ; missings spéciaux préservés par un aller-retour parquet ; round-trip `VarMeta` (format, label, longueur) par write/read ; troncature PDV : jamais d'UTF-8 invalide, ≤ longueur en caractères ; formats `$w.` et `w.d` ne paniquent sur aucune entrée et respectent la largeur en caractères ; tri SORT stable et conforme à `sas_cmp`.
- Nombre de cas borné pour rester < 30 s ; graine reproductible en CI.
- Un défaut trouvé : reproducer minimal rapporté (`blocked`), pas de correction silencieuse.

### Context
- `src/value.rs`, `src/missing.rs`, `src/dataset.rs`, `src/datastep/pdv.rs`, `src/formats/`, `docs/encoding.md`.

## J05-P5 — Différentiel chemin vectorisé / ligne à ligne

```yaml
id: J05-P5
kind: implement
tier: T3
size: M
depends_on: [J05-P4]
files:
  - tests/differential.rs
  - src/datastep/fastpath.rs
  - src/datastep/fastpath/
acceptance:
  - "cargo test -p sasrs --test differential 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- Chaque fixture `tests/fixtures/**/*.sas` exécutée avec `vectorize` vrai et faux : log (hors NOTE propre au fast-path, s'il en existe une), listing et datasets produits identiques bit à bit.
- Programmes DATA step générés (proptest) : SET + assignations numériques, missings spéciaux, KEEP/DROP/RENAME/FORMAT/LABEL, 0 ligne, FIRSTOBS/OBS.
- Écart trouvé : corriger le fast-path (repli vers la boucle si non équivalent) avec test de non-régression.

### Context
- `src/datastep/fastpath.rs`, `src/datastep/fastpath/tests.rs`, `src/datastep/exec/run.rs:7-13`, `RunOptions.vectorize`.

## J05-P6 — CI de conformité et couverture validée publique

```yaml
id: J05-P6
kind: implement
tier: T4
size: M
depends_on: [J05-P2, J05-P3, J05-P5]
files:
  - scripts/conformance-report.sh
  - conformance/STATUS.md
  - .github/workflows/ci.yml
  - scripts/check.sh
  - README.md
acceptance:
  - "test -x scripts/conformance-report.sh && scripts/conformance-report.sh --check"
  - "grep -q 'conformance' .github/workflows/ci.yml && grep -q 'properties' .github/workflows/ci.yml && grep -q 'differential' .github/workflows/ci.yml"
  - "grep -q 'conformance/STATUS.md' README.md"
```

### Scope
- `scripts/conformance-report.sh` génère `conformance/STATUS.md` (par PROC/zone : cas validés, divergences connues, provenance) ; `--check` échoue si le fichier committé est périmé.
- CI : jobs conformance, properties, differential dans `ci-ok` ; `scripts/check.sh` idem.
- `README.md` : légende à quatre états ; colonne ou marque « validé » alimentée par `conformance/STATUS.md` ; aucune ligne ne revendique « validé » sans cas.

### Context
- `conformance/`, `CONTRIBUTING.md` (quatre états), `.github/workflows/ci.yml`.

## J05-P7 — Review J05

```yaml
id: J05-P7
kind: review
tier: T2
size: S
depends_on: [J05-P1, J05-P2, J05-P3, J05-P4, J05-P5, J05-P6]
files: []
acceptance:
  - "test -f conformance/README.md && grep -q 'provenance' conformance/schema.md && grep -q 'known-divergence' conformance/schema.md && grep -q 'tolerance' conformance/schema.md"
  - "cargo test -p sasrs --test conformance 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "ls -d conformance/cases/example-*/ | wc -l | grep -qE '^[2-9]'"
  - "ls -d conformance/cases/base-*/ | wc -l | grep -qE '^([1-9][0-9])$'"
  - "ls -d conformance/cases/stat-*/ | wc -l | grep -qE '^([8-9]|[1-9][0-9])$'"
  - "grep -q 'proptest' Cargo.toml"
  - "cargo test -p sasrs --test properties 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "cargo test -p sasrs --test differential 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "test -x scripts/conformance-report.sh && scripts/conformance-report.sh --check"
  - "grep -q 'conformance' .github/workflows/ci.yml && grep -q 'properties' .github/workflows/ci.yml && grep -q 'differential' .github/workflows/ci.yml"
  - "grep -q 'conformance/STATUS.md' README.md"
  - "scripts/ci-status.sh consolidation"
  - "cargo test -p sasrs"
  - "cargo build --features graphics && cargo build --features s3"
  - "cargo fmt --check && cargo clippy --all-targets -- -D warnings"
```

### Scope
Review the merged milestone diff with verify-before-done, code-review, test-design. Report; change nothing.
