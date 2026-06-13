# Avancement `sas_interpreter` — curseur de la skill sasrs-impl

Ce fichier est l'état d'avancement machine-lisible du projet. La skill
`sasrs-impl` le lit pour savoir où reprendre, et le met à jour DANS LE MÊME
COMMIT que le code livré. Ne cocher une case que si : implémentation
complète (zéro `todo!()` restant dans le fichier), tests du fichier écrits,
`cargo test -p sas_interpreter` vert.

Jalon courant : **M12** (extensions macro : `%do %while`/`%until` + quoting) puis **M13** (branchement S3 réel). M1–M11 terminés.

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
- [x] WEIGHT dans MEANS & UNIVARIATE : statement `weight var;` ; `common::partition_weighted` (exclut value missing / weight missing / weight≤0) ; stats pondérées VARDEF=DF (SumWgt, Sum=Σwx, Mean=Σwx/Σw, Var=CSS_w/(n−1), Std, StdErr=Std/√ΣW, CV, USS) ; Min/Max non pondérés ; chemin sans-WEIGHT byte-identique. Différés documentés : skew/kurt non pondérés, médiane MEANS non pondérée, quantiles UNIVARIATE omis (note). CORR WEIGHT non livré (suite). +9 tests (877 total, 0 warning)
- [x] Intervalles de confiance MEANS : option `ALPHA=` (défaut 0.05, validée) + stats `CLM`/`LCLM`/`UCLM` ; `common::t_quantile` (inverse CDF t par bissection sur betai, validé t₀.₉₇₅,₁₀≈2.2281) ; demi-largeur = t·stderr (réutilise le StdErr affiché, pondéré si WEIGHT), n≥2 ; en-têtes "Lower/Upper NN% CL for Mean" ; OUTPUT OUT= câblé. Chemin par défaut byte-identique. +10 tests (887 total, 0 warning)
- [x] FREQ : CHISQ deux voies (Pearson + Likelihood Ratio : valeur, ddl=(r−1)(c−1), Prob via `common::chisq_sf` = Q(df/2, x/2) incomplète gamma ; table dégénérée gérée) ET NOFREQ/NOPERCENT/NOROW/NOCOL/NOCUM appliqués réellement (suppression des lignes/colonnes, 1 et 2 voies). CHISQ 1 voie différé. Sortie par défaut byte-identique (m5/freq inchangé). +8 tests (895 total, 0 warning)
- [x] Fixtures `tests/fixtures/m10/` (by_group, weight, confint, freq_chisq) + 4 snapshots **vérifiés à la main** : moyennes par groupe + min/max exacts, pondéré 14/6 & √(5/3) & Σwx=14, IC height [59.866, 64.808] (t₀.₉₇₅,₁₈), χ² 2×2 = 0.0586/ddl1/p0.8087 + NOROW/NOCOL appliqués. `cargo test` vert. **M10 TERMINÉ.**

## M11 — macro complet (promu ON par défaut)
Architecture (D3 + design détaillé, voir PLAN.md §Macro M11) : expansion **texte→texte
interfoliée** pilotée par l'exécuteur, état (`MacroEngine`) dans `Session` ; nouveau module
`src/macros/` (promu depuis `preprocess.rs`). Un segment brut est expansé PUIS lexé/parsé/exécuté ;
`CALL SYMPUT` écrit la table après l'étape, vu par le segment suivant.

- [x] M11.1 — `MacroEngine` dans `Session` (cfg-split : identité pure sous build défaut ; `%let`/`&var` sous `--features macros`, logique du spike déplacée verbatim). Expansion pilotée par `executor::run_program` (plus par `lib.rs`) ; pour cette unité, source ENTIER expansé une fois (identité → `src` inchangé → lexing/écho byte-identiques). `RawSegmenter`/interfoliage par bloc DÉFÉRÉ à M11.5. Gates : défaut 895 + snapshot (0 `.snap.new`), `--features macros` 903, 0 warning sur les deux. Flag conservé
- [x] M11.2 — `%macro name(params)/%mend` + invocation `%name(args)` (feature `macros`) : table `macros` + pile de `scopes` ; paramètres positionnels + mots-clés (défauts, args résolus dans le scope appelant) ; `&name` cherche scopes→global ; `%let` met à jour la 1ʳᵉ portée existante sinon crée global (`%local` confine, sinon fuite globale = SAS) ; `%local`/`%global` ; corps capturé verbatim (%mend équilibré) ; garde de récursion 100 (pas de panic) ; %if/%do/%eval/%sysfunc différés. Gates : défaut 895 (0 `.snap.new`), feature 918 (+15), 0 warning. (`-D warnings` OK)
- [x] M11.3 — `%if/%then/%else` + `%do/%end` + `%do i=a %to b [%by k]` itératif (feature `macros`) : branche prise expansée (l'autre vide), `&i` posé par itération, garde 1e6 itérations + step=0 → erreur propre (pas de hang/panic). `%do %while/%until` différé (note). Câblé sur `macro_eval`
- [x] M11.4 — `%eval` (feature `macros`) : évaluateur descente récursive, entiers (`/` tronque vers 0, `**` droite-assoc, wrapping anti-panic), comparaisons (`= eq ^= ne <> < lt …`), logique (`& and | or ^ not`), parenthèses ; opérande non entier → erreur SAS ; fonction `%eval(...)` + évaluation implicite dans `%if`/`%to`/`%by`. Gates : défaut 895 (0 `.snap.new`), feature 945 (+27), 0 warning
- [x] M11.5 — `CALL SYMPUT`/`SYMGET` + interfoliage par segment (`RawSegmenter` feature-gated : découpe aux frontières `run;`/`quit;`, ignore `;` en chaîne/`%macro`). `DsStmt::CallRoutine` (parsé 2 builds) ; `EvalCtx.symput_writes` drainé APRÈS l'étape → `macro_engine.set_symbol_global` (invisible dans la même étape = SAS) ; `SYMGET` lit `EvalCtx.macro_symbols` (instantané début d'étape) ; format numérique BEST12. **Build défaut cfg-split = identité M11.1 → byte-identique** ; per-segment + drain compilés sous `macros` seulement. Gates : défaut 896 (0 `.snap.new`), feature 953 (+8), 0 warning
- [x] M11.6 — `%sysfunc(fn(args))` (liste blanche → `functions::call`) ; vars auto `&SYSDATE9`/`&SYSTIME`/`&SYSDAY`/`&SYSVER` (FIGÉES sous `--deterministic` : 01JAN1960/00:00/Friday/9.4) ; quoting `%str`/`%nrstr` (sentinelles) ; indirection `&&&`/`&&var&i`. Tout sous feature `macros`. Gates : défaut 896 (0 `.snap.new`), feature 967 (+14), 0 warning. (l'agent a fini le code mais n'a pu lancer son gate — disque plein ; validé par l'orchestrateur après `cargo clean`)
- [x] M11.7 — feature `macros` RETIRÉE : processeur macro TOUJOURS actif. `IdentityMacroStage` + le `MacroEngine` identité (zero-sized) + la branche `run_program` source-entier supprimés ; seul le chemin per-segment subsiste. **Gate critique vert : `cargo test -p sas_interpreter` (sans `--features`) = 967 tests + snapshot, ZÉRO `.snap.new` (octet-identique), 0 warning.** Aucun fixture macro-free n'a divergé (fast-path identité : segment sans `%`/`&` → tranche inchangée ; n° de ligne via `LogWriter.src_line` partagé)
- [x] Fixtures `tests/fixtures/m11/` (macro_loop, eval_if, symput, sysfunc_autovars) + 4 snapshots **vérifiés à la main** : `%macro`+`%do` génère x=1,2,3 ; `%eval(7/2)`=3 + `%if` → big=7 ; `CALL SYMPUT`→étape suivante v=42 ; `%sysfunc(upcase)`=SAS + `&sysver`=9.4 + `&sysdate9`=01JAN1960. (NB : commentaires sans `%`/`&` — le processeur les scanne, simplification documentée.) `cargo test` vert (967 + snapshots), flag retiré. **M11 TERMINÉ.**

## M12 — extensions macro (processeur ON par défaut)
Différés de M11, dans `src/preprocess.rs` (macros toujours actif). Invariant : snapshots
m1–m11 octet-identiques (le nouveau comportement ne s'active que sur les nouvelles directives).

- [x] M12.1 — `%do %while(cond)` / `%do %until(cond)` : boucles conditionnelles, cond résolue (`&refs`) puis `macro_eval` à chaque tour. `%while` teste AVANT (0 itération possible), `%until` APRÈS (≥1). Garde `MAX_LOOP_ITERS` réutilisée (pas de hang/panic). +5 tests (972 total, 0 `.snap.new`, 0 warning).
- [ ] M12.2 — quoting : `%str`/`%nrstr` existent (M11.6) ; ajouter `%bquote`/`%nrbquote` (masquage à l'exécution, gère parenthèses/quotes non appariées), `%superq(nom)` (valeur du symbole SANS résoudre les `&`/`%` qu'elle contient), et variantes masquées `%qsysfunc`/`%qscan`/`%qsubstr`/`%qupcase`/`%qlowcase`. `%qscan`/etc. = version `%q*` des fonctions existantes. Différés documentés : `%unquote` partiel, `%qcmpres`.
- [ ] Fixtures `tests/fixtures/m12/` + snapshots (vérifiés main) ; DoD : cargo test vert.

## M13 — branchement S3 réel
Le stub `S3Library` (feature `s3`, M8) compile mais n'est pas branché. Objectif : LIBNAME
sur URI `s3://` lit/scanne réellement des parquet via Polars object-store.

- [ ] M13.1 — activer les features Polars `cloud`/`aws` derrière `s3` ; implémenter `S3Library` (scan/read parquet `s3://bucket/key` via `LazyFrame::scan_parquet` object-store ; `ScanArgsParquet`/`CloudOptions`), brancher dans `LibraryManager`/parsing LIBNAME (`libname x 's3://...';` → provider S3). Credentials via env AWS standard. Mutations (write/rename/delete) : best-effort ou erreur claire documentée.
- [ ] M13.2 — tests : parsing d'URI `s3://`, sélection du provider, chemins clé→table ; I/O réelle gardée (skip si pas de credentials / pas de réseau) ou contre un mock. Fixtures non applicables (réseau) — tests unitaires + doc. DoD : `cargo build --features s3` + `cargo test` verts.
