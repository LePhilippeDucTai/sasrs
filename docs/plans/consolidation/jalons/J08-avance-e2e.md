# J08 — XLSX, BY, ODS OUTPUT, feuille de route avancée, validation de bout en bout

Goal: clore la compatibilité prioritaire (XLSX, BY généralisé, objets ODS OUTPUT utiles, étude SAS7BDAT/XPT), produire la feuille de route avancée découpée par comportement borné (ex-M46–M49, M51–M66, issue #10), et valider le Goal de bout en bout : programme réaliste depuis une installation vierge, couverture publique qui ne promet que ce qui est implémenté et validé.
Depends on: J07 · Orchestrator: best/max

## J08-P1 — PROC IMPORT/EXPORT DBMS=XLSX

```yaml
id: J08-P1
kind: implement
tier: T3
size: M
depends_on: []
files:
  - src/procs/import.rs
  - src/procs/export.rs
  - src/output/excel.rs
  - src/output/xlsx.rs
  - Cargo.toml
  - Cargo.lock
  - tests/fixtures/j08/xlsx_
  - tests/snapshots/snapshot__fixtures@j08__xlsx_
acceptance:
  - "cargo test -p sasrs --lib xlsx_ 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "ls tests/fixtures/j08/xlsx_*.sas"
```

### Scope
- IMPORT `DBMS=XLSX|EXCEL` : lecture via `calamine` (I/O lourd → crate, décision de `PLAN.md` racine), `SHEET=`, `RANGE=`, `GETNAMES=`, types SAS stricts (dates Excel → dates SAS 1960), `GUESSINGROWS=` honoré ou ERROR.
- EXPORT `DBMS=XLSX` : réutilise l'écrivain XLSX pur Rust de `output/excel.rs`, extrait dans `output/xlsx.rs` ; `SHEET=`, `REPLACE`, formats appliqués selon la doc.
- Aller-retour EXPORT → IMPORT sans perte (valeurs, missings, dates) ; tests unitaires préfixés `xlsx_` ; fixtures `tests/fixtures/j08/xlsx_*.sas`.

### Context
- `procs/import.rs:79-352`, `procs/export.rs:156-199`, `output/excel.rs:89-140`.

## J08-P2 — BY généralisé (CORR, TABULATE)

```yaml
id: J08-P2
kind: implement
tier: T3
size: M
depends_on: []
files:
  - src/procs/corr/
  - src/procs/tabulate/
  - src/procs/common/by.rs
  - tests/fixtures/j08/by_
  - tests/snapshots/snapshot__fixtures@j08__by_
acceptance:
  - "cargo test -p sasrs --lib by_group 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- BY (y compris DESCENDING, contrôle de tri, en-tête de groupe SAS, `OUT=` avec variables BY) pour CORR et TABULATE via `common::by` ; invariant testé : un seul groupe = sortie sans BY.
- Les autres procs qui refusent BY depuis J02-P3 restent en ERROR ; leur liste passe dans la feuille de route (J08-P5).
- Tests unitaires préfixés `by_group` ; fixtures `tests/fixtures/j08/by_*.sas`.

### Context
- `procs/common/by.rs`, BY honoré par FREQ/MEANS/UNIVARIATE (`means/mod.rs:219-233`), `corr/parse.rs:116`, `tabulate/parse.rs:65-89`.

## J08-P3 — Objets ODS OUTPUT supplémentaires

```yaml
id: J08-P3
kind: implement
tier: T3
size: M
depends_on: []
files:
  - src/procs/univariate/
  - src/procs/freq/
  - src/procs/reg/
  - src/session/ods_output.rs
  - tests/fixtures/j08/odsout_
  - tests/snapshots/snapshot__fixtures@j08__odsout_
acceptance:
  - "cargo test -p sasrs --lib ods_output_ 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- Capture par nom d'objet SAS (colonnes et noms de la doc) : UNIVARIATE `Quantiles`, `ExtremeObs`, `TestsForNormality` ; FREQ `CrossTabFreqs`, `ChiSq`, `FishersExact` ; REG `ParameterEstimates`, `ANOVA`, `FitStatistics`.
- ODS SELECT/EXCLUDE reconnaît ces noms ; tests unitaires préfixés `ods_output_`.

### Context
- `session/ods_output.rs:72-217`, captures existantes (`freq/oneway.rs:101-103`, `univariate/emit.rs:113-190`).

## J08-P4 — Étude des adaptateurs SAS7BDAT/XPT

```yaml
id: J08-P4
kind: implement
tier: T4
size: S
depends_on: []
files:
  - docs/adr/0004-adaptateurs-sas7bdat-xpt.md
acceptance:
  - "grep -q '^## Décision' docs/adr/0004-adaptateurs-sas7bdat-xpt.md && grep -q 'XPT' docs/adr/0004-adaptateurs-sas7bdat-xpt.md && grep -qi 'licen' docs/adr/0004-adaptateurs-sas7bdat-xpt.md"
```

### Scope
- ADR : adaptateurs aux frontières (PROC IMPORT/EXPORT, `LIBNAME … XPORT`) sans remplacer le stockage Parquet ; options (crates pur Rust, readstat via FFI, écriture XPT v5/v8), licences, encodages, métadonnées (formats, labels, longueurs), plan de tests ; recommandation et items pour la feuille de route.
- Ne pas : écrire de code.

### Context
- Issue #9 § 6 ; ADR 0001 (stockage).

## J08-P5 — Feuille de route avancée par comportement borné

```yaml
id: J08-P5
kind: implement
tier: T1
size: M
depends_on: [J08-P4]
files:
  - docs/roadmap/avancee.md
  - PLAN.md
acceptance:
  - "for s in LOGISTIC GENMOD GLM ANOVA MIXED GLIMMIX PRINCOMP FACTOR DISCRIM CLUSTER FASTCLUS DISTANCE IML S3 Graphiques TABULATE REPORT DATASETS CATALOG OPTIONS; do grep -q \"^## .*$s\" docs/roadmap/avancee.md || exit 1; done"
  - "test $(grep -c 'Oracle' docs/roadmap/avancee.md) -ge 20 && grep -qi 'non-convergence' docs/roadmap/avancee.md && grep -qi 'singuli' docs/roadmap/avancee.md && grep -qi 'identifiabilit' docs/roadmap/avancee.md"
  - "grep -q 'M66' docs/roadmap/avancee.md && grep -q 'docs/roadmap/avancee.md' PLAN.md"
```

### Scope
- Une section `## <domaine>` par proc et pour S3, Graphiques, résidus Base (TABULATE `PCTN<>`, REPORT FLOW/COMPUTE, DATASETS, CATALOG, OPTIONS) ; items bornés par comportement (jamais « terminer PROC X »), chacun avec : état actuel (implémenté / validé / approximation / ERROR), dépendances, oracle indépendant prévu, cas de non-convergence, matrice singulière, paramètre sur frontière, identifiabilité et tolérances quand c'est numérique, bénéfice utilisateur, priorité.
- Contenu minimal issu de #10 : LOGISTIC (codages, multinomial, SCORE, ROC, BY, diagnostics prop-odds), GENMOD (OFFSET=, OUTPUT, GEE/REPEATED, ESTIMATE/CONTRAST, BY, échelle Gamma ML), GLM/ANOVA (covariables, comparaisons multiples, Type II/IV, contrastes, inverse généralisée, BY), MIXED (pentes aléatoires, LSMEANS/ESTIMATE/CONTRAST, KR/Satterthwaite), GLIMMIX (QUAD, pentes, diagnostics), multivarié/IML, S3 (exists/list/write/delete/rename, contrat), graphiques (options parse-only, images BY, légendes, styles, overlays), API Python native (ADR 0003), SAS7BDAT/XPT (ADR 0004).
- Ordre par dépendances et bénéfice ; table de correspondance ex-M46–M66 → items ; prêt à alimenter `/milestone-plan` d'un plan suivant.
- `PLAN.md` racine : section de correspondance pointée vers la feuille de route.

### Context
- `PLAN.md`/`PROGRESS.md` racine (M46–M66), issue #10, `docs/support-contract.md`, `conformance/STATUS.md`, `README.md`, ADR 0003 et 0004.

## J08-P6 — Validation de bout en bout et contrôle des promesses de couverture

```yaml
id: J08-P6
kind: implement
tier: T4
size: M
depends_on: [J08-P1, J08-P2, J08-P3]
files:
  - tests/e2e.rs
  - tests/e2e/
  - scripts/check-coverage-claims.sh
  - .github/workflows/ci.yml
  - scripts/check.sh
  - README.md
acceptance:
  - "cargo test -p sasrs --test e2e 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "test -x scripts/check-coverage-claims.sh && scripts/check-coverage-claims.sh"
  - "grep -q 'check-coverage-claims' .github/workflows/ci.yml && grep -q 'cargo install --path' .github/workflows/ci.yml"
```

### Scope
- `tests/e2e/` : programme réaliste de migration (CSV/XLSX importés → DATA step → SORT → MEANS NWAY `OUTPUT` → TRANSPOSE → COMPARE contre une table attendue → ODS OUTPUT → EXPORT XLSX), exécuté par le binaire et par `sasrs::api` ; tables produites comparées à des attendus issus du corpus de conformité ou d'exemples SAS publiés ; codes retour et diagnostics vérifiés.
- CI : job « installation vierge » (`cargo install --path . --root <tmp>` puis exemple CLI de `docs/getting-started.md`) ; job `check-coverage-claims` dans `ci-ok`.
- `scripts/check-coverage-claims.sh` : chaque ligne marquée validée dans `README.md` a au moins un cas `validated` dans `conformance/STATUS.md` ; sinon échec.
- `README.md` : lignes IMPORT/EXPORT, CORR, TABULATE, ODS OUTPUT mises à jour ; aucune promesse au-delà de l'implémenté et validé.

### Context
- `docs/getting-started.md`, `conformance/`, `scripts/conformance-report.sh`, `CONTRIBUTING.md`.

## J08-P7 — Review J08

```yaml
id: J08-P7
kind: review
tier: T2
size: S
depends_on: [J08-P1, J08-P2, J08-P3, J08-P4, J08-P5, J08-P6]
files: []
acceptance:
  - "cargo test -p sasrs --lib xlsx_ 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "ls tests/fixtures/j08/xlsx_*.sas"
  - "cargo test -p sasrs --lib by_group 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "cargo test -p sasrs --lib ods_output_ 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "grep -q '^## Décision' docs/adr/0004-adaptateurs-sas7bdat-xpt.md && grep -q 'XPT' docs/adr/0004-adaptateurs-sas7bdat-xpt.md && grep -qi 'licen' docs/adr/0004-adaptateurs-sas7bdat-xpt.md"
  - "for s in LOGISTIC GENMOD GLM ANOVA MIXED GLIMMIX PRINCOMP FACTOR DISCRIM CLUSTER FASTCLUS DISTANCE IML S3 Graphiques TABULATE REPORT DATASETS CATALOG OPTIONS; do grep -q \"^## .*$s\" docs/roadmap/avancee.md || exit 1; done"
  - "test $(grep -c 'Oracle' docs/roadmap/avancee.md) -ge 20 && grep -qi 'non-convergence' docs/roadmap/avancee.md && grep -qi 'singuli' docs/roadmap/avancee.md && grep -qi 'identifiabilit' docs/roadmap/avancee.md"
  - "grep -q 'M66' docs/roadmap/avancee.md && grep -q 'docs/roadmap/avancee.md' PLAN.md"
  - "cargo test -p sasrs --test e2e 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "test -x scripts/check-coverage-claims.sh && scripts/check-coverage-claims.sh"
  - "grep -q 'check-coverage-claims' .github/workflows/ci.yml && grep -q 'cargo install --path' .github/workflows/ci.yml"
  - "scripts/conformance-report.sh --check"
  - "scripts/ci-status.sh consolidation"
  - "cargo test -p sasrs"
  - "cargo build --features graphics && cargo build --features s3"
  - "cargo fmt --check && cargo clippy --all-targets -- -D warnings"
```

### Scope
Review the merged milestone diff with verify-before-done, code-review, test-design. Report; change nothing.
