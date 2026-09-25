# J03 — Divergences numériques et encodage

Goal: statistiques pondérées conformes contre un oracle indépendant (MEANS/SUMMARY, UNIVARIATE, y compris `OUTPUT OUT=`), contrat d'encodage en caractères appliqué partout (lexer UTF-8, formats et informats sans découpe d'octets, largeurs), divergences DATA step corrigées (issues #5 et #6, volet encodage).
Depends on: J02 · Orchestrator: sonnet/high

## J03-P1 — Oracle indépendant des statistiques pondérées

```yaml
id: J03-P1
kind: implement
tier: T4
size: S
depends_on: []
files:
  - tools/oracles/weighted_stats.py
  - tests/oracles/weighted_stats.json
acceptance:
  - "python3 tools/oracles/weighted_stats.py --check"
  - "python3 -c \"import json; d=json.load(open('tests/oracles/weighted_stats.json')); assert len(d['cases'])>=8 and all('provenance' in c and 'expected' in c for c in d['cases'])\""
```

### Scope
- Script Python, bibliothèque standard seulement, qui implémente depuis la documentation SAS 9.4 (URLs citées dans le JSON) : N, NMISS, SUMWGT, MEAN, VAR/STD (VARDEF=DF et WEIGHT), MEDIAN, P1…P99, Q1, Q3, QRANGE pondérés (définition 5 pondérée), traitement des poids nuls, négatifs et manquants par PROC MEANS et par PROC UNIVARIATE (EXCLNPWGT inclus).
- Cas ≥ 8 : poids fortement déséquilibrés, fractionnaires, nuls, négatifs, manquants, tous égaux (doit égaler la définition 5 non pondérée), une seule observation, poids cumulé tombant exactement sur une borne.
- `--check` recalcule et compare au JSON ; exit non nul en cas d'écart.
- Indépendance : ne pas lire `src/procs/means/`, `src/procs/univariate/`, `src/procs/common/stats.rs` ; toute règle SAS incertaine est marquée `"uncertain": "<raison>"` dans le cas.

### Context
- Doc SAS 9.4 : Base SAS Procedures Guide (MEANS, « Keywords and Formulas », « Calculating Percentiles »), UNIVARIATE (« Calculating Percentiles », WEIGHT).

## J03-P2 — Statistiques pondérées MEANS/SUMMARY et UNIVARIATE

```yaml
id: J03-P2
kind: implement
tier: T2
size: M
depends_on: [J03-P1]
files:
  - src/procs/common/stats.rs
  - src/procs/means/
  - src/procs/univariate/
  - tests/weighted_oracle.rs
  - tests/fixtures/j03/weighted_
  - tests/snapshots/snapshot__fixtures@j03__weighted_
acceptance:
  - "cargo test -p sasrs --test weighted_oracle 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "grep -q 'fn weighted_quantile_def5' src/procs/common/stats.rs"
```

### Scope
- `weighted_quantile_def5` déplacé dans `procs/common/stats.rs` et utilisé par MEANS/SUMMARY pour MEDIAN, percentiles et QRANGE sous WEIGHT, dans les quatre chemins (listing, PRINTALLTYPES/WAYS/TYPES, `OUTPUT OUT=`, ODS `Summary`).
- N/NMISS/SUMWGT et poids ≤ 0 conformes à l'oracle (EXCLNPWGT honoré ou ERROR explicite).
- UNIVARIATE : `OUTPUT OUT=` honore WEIGHT (moyenne, écart-type, somme, quantiles) ; tests de normalité et capture ODS sous WEIGHT conformes à SAS (calcul, ou NOTE SAS s'ils ne sont pas disponibles) ; mode et titre « Quantiles (Definition 5) » vérifiés ; doc-comment `//!` de `univariate/mod.rs` à jour.
- `tests/weighted_oracle.rs` lit `tests/oracles/weighted_stats.json` et exécute chaque cas de bout en bout via `sasrs::run` (DATALINES + `OUTPUT OUT=` relu), tolérance relative 1e-10 ; un cas `uncertain` en échec est signalé, jamais réécrit.
- Ne pas : modifier `tests/oracles/weighted_stats.json` ni le script d'oracle.
- Snapshots existants modifiés (m10 weight, m33 univariate_weighted…) : justification par fixture.

### Context
- `means/stats.rs:114-256`, `means/report.rs:92-190`, `means/output.rs:53-237`, `univariate/stats.rs:90-219`, `univariate/output.rs:24-149`, `univariate/mod.rs:302-395`, `common/stats.rs:53-71`.

## J03-P3 — Contrat d'encodage, lexer UTF-8 et BOM

```yaml
id: J03-P3
kind: implement
tier: T3
size: M
depends_on: []
files:
  - docs/encoding.md
  - src/lexer/
  - src/main.rs
  - src/datastep/build.rs
  - src/macros/include.rs
  - src/dataset.rs
  - src/procs/append.rs
  - tests/snapshots/snapshot__fixtures@m16__constructions.sas.snap
  - tests/snapshots/snapshot__fixtures@m34__glimmix_links_laplace.sas.snap
acceptance:
  - "test -f docs/encoding.md && grep -q 'caractères' docs/encoding.md && grep -q 'UTF-8' docs/encoding.md"
  - "cargo test -p sasrs --lib utf8 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "! LC_ALL=C grep -qP '\\xc3\\xa2\\xc2\\x80' tests/snapshots/snapshot__fixtures@m16__constructions.sas.snap tests/snapshots/snapshot__fixtures@m34__glimmix_links_laplace.sas.snap"
```

### Scope
- `docs/encoding.md` : contrat D-001 (longueurs et troncatures en caractères = session SAS LATIN1/WLATIN1 ; jamais d'UTF-8 invalide ; écart avec une session SAS UTF-8 ; entrées UTF-8 strictes, BOM ignoré ; sorties UTF-8, RTF échappé, PDF `?` documenté).
- Lexer : littéraux chaîne décodés en UTF-8 (fini le `b as char` Latin-1) ; identifiants toujours ASCII.
- BOM UTF-8 retiré du source `.sas`, des fichiers `%INCLUDE` et `INFILE` ; UTF-8 invalide → ERROR nommant le fichier (plus de commentaire silencieux dans `%INCLUDE`).
- `VarMeta.length` documenté « caractères » ; commentaire faux de `procs/append.rs:132` corrigé.
- Snapshots m16 `constructions` et m34 `glimmix_links_laplace` corrigés (tiret cadratin rendu correctement) avec justification ; tout autre snapshot existant modifié est justifié (conflit éventuel traité comme conflit de merge).
- Tests unitaires préfixés `utf8` (LENGTH('é')=1, TITLE non ASCII, BOM, UTF-8 invalide).

### Context
- `lexer/literal.rs:106`, `lexer/token.rs:180,324-330`, `main.rs:46`, `datastep/build.rs:357-361`, `macros/include.rs:143-148`, `dataset.rs:24-32`.
- D-001 (`DECISIONS_LOG.md`).

## J03-P4 — Formats et informats sans découpe d'octets

```yaml
id: J03-P4
kind: implement
tier: T3
size: M
depends_on: []
files:
  - src/formats/builtin/mod.rs
  - src/formats/builtin/informat.rs
  - src/formats/builtin/helpers.rs
  - src/formats/builtin/tests/
  - src/formats/mod.rs
  - src/formats/userdef/mod.rs
  - src/procs/report/render.rs
  - src/procs/report/output.rs
acceptance:
  - "cargo test -p sasrs --lib multibyte 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "! grep -nE 'out\\.truncate\\(w\\)' src/formats/builtin/mod.rs src/formats/mod.rs"
```

### Scope
- `$w.`, `$CHAR`, `$F`, `$QUOTE`, `$HEX`, `$UPCASE` et le repli de `formats/mod.rs` : troncature et remplissage en caractères ; plus aucune panique sur du multioctet.
- `right_justify` : largeur en caractères (plus de « afé » pour « Café » w=4) ; `$w.` informat en caractères ; informat DATE protégé contre le non-ASCII ; largeur par défaut des formats utilisateur en caractères.
- PROC REPORT : `pad_cell` et largeurs de colonnes (`render.rs`), longueurs inférées de `OUT=` (`output.rs`) en caractères.
- Tests unitaires préfixés `multibyte` (accents, CJK, emoji, largeur limite, espaces finaux) pour chaque fonction corrigée.

### Context
- `formats/builtin/mod.rs:214-280`, `formats/mod.rs:337-444`, `formats/builtin/informat.rs:57-139`, `formats/builtin/helpers.rs:5-11,300`, `formats/userdef/mod.rs:190-197`, `procs/report/render.rs:247-330`, `procs/report/output.rs:39-45`.
- `docs/encoding.md` (J03-P3), D-001.

## J03-P5 — Largeurs de mise en page et longueurs inférées en caractères

```yaml
id: J03-P5
kind: implement
tier: T4
size: M
depends_on: []
files:
  - src/listing.rs
  - src/procs/common/format.rs
  - src/output/rtf.rs
  - src/output/pdf.rs
  - src/procs/iml/render.rs
  - src/procs/format/cntl.rs
  - src/procs/distance.rs
  - src/procs/freq/oneway.rs
acceptance:
  - "cargo test -p sasrs --lib char_width 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- Centrage des titres et largeurs de colonnes du listing, de RTF, PDF et IML calculés en caractères (le formatage `{:<w$}` compte déjà en caractères).
- Longueurs `VarMeta` inférées depuis les données en caractères : CNTLOUT, DISTANCE, FREQ `OUT=`.
- Tests unitaires préfixés `char_width` (table avec valeurs accentuées alignée, longueur CNTLOUT d'un label accentué).
- Ne pas : toucher à TRANSPOSE (réécrit en J07-P4) ni à UNIVARIATE (J03-P2).

### Context
- `listing.rs:51,106-173`, `procs/common/format.rs:6`, `output/rtf.rs:88-93`, `output/pdf.rs:100-105`, `procs/iml/render.rs:47-50`, `procs/format/cntl.rs:318-331`, `procs/distance.rs:311`, `procs/freq/oneway.rs:286-291`.

## J03-P6 — Divergences de l'étape DATA

```yaml
id: J03-P6
kind: implement
tier: T3
size: M
depends_on: []
files:
  - src/datastep/exec/update_modify.rs
  - src/datastep/functions/char/
  - src/datastep/functions/tests/
  - src/datastep/mod.rs
  - src/datastep/program.rs
  - src/datastep/tests/
  - tests/fixtures/j03/datastep_
  - tests/snapshots/snapshot__fixtures@j03__datastep_
acceptance:
  - "cargo test -p sasrs --lib datastep_divergence 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- `UPDATE` : les transactions sans maître sont ajoutées, toutes les transactions d'une même clé s'appliquent dans l'ordre, valeurs manquantes de transaction sans effet (sauf `UPDATEMODE=NOMISSINGCHECK`) — valeurs attendues copiées de l'exemple documenté SAS (URL en commentaire).
- `FIND` : position de départ selon la doc (vérifier `FIND('abc','a')` = 1) ; corriger le test `intnx.rs` qui verrouille l'erreur.
- FORMAT/ATTRIB sur une variable inconnue : la variable est créée comme en SAS.
- `REPEAT` plafonné en caractères ; STRIP/CATS/CATX/COMPBL : blancs retirés selon la doc (espaces, pas les tabulations).
- Tests unitaires préfixés `datastep_divergence` ; chaque valeur attendue porte sa source (doc SAS citée) ; fixtures `tests/fixtures/j03/datastep_*.sas`.

### Context
- `datastep/exec/update_modify.rs:9-12`, `functions/char/search.rs:107-137`, `functions/char/transform.rs:103-166`, `functions/char/concat.rs:17-30`, `datastep/mod.rs:444-449`, `datastep/program.rs:109-111`, `functions/tests/intnx.rs:146-151`.

## J03-P7 — Documentation de couverture (J03)

```yaml
id: J03-P7
kind: implement
tier: T5
size: S
depends_on: [J03-P1, J03-P2, J03-P3, J03-P4, J03-P5, J03-P6]
files:
  - README.md
  - docs/support-contract.md
acceptance:
  - "grep -q 'docs/encoding.md' README.md"
  - "grep -q 'EXCLNPWGT' README.md"
```

### Scope
- `README.md` : lignes MEANS/SUMMARY et UNIVARIATE (WEIGHT désormais pondéré, y compris `OUTPUT OUT=` ; ce qui reste non couvert), section encodage courte + lien `docs/encoding.md`, DATA step (UPDATE, FIND), note de la limite « caractères » dans la ligne `LENGTH`.
- `docs/support-contract.md` : lignes J03.
- Audit README/PLAN/PROGRESS vs comportement réel pour les zones touchées en J03.

### Context
- Diffs J03 ; `CONTRIBUTING.md` (quatre états).

## J03-P8 — Review J03

```yaml
id: J03-P8
kind: review
tier: T2
size: S
depends_on: [J03-P1, J03-P2, J03-P3, J03-P4, J03-P5, J03-P6, J03-P7]
files: []
acceptance:
  - "python3 tools/oracles/weighted_stats.py --check"
  - "python3 -c \"import json; d=json.load(open('tests/oracles/weighted_stats.json')); assert len(d['cases'])>=8 and all('provenance' in c and 'expected' in c for c in d['cases'])\""
  - "cargo test -p sasrs --test weighted_oracle 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "grep -q 'fn weighted_quantile_def5' src/procs/common/stats.rs"
  - "test -f docs/encoding.md && grep -q 'caractères' docs/encoding.md && grep -q 'UTF-8' docs/encoding.md"
  - "cargo test -p sasrs --lib utf8 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "! LC_ALL=C grep -qP '\\xc3\\xa2\\xc2\\x80' tests/snapshots/snapshot__fixtures@m16__constructions.sas.snap tests/snapshots/snapshot__fixtures@m34__glimmix_links_laplace.sas.snap"
  - "cargo test -p sasrs --lib multibyte 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "! grep -nE 'out\\.truncate\\(w\\)' src/formats/builtin/mod.rs src/formats/mod.rs"
  - "cargo test -p sasrs --lib char_width 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "cargo test -p sasrs --lib datastep_divergence 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "grep -q 'docs/encoding.md' README.md"
  - "grep -q 'EXCLNPWGT' README.md"
  - "scripts/ci-status.sh consolidation"
  - "cargo test -p sasrs"
  - "cargo build --features graphics && cargo build --features s3"
  - "cargo fmt --check && cargo clippy --all-targets -- -D warnings"
```

### Scope
Review the merged milestone diff with verify-before-done, code-review, test-design. Report; change nothing.
