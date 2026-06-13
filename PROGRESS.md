# Avancement `sas_interpreter` — curseur de la skill sasrs-impl

Ce fichier est l'état d'avancement machine-lisible du projet. La skill
`sasrs-impl` le lit pour savoir où reprendre, et le met à jour DANS LE MÊME
COMMIT que le code livré. Ne cocher une case que si : implémentation
complète (zéro `todo!()` restant dans le fichier), tests du fichier écrits,
`cargo test -p sas_interpreter` vert.

Jalon courant : **M10** (M1–M9 complets ; extension — voir M10/M11)

## M1 — pipeline exécutable de bout en bout
Ordre strict (dépendances), sauf ⫽ parallélisables :

- [x] `src/parser/mod.rs` — StatementStream, découpeur de blocs, récupération d'erreur (Opus, élevé)
- [x] `src/parser/expr.rs` — Pratt, précédence SAS, littéraux date, missings `.a` (Opus, élevé)
- [x] `src/parser/datastep.rs` — statements M1 (Opus, moyen)
- [x] ⫽ `src/parser/global.rs` — LIBNAME/TITLE/OPTIONS (Sonnet, faible)
- [x] `src/datastep/pdv.rs` — PDV (Sonnet, moyen)
- [x] `src/datastep/mod.rs` — compilation PDV, inférence types (Fable, élevé)
- [x] `src/datastep/eval.rs` — évaluateur, coercitions (Opus, moyen-élevé)
- [x] ⫽ `src/datastep/functions.rs` — ~25 fonctions table-driven (Sonnet, moyen)
- [x] `src/datastep/exec.rs` — boucle implicite, builders, NOTEs (Fable, élevé)
- [x] ⫽ `src/procs/mod.rs` — registre (Sonnet, faible)
- [x] `src/procs/print.rs` — PROC PRINT (Sonnet, moyen)
- [x] `src/executor.rs` — boucle blocs, statements globaux, timing (Opus, moyen)
- [x] Activer `tests/snapshot.rs` (retirer `#[ignore]`), générer/relire les snapshots des 3 fixtures m1/, les vérifier à la main contre le comportement SAS attendu, les committer
- [x] DoD M1 : `cargo test -p sas_interpreter` vert snapshots inclus ; `sasrs tests/fixtures/m1/set_filter.sas` plausible ; mettre à jour les ✅ dans PLAN.md

## M2 — cœur de l'étape DATA
- [x] RETAIN, sum statement, LENGTH (compile+exec+parser)
- [x] DO itératif (TO/BY/WHILE/UNTIL), DELETE
- [x] ARRAY + indexation
- [x] KEEP/DROP/RENAME/WHERE en options de dataset (entrées ET sorties), sorties multiples avec OUTPUT ciblé
- [x] Missings spéciaux bout en bout (round-trip parquet testé), `_ERROR_` + NOTEs d'erreurs runtime
- [x] Fixtures m2/ + snapshots ; DoD : cargo test vert

## M3 — monde BY
- [x] `src/procs/sort.rs` (collation missings, NODUPKEY, DESCENDING, OUT=)
- [x] SET avec BY (interclassement), FIRST./LAST.
- [x] MERGE avec BY (match-merge exact, IN=, détection désordre) — tests contre sorties SAS calculées à la main
- [x] Fixtures m3/ + snapshots

## M4 — formats
- [x] `src/formats/mod.rs` + `builtin.rs` (formats puis informats) + tests table-driven
- [x] INPUT()/PUT() branchés dans functions.rs ; FORMAT/LABEL/ATTRIB statements
- [x] `src/formats/userdef.rs` + `src/procs/format.rs`
- [x] Persistance VarMeta + `src/procs/contents.rs` (persistance format/label via sidecar JSON `<table>.parquet.sasmeta.json` faite en box 2 — Polars 0.46 ParquetWriter n'expose pas d'API KV parquet ; `src/procs/contents.rs` fait)
- [x] Fixtures m4/ + snapshots

## M5 — procs statistiques
- [x] `src/procs/means.rs` (CLASS, _TYPE_/_FREQ_, OUTPUT OUT=)
- [x] `src/procs/freq.rs` (1 voie, 2 voies, MISSING)
- [x] `src/procs/univariate.rs` (quantiles définition 5)
- [x] Fixtures m5/ + snapshots

## M6 — PROC SQL
- [x] `src/sql/parser.rs` + compléments `ast.rs` (tests d'AST)
- [x] `src/sql/plan.rs` (joins, group/having/order, CALCULATED, remerge + NOTE, missing semantics) — LIKE limité à préfixe/suffixe/exact (feature regex Polars non activée) ; EXCEPT/INTERSECT ALL approximés
- [x] CREATE TABLE/DROP/INSERT/DELETE/DESCRIBE ; SELECT nu vers listing
- [x] Fixtures m6/ + snapshots

## M7 — gestion de données
- [x] `src/procs/transpose.rs` (BY/ID/VAR, _NAME_/COLn)
- [x] `src/procs/append.rs` (FORCE)
- [x] `src/procs/datasets.rs` (+ `rename` dans LibraryProvider)
- [x] OPTIONS OBS=/FIRSTOBS= ; fonctions lot 2 (INTNX/INTCK, LAG/DIF par site d'appel) — OBS=/FIRSTOBS= appliqués à l'entrée SET de l'étape DATA (pas encore aux lectures des procs)
- [x] Fixtures m7/ + snapshots

## M8 — durcissement
- [x] Spike macro `%let` derrière feature flag (valider la couture TextStage)
- [x] Stub `S3Library` derrière feature `s3` (compile, non branché) — trait `LibraryProvider` sur scan parquet via URI `s3://`, mutations renvoient une erreur ; I/O cloud réelle = suite (features Polars `cloud`/`aws`)
- [x] Fast-path vectorisé optionnel des steps simples — `src/datastep/fastpath.rs` (opt-in `Session.vectorize` / CLI `--vectorize`, OFF par défaut). Périmètre v1 prouvé équivalent : SET d'un seul dataset (sans BY/MERGE/WHERE), assignations numériques (littéraux, copies préservant le NaN-payload, +/-/*), une sortie, output implicite ; tout le reste retombe sur la boucle ligne-à-ligne via `eligible()`. NOTE "missing generated" répliquée. 10 tests (équivalence bit-à-bit output + log, préservation `.A`, rejets du garde-fou, repli)
- [x] Revue checklist pièges (PLAN.md §Checklist) sur tout le code — 8/8 conformes : nullify_specials (eager + réplique lazy sql), sas_cmp partout (sort/means/freq/transpose/univariate via décodage Value, jamais d'agrégation native sur les spéciaux), aucun get_row, WARNING i64>2^53, collation tri par sas_cmp, troncature char + comparaison sans blancs finaux, NOTEs "variables." invariable, LAG/DIF FIFO par site d'appel (clé = ptr args). Seul correctif : dé-duplication de la normalisation des missings dans sql/plan.rs (scan_normalized délègue à normalize_specials)

## M9 — nouveaux PROCs
Ordre (effort/dépendances croissants), un incrément (= une case) par invocation.
Pattern d'ajout : nouveau `src/procs/<nom>.rs` (`<Nom>Ast` + `parse(ts)` + `execute(ast, session)`),
`pub mod` + variante `ProcAst`, bras dans `parse_proc`/`execute_proc` (mod.rs), lecture via
`provider.read()`+`forward`, rendu via `listing.page_header()`+`write_table`, `last_dataset` si OUT=.

- [x] `src/procs/common.rs` — helpers partagés extraits (verbatim) : `decode_column` (6 copies identiques fusionnées), `sample_std`, `partition_numeric`, `group_by_keys` (= ex-`means::group_by_class`). means/freq/univariate/sort/transpose/append rebranchés, imports morts nettoyés. Refactor pur : **789 tests + snapshot inchangés, zéro warning**. (`resolve_input` laissé par-proc : implémentations non identiques — hors périmètre de cet incrément)
- [x] `src/procs/corr.rs` — PROC CORR Pearson : VAR/WITH (défaut VAR = numériques), NOSIMPLE/NOPROB/NOCORR ; Simple Statistics + matrice de coefficients (r 5 décimales, `Prob > |r|` via t-CDF = beta incomplète régularisée, N par cellule si N appariés varient), observations appariées-complètes, variable constante → `.`. OUT= → erreur "not yet implemented" (suite). 18 tests. (corrigé : valeur attendue d'un test de r erronée côté agent)
- [x] `src/procs/rank.rs` — PROC RANK : VAR (défaut numériques) / RANKS (ajout vs remplacement), GROUPS=, TIES=(MEAN/LOW/HIGH/DENSE), DESCENDING, OUT= (défaut = écrase DATA=) ; collation `sas_cmp`, missing→rang missing exclu du calcul, pass-through préserve les payloads. BY + méthodes FRACTION/PERCENT/NORMAL/SAVAGE → erreur "not yet implemented" (suite). 20 tests (827 total verts, 0 warning)
- [x] `src/procs/tabulate.rs` — PROC TABULATE (v1, listing seul) : CLASS/VAR + `table` 1–2 dimensions, grammaire `term{term}`(empilement) / `factor{*factor}`(croisement) / parenthèses ; stats N/NMISS/SUM/MEAN/MIN/MAX/STD (défaut VAR→SUM, classe seule→N) ; cellules calculées par appariement `sas_cmp` + `partition_numeric`+`means::compute`. Différés (erreurs propres) : 3ᵉ dimension, croisement 2 VAR/2 stats, formats/labels d'en-tête, ALL, PCTN/PCTSUM, MISSING. En-têtes `*`-joints à plat (vs grille SAS). 12 tests (839 total, 0 warning)
- [x] `src/procs/report.rs` — PROC REPORT (v1, listing seul) : COLUMN/COLUMNS + DEFINE (DISPLAY/ORDER/GROUP/ANALYSIS+stat, order=, label) ; rapport détail (1 ligne/obs) OU sommaire groupé (`group_by_keys` + `means::compute`) ; défauts num→ANALYSIS SUM, char→DISPLAY ; noheader. Différés (erreurs propres) : ACROSS, COMPUTE, BREAK/RBREAK, LINE, WHERE, OUT=, options DEFINE avancées. 21 tests (860 total, 0 warning)
- [x] Fixtures `tests/fixtures/m9/` (corr, rank, tabulate, report) + 4 snapshots, **vérifiés à la main vs sashelp.class** : sommes/moyennes/écarts-types exacts, corrélations = valeurs documentées (0.87779/0.81143/0.74089), rangs & quartiles exacts, fréquences F=9/M=10. `cargo test` vert. **M9 TERMINÉ.**

## M10 — stats avancées (procs existants)
- [x] BY-group dans MEANS & UNIVARIATE : statement BY honoré (helper `common::by_groups` — vérifie le tri par clé BY via `sas_cmp`, groupes contigus en ordre d'entrée, erreur "not sorted in ascending sequence" sinon → arrêt). Une section par groupe avec en-tête `var=val` ; MEANS combine BY (externe) × CLASS (interne, _TYPE_/_FREQ_) ; chemin sans-BY byte-identique (m5 inchangés). + UNIVARIATE `OUTPUT OUT=` (mean/std/min/max/median/q1/q3/pNN/n/nmiss/sum/range/qrange, 1 ligne/groupe BY). +8 tests (868 total, 0 warning)
- [ ] WEIGHT dans MEANS & UNIVARIATE (et CORR si livré) : stats pondérées via `partition_weighted` (D2 ; poids ≤0/missing exclus)
- [ ] Intervalles de confiance MEANS : `ALPHA=`, `CLM`/`LCLM`/`UCLM` ; helper quantile t de Student (`common.rs`)
- [ ] FREQ : test CHISQ (Pearson χ², ddl, p-value, 2 voies) ET application réelle de NOROW/NOCOL/NOPERCENT/NOFREQ/NOCUM (parsés puis ignorés : `freq.rs:192-199`)
- [ ] Fixtures `tests/fixtures/m10/` + snapshots ; DoD : cargo test vert ; jalon courant → M11

## M11 — macro complet (promu ON par défaut)
Architecture (D3 + design détaillé, voir PLAN.md §Macro M11) : expansion **texte→texte
interfoliée** pilotée par l'exécuteur, état (`MacroEngine`) dans `Session` ; nouveau module
`src/macros/` (promu depuis `preprocess.rs`). Un segment brut est expansé PUIS lexé/parsé/exécuté ;
`CALL SYMPUT` écrit la table après l'étape, vu par le segment suivant.

- [ ] M11.1 — déplacer l'expansion dans la boucle exécuteur ; `Session.macro_engine` ; segmenteur brut + expand→lex par segment. `%let`/`&var` IDENTIQUE (réutilise `read_name`/`resolve_value`/`resolve_refs_once`). **Invariant pass-through octet-identique** (source sans macro → inchangé) + écho des n° de ligne ORIGINAUX. Flag conservé ; tests de non-régression
- [ ] M11.2 — `%macro name(params)/%mend` + invocation `%name(args)` : table de définitions, paramètres positionnels + mots-clés (défauts), expanseur récursif (garde de profondeur), `%local`/`%global`
- [ ] M11.3 — `%if/%then/%else/%do/%end` + `%do i=a %to b [%by k]` itératif (génération de texte ; garde d'itérations)
- [ ] M11.4 — `%eval` : arithmétique entière (`/` tronque), comparaisons, logique ; câblé dans %if/%to/%by
- [ ] M11.5 — `CALL SYMPUT`/`SYMGET` : `DsStmt::CallRoutine`, `EvalCtx.symput_writes` (drainé APRÈS l'étape → écrit `session.macro_engine` global) + `EvalCtx.macro_symbols_snapshot` (SYMGET lit l'instantané début d'étape). Parser `call symput(...)` dans `parser/datastep.rs`
- [ ] M11.6 — `%sysfunc(fn(args))` (liste blanche `functions::call`) ; vars auto `&SYSDATE9`/`&SYSTIME`/`&SYSVER` (FIGÉES sous `--deterministic`) ; quoting `%str`/`%nrstr` (sentinelles ; `%bquote`/`%superq` en suite) ; indirection `&&&`
- [ ] M11.7 — retrait du feature flag `macros` (ON par défaut ; supprimer les `#[cfg(feature="macros")]`). **Gate : toute la suite (789 tests + snapshots) verte SANS `--features`, snapshots octet-identiques**
- [ ] Fixtures `tests/fixtures/m11/` + snapshots ; DoD : cargo test vert, flag retiré
