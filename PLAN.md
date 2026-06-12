# `sas_interpreter` — Plan d'implémentation

Interpréteur du langage **SAS** (référence : SAS 9.4 classique, pré-Viya) écrit en **Rust**,
moteur de données **Polars**, tables au format **Parquet**. Pas d'UI : un binaire batch
`sasrs script.sas` qui produit une **log fidèle SAS** et un **listing** texte.

Ce document est la feuille de route du projet. Chaque fichier source non encore implémenté
existe déjà en **squelette compilable** : le plan détaillé du fichier (sémantique SAS à
respecter, pièges, algorithmes) est dans le doc-commentaire en tête du fichier, les
signatures publiques sont posées, les corps sont en `todo!()`. Un agent peut donc prendre
un fichier, lire son en-tête, et l'implémenter sans contexte supplémentaire.

## Décisions actées (ne pas rediscuter)

| Sujet | Décision |
|---|---|
| Stockage | `LIBNAME lib '/chemin';` = dossier local ; table = `<dossier>/<nom>.parquet` ; `WORK` = tempdir. Trait `LibraryProvider` pour brancher S3 plus tard (features Polars `cloud, aws` derrière un feature flag `s3`). |
| Types | Modèle SAS strict : Numérique (f64) + Caractère uniquement. Dates = nombres (epoch **1960-01-01**) portées par des formats d'affichage. Parquet natif (int/date/datetime/bool) coercé en f64 à la lecture. |
| Missings | `.` ⇔ null Polars ; spéciaux `._`, `.A`–`.Z` ⇔ NaN à payload (préservé par parquet). `missing::nullify_specials()` obligatoire avant tout calcul Polars natif. |
| Macro | Phase ultérieure ; l'emplacement (`preprocess::TextStage`) est réservé dès maintenant et le parsing est incrémental bloc par bloc pour l'accueillir. |
| PROC SQL | Parser dédié du dialecte SAS (CALCULATED, remerge, lib.table) → Polars lazy. Pas le SQLContext Polars. |
| Sortie | Log SAS (NOTE:/WARNING:/ERROR:, compteurs d'obs, temps réel/CPU) + listing texte. `--deterministic` fige les temps pour les snapshots. Divergence assumée : pas de date/numéro de page (≡ NODATE NONUMBER). |
| Périmètre | Étape DATA complète + PROC SORT, MEANS/SUMMARY, TRANSPOSE, FREQ, SQL, UNIVARIATE, PRINT, CONTENTS, DATASETS, APPEND, FORMAT. Procs SAS Viya : hors périmètre. |

## Architecture

```
source .sas
  → preprocess::TextStage        (identité aujourd'hui ; macro %let/&var demain)
  → lexer::Lexer                 (tokens + spans)
  → parser::StatementStream      (découpe en blocs : global | DATA | PROC — UN bloc
                                  à la fois, exécuté avant de parser la suite)
  → executor                     (écho log, dispatch, timing, récupération d'erreur)
       ├─ étape DATA : datastep::compile (PDV) → datastep::exec (boucle implicite)
       ├─ PROCs : procs::* (un sous-parser + un exécuteur par proc)
       └─ PROC SQL : sql::parser → sql::plan (LazyFrame)
  ↘ session : LibraryManager (librefs→parquet), LogWriter, ListingWriter, options
```

## État, modèle suggéré et effort par fichier

Légende état : ✅ implémenté + tests unitaires · 🦴 squelette (plan en tête de fichier, corps `todo!()`).
Modèle suggéré : **Sonnet** (économique — tâche cadrée/mécanique), **Opus** (raisonnement
soutenu), **Fable** (sémantique SAS pointue, risques d'erreurs subtiles). L'effort est celui
du modèle suggéré. Un modèle supérieur peut toujours prendre la tâche d'un inférieur ;
l'inverse est déconseillé pour les fichiers marqués Fable.

### Fondations (jalon M1 — faites)

| Fichier | État | Rôle | Revue par |
|---|---|---|---|
| `Cargo.toml` (workspace + crate) | ✅ | Polars 0.46 (lazy, parquet, dtypes), clap, insta | — |
| `src/error.rs` | ✅ | SasError (parse/runtime/io/polars) | — |
| `src/source.rs` | ✅ | spans → lignes (écho log) | — |
| `src/token.rs` | ✅ | tokens, spans, suffixes de littéraux `d/t/dt/n` | — |
| `src/lexer.rs` | ✅ | lexer manuel : opérateurs-mots SAS, `'...'d`, commentaires | — |
| `src/value.rs` | ✅ | `Value`, 28 missings, `sas_cmp` (`.=.` vrai !), BESTw. | — |
| `src/missing.rs` | ✅ | NaN-payload ⇔ missings spéciaux, `nullify_specials` | — |
| `src/dataset.rs` | ✅ | lecture/écriture parquet + coercition types→SAS (dates 1960) | — |
| `src/library.rs` | ✅ | trait `LibraryProvider`, `DirLibrary`, WORK tempdir | — |
| `src/log.rs` | ✅ | écho numéroté, NOTE/WARNING/ERROR, timing réel/CPU | — |
| `src/listing.rs` | ✅ | titres centrés, tables monospace | — |
| `src/session.rs` | ✅ | état de session (libs, log, listing, options, _LAST_) | — |
| `src/preprocess.rs` | ✅ | `TextStage` (emplacement macro) | — |
| `src/ast.rs` | ✅ | AST blocs/expressions/étape DATA | — |
| `src/lib.rs`, `src/main.rs` | ✅ | API `run()`, CLI `sasrs` (clap) | — |
| `tests/common/mod.rs` | ✅ | génération parquet sashelp.class | — |

### Jalon M1 — rendre le pipeline exécutable (à coder maintenant)

| Fichier | État | Modèle | Effort | Contenu |
|---|---|---|---|---|
| `src/parser/mod.rs` | ✅ | **Opus** | élevé | `StatementStream`, découpeur de blocs, récupération d'erreur — c'est la pièce architecturale (couture macro + grammaires par proc) |
| `src/parser/expr.rs` | ✅ | **Opus** | élevé | Pratt avec la précédence SAS *inhabituelle* (`NOT` lie fort, `**` droite-associatif), littéraux date, missings spéciaux `.a` |
| `src/parser/datastep.rs` | ✅ | **Opus** | moyen | statements M1 (SET/assign/IF/DO/OUTPUT/KEEP/DROP/STOP), erreurs "not yet implemented" propres |
| `src/parser/global.rs` | ✅ | **Sonnet** | faible | LIBNAME / TITLE / OPTIONS |
| `src/datastep/pdv.rs` | ✅ | **Sonnet** | moyen | PDV : lookup insensible casse, troncature longueur char, reset non-retenues |
| `src/datastep/mod.rs` | 🦴 | **Fable** | élevé | compilation : PDV en ordre de première référence, inférence type/longueur, KEEP/DROP, output implicite — sémantique SAS dense |
| `src/datastep/exec.rs` | 🦴 | **Fable** | élevé | boucle implicite (fin d'étape AU MILIEU de l'itération sur EOF), flux NextIter/EndStep, builders de sortie, NOTEs exactes |
| `src/datastep/eval.rs` | 🦴 | **Opus** | moyen-élevé | coercitions, propagation missing, comparaisons via `sas_cmp`, notes de conversion |
| `src/datastep/functions.rs` | 🦴 | **Sonnet** | moyen | dispatch table-driven ~25 fonctions (SUM ignore les missings !), tests table-driven |
| `src/executor.rs` | 🦴 | **Opus** | moyen | boucle blocs→exécution, exécution des statements globaux, timing |
| `src/procs/mod.rs` | 🦴 | **Sonnet** | faible | registre parse/execute des procs |
| `src/procs/print.rs` | 🦴 | **Sonnet** | moyen | PROC PRINT (Obs/VAR/NOOBS, alignements, _LAST_) |
| `tests/snapshot.rs` | ✅* | **Sonnet** | faible | *écrit mais `#[ignore]` — retirer l'ignore à la fin de M1, `cargo insta review`, vérifier les 3 fixtures m1/ |

**Definition of done M1** : `cargo test -p sas_interpreter` vert avec les snapshots activés ;
`sasrs tests/fixtures/m1/set_filter.sas` affiche les ados de CLASS avec une log plausible.

### Jalons M2+ (squelettes prêts, à étendre)

| Fichier / tâche | Jalon | Modèle | Effort | Notes |
|---|---|---|---|---|
| Étape DATA : RETAIN, DO itératif, arrays, sum statement, LENGTH, WHERE, options de dataset, sorties multiples | M2 | **Fable** | élevé | étend parser/datastep + compile/exec ; missings spéciaux bout en bout |
| `src/procs/sort.rs` | M3 | **Opus** | moyen | collation : colonne compagnon de rang des missings (le piège est documenté dans le fichier) |
| SET/MERGE avec BY, FIRST./LAST., IN= | M3 | **Fable** | élevé | match-merge SAS exact (persistance du côté court) ; tests contre sorties SAS calculées à la main |
| `src/formats/mod.rs` | M4 | **Sonnet** | moyen | FormatSpec, catalogue, résolution user→builtin→fallback |
| `src/formats/builtin.rs` | M4 | **Sonnet** | moyen-élevé | table-driven, beaucoup de cas ; informat `5.2` piège des décimales implicites |
| `src/formats/userdef.rs` + `src/procs/format.rs` | M4 | **Sonnet** | moyen | plages low-<high, OTHER |
| Persistance VarMeta dans les métadonnées KV parquet (`dataset.rs`) | M4 | **Opus** | moyen | clé `"sas_meta"` JSON ; API KV isolée dans dataset.rs |
| `src/procs/contents.rs` | M4 | **Sonnet** | faible | métadonnées seulement |
| `src/procs/means.rs` | M5 | **Opus** | élevé | combinatoire `_TYPE_`/`_FREQ_` de CLASS |
| `src/procs/freq.rs` | M5 | **Opus** | moyen | 1 et 2 voies, option MISSING |
| `src/procs/univariate.rs` | M5 | **Opus** | moyen-élevé | quantiles **définition 5** à la main (pas ceux de Polars) |
| `src/sql/ast.rs` | M6 | **Sonnet** | faible | types posés, compléter au fil du parser |
| `src/sql/parser.rs` | M6 | **Opus** | élevé | mots-clés contextuels, CALCULATED, BETWEEN/IS NULL/LIKE |
| `src/sql/plan.rs` | M6 | **Fable** | élevé | remerge + NOTE exacte, `= .` → is_null, join_nulls, ORDER BY missings premiers |
| `src/procs/transpose.rs` | M7 | **Opus** | moyen-élevé | nommage `_NAME_`/`COLn`/ID — ne pas utiliser le pivot Polars |
| `src/procs/append.rs` | M7 | **Sonnet** | moyen | règles FORCE |
| `src/procs/datasets.rs` | M7 | **Sonnet** | moyen | run-group, delete/change ; ajouter `rename` au trait LibraryProvider |
| Préprocesseur macro (`preprocess.rs`) : %let, &var, %macro/%mend, %if/%do, CALL SYMPUT | M8 | **Fable** | élevé | la couture existe ; commencer par un spike %let derrière un feature flag |
| `S3Library` derrière feature `s3` | M8 | **Opus** | moyen | même trait, scan/sink cloud Polars |
| Fast-path vectorisé des steps simples (SET+assign+IF → LazyFrame) | M8 | **Fable** | élevé | optionnel, derrière la même interface StepProgram |

### Conseils d'orchestration

- **Ordre M1 strict** : `parser/mod.rs` → `parser/expr.rs` → `parser/{datastep,global}.rs`
  → `datastep/pdv.rs` → `datastep/{mod,eval,functions}.rs` → `datastep/exec.rs` →
  `procs/{mod,print}.rs` → `executor.rs` → activer `tests/snapshot.rs`.
  Paralléliser : `functions.rs`, `global.rs`, `procs/print.rs` sont indépendants.
- Chaque PR/fichier : implémenter AUSSI les tests unitaires esquissés dans l'en-tête.
- Ne jamais contourner `Value::sas_cmp` ni `missing::nullify_specials` (voir la checklist).

## Checklist des pièges (à vérifier à chaque revue)

1. **NaN ≠ null Polars** : `nullify_specials()` avant toute agrégation/jointure native.
2. **`. = .` est VRAI en SAS** : toute comparaison passe par `Value::sas_cmp` ; en SQL,
   `join_nulls` + traduction `= .` → `is_null()`.
3. **Jamais `DataFrame::get_row`** dans la boucle implicite — downcast chunked une fois
   par colonne (déjà la convention dans `datastep::InputData`).
4. **i64 > 2^53** : WARNING perte de précision à la lecture (fait dans `dataset.rs`).
5. **Tri** : les flags nulls de Polars ignorent les missings spéciaux → colonne de rang.
6. **Longueur char fixe** : troncature à l'assignation PDV ; comparaison ignore les blancs finaux.
7. **NOTEs du log au pluriel invariable** ("1 variables.") — c'est fidèle à SAS, ne pas "corriger".
8. **LAG/DIF** (M2+) : une file FIFO **par site d'appel**, pas par variable.

## Vérification

- `cargo test -p sas_interpreter` : tests unitaires des fondations (lexer, sas_cmp,
  NaN-payload, log, listing) — déjà verts ; snapshots insta dès la fin de M1.
- Snapshots : `tests/snapshot.rs` exécute chaque `tests/fixtures/**/*.sas` en
  `--deterministic` et verrouille `log + listing + code retour`. Les parquets d'entrée
  sont générés par `tests/common/mod.rs` (clone de sashelp.class), jamais commités.
- Oracle : pour les nouvelles fixtures, comparer à une sortie SAS 9.4 réelle (ou WPS /
  documentation SAS) avant d'accepter le snapshot — verrouiller la fidélité, pas
  l'auto-cohérence.
- Manuel : `cargo run -p sas_interpreter --bin sasrs -- tests/fixtures/m1/set_filter.sas`
  (depuis un répertoire contenant `data/class.parquet`).
