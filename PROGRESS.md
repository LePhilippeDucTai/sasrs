# Avancement `sas_interpreter` — curseur de la skill sasrs-impl

Ce fichier est l'état d'avancement machine-lisible du projet. La skill
`sasrs-impl` le lit pour savoir où reprendre, et le met à jour DANS LE MÊME
COMMIT que le code livré. Ne cocher une case que si : implémentation
complète (zéro `todo!()` restant dans le fichier), tests du fichier écrits,
`cargo test -p sas_interpreter` vert.

Jalon courant : **M1**

## M1 — pipeline exécutable de bout en bout
Ordre strict (dépendances), sauf ⫽ parallélisables :

- [x] `src/parser/mod.rs` — StatementStream, découpeur de blocs, récupération d'erreur (Opus, élevé)
- [x] `src/parser/expr.rs` — Pratt, précédence SAS, littéraux date, missings `.a` (Opus, élevé)
- [ ] `src/parser/datastep.rs` — statements M1 (Opus, moyen)
- [x] ⫽ `src/parser/global.rs` — LIBNAME/TITLE/OPTIONS (Sonnet, faible)
- [x] `src/datastep/pdv.rs` — PDV (Sonnet, moyen)
- [ ] `src/datastep/mod.rs` — compilation PDV, inférence types (Fable, élevé)
- [ ] `src/datastep/eval.rs` — évaluateur, coercitions (Opus, moyen-élevé)
- [ ] ⫽ `src/datastep/functions.rs` — ~25 fonctions table-driven (Sonnet, moyen)
- [ ] `src/datastep/exec.rs` — boucle implicite, builders, NOTEs (Fable, élevé)
- [ ] ⫽ `src/procs/mod.rs` — registre (Sonnet, faible)
- [ ] `src/procs/print.rs` — PROC PRINT (Sonnet, moyen)
- [ ] `src/executor.rs` — boucle blocs, statements globaux, timing (Opus, moyen)
- [ ] Activer `tests/snapshot.rs` (retirer `#[ignore]`), générer/relire les snapshots des 3 fixtures m1/, les vérifier à la main contre le comportement SAS attendu, les committer
- [ ] DoD M1 : `cargo test -p sas_interpreter` vert snapshots inclus ; `sasrs tests/fixtures/m1/set_filter.sas` plausible ; mettre à jour les ✅ dans PLAN.md

## M2 — cœur de l'étape DATA
- [ ] RETAIN, sum statement, LENGTH (compile+exec+parser)
- [ ] DO itératif (TO/BY/WHILE/UNTIL), DELETE
- [ ] ARRAY + indexation
- [ ] KEEP/DROP/RENAME/WHERE en options de dataset (entrées ET sorties), sorties multiples avec OUTPUT ciblé
- [ ] Missings spéciaux bout en bout (round-trip parquet testé), `_ERROR_` + NOTEs d'erreurs runtime
- [ ] Fixtures m2/ + snapshots ; DoD : cargo test vert

## M3 — monde BY
- [ ] `src/procs/sort.rs` (collation missings, NODUPKEY, DESCENDING, OUT=)
- [ ] SET avec BY (interclassement), FIRST./LAST.
- [ ] MERGE avec BY (match-merge exact, IN=, détection désordre) — tests contre sorties SAS calculées à la main
- [ ] Fixtures m3/ + snapshots

## M4 — formats
- [ ] `src/formats/mod.rs` + `builtin.rs` (formats puis informats) + tests table-driven
- [ ] INPUT()/PUT() branchés dans functions.rs ; FORMAT/LABEL/ATTRIB statements
- [ ] `src/formats/userdef.rs` + `src/procs/format.rs`
- [ ] Persistance VarMeta en métadonnées KV parquet (`sas_meta`) + `src/procs/contents.rs`
- [ ] Fixtures m4/ + snapshots

## M5 — procs statistiques
- [ ] `src/procs/means.rs` (CLASS, _TYPE_/_FREQ_, OUTPUT OUT=)
- [ ] `src/procs/freq.rs` (1 voie, 2 voies, MISSING)
- [ ] `src/procs/univariate.rs` (quantiles définition 5)
- [ ] Fixtures m5/ + snapshots

## M6 — PROC SQL
- [ ] `src/sql/parser.rs` + compléments `ast.rs` (tests d'AST)
- [ ] `src/sql/plan.rs` (joins, group/having/order, CALCULATED, remerge + NOTE, missing semantics)
- [ ] CREATE TABLE/DROP/INSERT/DELETE/DESCRIBE ; SELECT nu vers listing
- [ ] Fixtures m6/ + snapshots

## M7 — gestion de données
- [ ] `src/procs/transpose.rs` (BY/ID/VAR, _NAME_/COLn)
- [ ] `src/procs/append.rs` (FORCE)
- [ ] `src/procs/datasets.rs` (+ `rename` dans LibraryProvider)
- [ ] OPTIONS OBS=/FIRSTOBS= ; fonctions lot 2 (INTNX/INTCK, LAG/DIF par site d'appel)
- [ ] Fixtures m7/ + snapshots

## M8 — durcissement
- [ ] Spike macro `%let` derrière feature flag (valider la couture TextStage)
- [ ] Stub `S3Library` derrière feature `s3` (compile, non branché)
- [ ] Fast-path vectorisé optionnel des steps simples
- [ ] Revue checklist pièges (PLAN.md §Checklist) sur tout le code
