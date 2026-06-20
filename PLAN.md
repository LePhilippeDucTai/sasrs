# `sasrs` — Plan d'implémentation

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
| Sortie | Log SAS (NOTE:/WARNING:/ERROR:, compteurs d'obs, temps réel/CPU) + listing texte. `--deterministic` fige les temps pour les snapshots. Divergence assumée : pas de date/numéro de page (≡ NODATE NONUMBER). **À partir de M22 : couche ODS par trait `OutputDestination` (HTML/RTF/PDF/Excel + ODS OUTPUT→datasets), le listing texte restant la destination par défaut byte-identique.** |
| Graphiques (M29+) | Images **PNG/SVG via crate `plotters`** routées par ODS GRAPHICS (feature `graphics`). Snapshots = log/listing + assertion d'existence + format/dimensions de l'image, jamais le pixel. Pas d'UI temps réel. |
| Numérique statistique (M24+) | **Fait maison** dans `src/stat/` (lois normale/t/F/χ²/gamma/bêta + algèbre linéaire : Cholesky, QR, moindres carrés, Jacobi), pour le déterminisme et la stabilité des snapshots. Crates externes réservées à l'I/O lourd (`calamine`, `rust_xlsxwriter`, `printpdf`, `plotters`). PRNG MT19937 maison, graine figée sous `--deterministic`. |
| Périmètre | **Élargi (roadmap M14–M30, voir PROGRESS.md)** vers une parité large SAS 9.4 Base + STAT + graphiques statiques : I/O fichiers plats (INFILE/INPUT/DATALINES, FILE/PUT, IMPORT/EXPORT), bibliothèque de fonctions complète, hash, compléments SQL/macro/formats, complétion des procs, **ODS** (HTML/RTF/PDF/Excel + ODS OUTPUT→datasets), **modélisation statistique** (TTEST/NPAR1WAY/REG/ANOVA/GLM/LOGISTIC/GENMOD/PRINCOMP/FACTOR/CLUSTER/DISCRIM/MIXED/GLIMMIX) et **graphiques** (SGPLOT/GPLOT/GCHART/PLOT en images PNG/SVG). Seul reste hors périmètre l'interactivité temps réel (pas d'UI : les graphiques sont des fichiers image). Procs SAS Viya (CAS) : hors périmètre. |

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
| `src/datastep/mod.rs` | ✅ | **Fable** | élevé | compilation : PDV en ordre de première référence, inférence type/longueur, KEEP/DROP, output implicite — sémantique SAS dense |
| `src/datastep/exec.rs` | ✅ | **Fable** | élevé | boucle implicite (fin d'étape AU MILIEU de l'itération sur EOF), flux NextIter/EndStep, builders de sortie, NOTEs exactes |
| `src/datastep/eval.rs` | ✅ | **Opus** | moyen-élevé | coercitions, propagation missing, comparaisons via `sas_cmp`, notes de conversion |
| `src/datastep/functions.rs` | ✅ | **Sonnet** | moyen | dispatch table-driven ~25 fonctions (SUM ignore les missings !), tests table-driven |
| `src/executor.rs` | ✅ | **Opus** | moyen | boucle blocs→exécution, exécution des statements globaux, timing |
| `src/procs/mod.rs` | ✅ | **Sonnet** | faible | registre parse/execute des procs |
| `src/procs/print.rs` | ✅ | **Sonnet** | moyen | PROC PRINT (Obs/VAR/NOOBS, alignements, _LAST_) |
| `tests/snapshot.rs` | ✅ | **Sonnet** | faible | actif — les 3 fixtures m1/ sont verrouillées (log + listing + exit), vérifiées à la main |

**Definition of done M1** : `cargo test -p sasrs` vert avec les snapshots activés ;
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
| Persistance VarMeta (`dataset.rs`) | M4 | **Opus** | moyen | ✅ FAIT (box 2). Polars 0.46 `ParquetWriter` n'expose **aucune** API KV parquet → format/label persistés dans un sidecar JSON `<table>.parquet.sasmeta.json` (écrit seulement si une var porte un format/label → round-trip identique sinon, snapshots stables). API isolée dans `dataset.rs`. |
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
| `S3Library` derrière feature `s3` | M8 | ✅ | moyen | même trait `LibraryProvider`, scan parquet via URI `s3://` ; non branché ; cloud réel = features Polars `cloud`/`aws` (suite) |
| `src/datastep/fastpath.rs` — fast-path vectorisé des steps simples (SET+assign → LazyFrame) | M8 | ✅ | élevé | opt-in (`Session.vectorize`/`--vectorize`), OFF par défaut ; v1 = SET simple + assignations numériques (littéraux/copies/+−*), prouvé équivalent au chemin ligne-à-ligne (tests bit-à-bit + log) ; subsetting IF / `/` / `**` / char repliés sur la boucle |

### Jalons M9–M11 (extension — roadmap dans PROGRESS.md)

| Fichier / tâche | Jalon | Modèle | Effort | Notes |
|---|---|---|---|---|
| `src/procs/common.rs` | M9 | ✅ | moyen | `decode_column`/`sample_std`/`partition_numeric`/`group_by_keys` extraits (verbatim) ; means/freq/univariate/sort/transpose/append rebranchés ; refactor pur, sorties inchangées (`resolve_input` laissé par-proc) |
| `src/procs/corr.rs` | M9 | ✅ | moyen | Pearson (VAR/WITH), Simple Statistics, matrice r + `Prob>|r|` (t-CDF via betai), N appariés ; NOSIMPLE/NOPROB/NOCORR ; OUT= = erreur (suite) ; réutilise common |
| `src/procs/rank.rs` | M9 | ✅ | moyen | VAR/RANKS, GROUPS=, TIES=(MEAN/LOW/HIGH/DENSE), DESCENDING, OUT= ; collation `sas_cmp`, missing→missing ; BY + méthodes alt. = erreur (suite) |
| `src/procs/tabulate.rs` | M9 | ✅ | élevé | v1 listing : CLASS/VAR, `table` 1–2 dims (empilement/croisement/parenthèses), stats N/NMISS/SUM/MEAN/MIN/MAX/STD ; en-têtes plats, 3ᵉ dim + croisements 2 VAR/stats + PCTN/formats différés (erreurs) |
| `src/procs/report.rs` | M9 | ✅ | élevé | v1 listing : COLUMN + DEFINE (DISPLAY/ORDER/GROUP/ANALYSIS+stat) ; détail ou sommaire groupé ; ACROSS/COMPUTE/BREAK/RBREAK/LINE/WHERE/OUT= différés (erreurs) |
| BY-group + WEIGHT + CI dans means/univariate ; CHISQ + options FREQ | M10 | **Opus** | élevé | étend les procs M5 ; `partition_weighted`, quantile t, χ² Pearson |
| `src/macros/` (depuis `preprocess.rs`) — processeur macro complet | M11 | **Fable** | élevé | voir §Macro M11 ci-dessous ; 7 unités incrémentales |

### Macro M11 — architecture (décision actée)

> **MAJ M32** : le processeur macro vit désormais dans le module `src/macros/`
> (`mod.rs` façade + `error`/`scan`/`symbols`/`quoting`/`eval`/`functions`/`control`/
> `define`/`include`/`expand`). `src/preprocess.rs` n'est plus qu'un shim de re-export
> (`pub use crate::macros::*`) ; les références historiques à `preprocess.rs` ci-dessous
> désignent ce module.

Modèle choisi : **expansion texte→texte PRE-lexer, interfoliée** avec la boucle de
`executor::run_program`, état dans `Session` (pas de transformeur de tokens : les corps
de `%macro` contiennent du texte SAS arbitraire, et `%str`/`%nrstr` masquent des caractères
au scanner — naturellement un travail au niveau texte ; c'est aussi le modèle réel de SAS).

- **État** : nouveau `pub macro_engine: MacroEngine` sur `Session` (construit dans `Session::new`
  depuis `deterministic`). `MacroEngine { symbols: SymbolTable{global, scopes}, macros: HashMap<String,MacroDef>, deterministic, guard }`.
- **Seam** : `executor::run_program` ne reçoit plus un source pré-expansé ; il itère sur des
  **segments bruts** (`RawSegmenter`, découpe aux frontières top-level en respectant
  `%macro…%mend` et le masquage `%str/%nrstr`), appelle `macro_engine.expand_open_code(raw)`,
  puis lexe/parse/exécute le texte expansé via un `StatementStream` transitoire (les bras du
  match `Block` sont réutilisés tels quels). `lib.rs` cesse de pré-expanser.
- **CALL SYMPUT** : `EvalCtx` (construit par étape, sans `&mut Session`) gagne
  `symput_writes: Vec<(String,String)>` ; `DsStmt::CallRoutine` y pousse ; APRÈS l'exécution de
  l'étape, `exec::execute` draine vers `macro_engine.set_symbol_global` (visible au segment
  suivant — fidèle à SAS : invisible dans la même étape). `SYMGET` lit un instantané
  `EvalCtx.macro_symbols_snapshot` pris en début d'étape.
- **Écho log** : afficher les n° de ligne ORIGINAUX (pas le texte généré ; MPRINT hors périmètre).
- **Vars auto / déterminisme** : `&SYSDATE9`/`&SYSTIME`/`&SYSVER` figées sous `--deterministic`
  (sinon snapshots instables). `today_sas()` doit devenir deterministic-aware si SYSDATE9 en dérive.
- **Invariant de bascule (M11.7)** : `expand_open_code` est l'IDENTITÉ pour tout segment sans
  déclencheur macro résolu → les 789 tests + snapshots restent octet-identiques sans `--features`.
- Découpage en 7 unités commit+push : voir PROGRESS.md (M11.1 … M11.7).

### Phase E (M31–M35) — qualité & complétion (roadmap dans PROGRESS.md)

Chantier demandé après la complétion de M30. Trois axes : (1) **refactorisation en style
fonctionnel** + généralisation/réduction de complexité (M31, M32) ; (2) **complétion maximale
des options** des procs partiellement supportés (M33, M34) ; (3) **support total des macros**
(M35). Invariant sur les jalons de refactor : sortie **octet-identique** (zéro `.snap.new`),
commits d'extraction « move-only ». Les jalons de complétion font rétrécir en miroir la colonne
« non couvert » des tableaux de couverture de `README.md`.

| Tâche | Jalon | Modèle | Effort | Notes |
|---|---|---|---|---|
| ✅ Couche de parsing PROC partagée (`src/procs/common.rs` : `parse_proc_options`/`parse_proc_body`/`expect_eq`/`parse_dataset_opt`/`unknown_option_error`/`resolve_last_dataset` + `parse_by`/`parse_var_list`/`parse_class`/`parse_weight`) puis migration des ~40 procs | M31 | **Opus** | élevé | **FAIT** : combinateurs purs pilotés par closure `FnMut(&mut StatementStream,&str)->Result<bool>` ; migration par tiers (canaris `print`/`sort` → Tier B 6/7 → Tier C 6/6 → Tier D 16/17) ; `unknown_option_error` reproduit message+span à l'octet ; messages divergents (`means`/`freq`/…), `iml`, `catalog`, `report` body gardés inline. ~−1500 lignes, 0 `.snap.new` |
| ✅ Scission `src/preprocess.rs` → module `src/macros/` (`mod`/`error`/`scan`/`symbols`/`quoting`/`eval`/`functions`/`control`/`define`/`include`/`expand`) | M32 | **Opus** | élevé | **FAIT** : `preprocess.rs` (4757 l.) → 12 fichiers `src/macros/` (façade `mod.rs` 1497 l. : struct + `new`/`expand_open_code`/`TextStage`/`RawSegmenter`) ; façade `preprocess` re-export (imports inchangés) ; déplacements verbatim via blocs `impl MacroEngine` en sous-module (0 changement d'appel) ; généralisations livrées : `apply_quoting` unifié (5 fns quoting + `%q*`), registre `functions::lookup` (string-fns) ; `tokenize_eval` déjà partagé. Octet-identique |
| Complétion options procs Base/descriptifs : FREQ (BY/WEIGHT/LIST/≥3 voies/Fisher r×c), UNIVARIATE (probplot/cdfplot/pondéré), MEANS (WAYS/TYPES/percentiles), TABULATE (OUT=/4ᵉ dim/PCTN<>), REPORT (FORMAT=/COMPUTE complexe), PRINT/CONTENTS/DATASETS/SORT/APPEND | M33 | **Opus/Fable** | élevé | une case = un proc/lot ; fixtures `tests/fixtures/m33/` + snapshots vérifiés ; README 🟡→✅ |
| Complétion options procs stat/modélisation : CORR (partial/Hoeffding/pondéré), TTEST/NPAR1WAY (BY/scores/exact), REG (NOINT/SELECTION=), ANOVA/GLM (interactions/CLASS multiples), LOGISTIC/GENMOD (CLASS/LINK=/DIST=GAMMA/multinomial), MIXED/GLIMMIX (AR(1)/UN/NOINT/LAPLACE), PRINCOMP/FACTOR/DISCRIM (OUT= scoring), CLUSTER (OUTTREE=), IML (SHAPE/DET/EIGEN/`a:b`), graphiques résiduels | M34 | **Opus/Fable** | très élevé | oracles vérifiés vs SAS 9.4 ; numérique maison `src/stat/` ; fixtures `tests/fixtures/m34/` |
| Macro complétion totale : `%SYSFUNC` délégué à toute la lib `functions::call`, `%INCLUDE` fileref/non-quoté/stdin, `%LENGTH("")`→1, vars auto restantes, audit exhaustif statements/fonctions macro | M35 | **Opus** | élevé | processeur toujours actif ; nouveau comportement seulement sur nouvelles directives → snapshots m1–m34 inchangés ; tableau Macro README → ✅ |

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

- `cargo test -p sasrs` : tests unitaires des fondations (lexer, sas_cmp,
  NaN-payload, log, listing) — déjà verts ; snapshots insta dès la fin de M1.
- Snapshots : `tests/snapshot.rs` exécute chaque `tests/fixtures/**/*.sas` en
  `--deterministic` et verrouille `log + listing + code retour`. Les parquets d'entrée
  sont générés par `tests/common/mod.rs` (clone de sashelp.class), jamais commités.
- Oracle : pour les nouvelles fixtures, comparer à une sortie SAS 9.4 réelle (ou WPS /
  documentation SAS) avant d'accepter le snapshot — verrouiller la fidélité, pas
  l'auto-cohérence.
- Manuel : `cargo run -p sasrs --bin sasrs -- tests/fixtures/m1/set_filter.sas`
  (depuis un répertoire contenant `data/class.parquet`).
