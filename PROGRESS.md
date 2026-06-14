# Avancement `sas_interpreter` — curseur de la skill sasrs-impl

Ce fichier est l'état d'avancement machine-lisible du projet. La skill
`sasrs-impl` le lit pour savoir où reprendre, et le met à jour DANS LE MÊME
COMMIT que le code livré. Ne cocher une case que si : implémentation
complète (zéro `todo!()` restant dans le fichier), tests du fichier écrits,
`cargo test -p sas_interpreter` vert.

Jalon courant : **M16** (constructions étape DATA). M1–M15 terminés. Roadmap M14–M30 ouverte
(couverture SAS quasi-intégrale : I/O fichiers plats, bibliothèque de fonctions, hash,
compléments SQL/macro/formats, complétion des procs, ODS, modélisation statistique,
graphiques). Décisions verrouillées : graphiques en images PNG/SVG via `plotters` ;
dépendances mixtes (crates pour l'I/O lourd, numérique stat **fait maison** dans `src/stat/`) ;
ODS HTML/RTF/PDF/Excel + ODS OUTPUT→datasets.

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
- [x] M12.2 — quoting : fonctions macro `%upcase`/`%lowcase`/`%substr`/`%scan`/`%index`/`%length` (ajoutées, texte brut) ; `%superq(nom)` (valeur masquée, `&`/`%` non résolus) ; `%bquote`/`%nrbquote` (résolvent puis masquent ; nrbquote masque aussi `&`/`%` ; gère quotes/parens non appariées) ; variantes masquées `%qsysfunc`/`%qupcase`/`%qlowcase`/`%qsubstr`/`%qscan`. Réutilise le schéma de sentinelles de `%str`. Différés : `%unquote`/`%cmpres`/`%sysevalf`/`%symexist`. (Note : `%length("")`→0 vs SAS 1, documenté.) +17 tests (989 total, 0 `.snap.new`, 0 warning).
- [x] Fixtures `tests/fixtures/m12/` (do_while, quoting) + snapshots **vérifiés main** : `%do %while` génère x=1..4 ; `%qupcase`=ABCDEF, `%substr`=bcd, `%scan`=b, `%length`=6. `cargo test` vert (989 + snapshots). **M12 TERMINÉ.**

## M13 — branchement S3 réel
Le stub `S3Library` (feature `s3`, M8) compile mais n'est pas branché. Objectif : LIBNAME
sur URI `s3://` lit/scanne réellement des parquet via Polars object-store.

- [x] M13.1 — feature `s3 = ["polars/cloud","polars/aws"]` ; `S3Library::from_uri` (parse `s3://bucket/prefix`), `read()` = scan parquet cloud via `ScanArgsParquet`+`CloudOptions` (credentials env AWS) ; LIBNAME `s3://...` routé vers `S3Library` (cfg-gated : sous build défaut, `s3://` reste un chemin local — inchangé) ; trait `LibraryProvider::is_cloud()` ; mutations → erreur claire. Crates cloud/aws récupérées et compilées dans cet env.
- [x] M13.2 — tests (`cfg(feature=s3)`) : parsing URI `s3://`, sélection du provider (cloud vs DirLibrary), chemins clé→table. I/O réelle non testée (besoin creds+réseau, documenté). DoD : défaut 989 (0 warning, octet-identique) ; `--features s3` build + 999 tests (+10) verts, 0 warning. (NB disque : impossible de garder simultanément les artefacts des 2 variantes — validées séparément après `cargo clean`.) **M13 TERMINÉ.**

# Roadmap M14–M30 — couverture SAS quasi-intégrale

Objectif : parité fonctionnelle large avec SAS 9.4 Base + STAT + graphiques statiques.
Invariant transverse : snapshots m1–m13 **octet-identiques** (le nouveau comportement ne
s'active que sur les nouvelles directives), `sas_cmp`/`nullify_specials` partout, fidélité SAS
vérifiée à la main contre un oracle réel. Ordre d'exécution recommandé : M14 → M15 → M18 →
M16 → M17 → M19 → M20 → M21 → M22 → M24 → M25 → M26 → M23 → M27 → M29 → M28 → M30.

## PHASE A — combler le périmètre existant

## M14 — I/O fichiers plats
Le plus gros déblocage : aujourd'hui tout entre/sort en parquet, impossible de lire un CSV/texte.
- [x] M14.1 — `INFILE` + `INPUT` (list/column/formatted) + `DATALINES`/`CARDS` : capture verbatim DATALINES/CARDS/`4` au lexer (`TokenKind::DataLines`, terminateur `;`/`;;;;`), tokens `@`/`:` ; `DsStmt::{Infile,Input,Datalines}` + structs `InfileSource`/`InfileOptions`/`InputItem` ; `TextInput`/`InputAction` compilés parallèles à `InputData` (SET), garde SET+INFILE exclusif ; `exec_input` pilote la boucle implicite comme SET (EOF mid-itération → EndStep). Modes liste/colonne/formaté, `$`, `:`-modifier, `@n`/`+n`/`/`, hold `@`/`@@`, DSD (champs vides→missing, guillemets), DELIMITER=/DLM=, FIRSTOBS=/OBS=, MISSOVER/TRUNCOVER/STOPOVER, LRECL= (no-op), informat décimal implicite réutilisé via `FormatCatalog::informat`, troncature char au PDV. +34 tests (1023 total, 0 warning, 0 `.snap.new`). Différés (erreurs propres) : SET+INFILE mixte, fichier illisible, STOPOVER, informat invalide. **À VÉRIFIER au DoD M14** : la NOTE "N records were read from the infile DATALINES." — SAS l'émet pour un fichier externe (+ min/max record length) mais probablement PAS pour DATALINES inline ; confronter au comportement réel à la génération du snapshot `datalines.sas`
- [x] M14.2 — `FILE` + `PUT` (sortie texte ; `@`/`@@` hold, `/`) : `DsStmt::{File,Put}` + `enum PutDest` / `enum PutItem` (miroir de sortie d'INPUT) ; parseur `parse_file`/`parse_put` (forme nommée `name=`, formatée `x 8.2`, littérale, `_all_`, pointeurs `@n`/`+n`/`/`, hold `@`/`@@`) ; `PutState` dans le Runner (destination courante, ligne tampon `String`, curseur colonne, drapeaux de hold) initialisé à **LOG** (défaut SAS) ; `exec_file` bascule la destination (flush de la ligne pendante non maintenue au changement), `exec_put` rend les items dans le tampon, relâche en fin de PUT sauf hold (le `@` simple est relâché au début de l'itération suivante, le `@@` survit ; flush du held en fin d'étape). Sorties bufferisées dans le Runner puis rejouées APRÈS la boucle (exec.rs n'a pas `&mut session` en boucle) vers `LogWriter::put_line` (verbatim, sans préfixe NOTE) / `ListingWriter::write_line` (FILE PRINT) / fichiers externes (création+troncature, regroupés par chemin). Formatage réutilise `FormatCatalog::format` + `format_best` (pas de réimplémentation). Variable PUT inconnue → erreur runtime. +21 tests (1044 total, 0 warning sous `-D warnings`, 0 `.snap.new` ; m1–m13 + m14.1 octet-identiques). Différé : aucun (`_all_` implémenté ; `_WEBOUT` hors périmètre M14.2).
- [x] ⫽ M14.3 — `PROC IMPORT`/`PROC EXPORT` : CSV/TAB/DLM via Polars `CsvReadOptions`/`CsvWriter` (feature `csv` ajoutée à polars) ; `src/procs/{import,export}.rs` (`ImportAst`/`ExportAst` + `ImportDbms`/`ExportDbms` Csv/Tab/Dlm, `parse`, `execute`), variantes `ProcAst::{Import,Export}` + bras parse/execute. IMPORT : `DATAFILE=`/`OUT=`/`DBMS=`/`REPLACE`, sous-statements `GETNAMES=`/`DELIMITER=`/`DLM=`/`GUESSINGROWS=` ; lecture → `SasDataset::from_dataframe` (coercition types SAS, WARNING i64>2^53, notes `forward`), écriture biblio, NOTE "has N observations and M variables." + `_LAST_`. EXPORT : `DATA=`(ou `_LAST_`)/`OUTFILE=`/`DBMS=`/`DELIMITER=`, `CsvWriter` (header), NOTE "N records were written to the file '...'.". Excel (`DBMS=XLSX`/`EXCEL`/`XLS`) différé → erreur propre (calamine/rust_xlsxwriter indisponibles dans cet env). +39 tests (1083 total, 0 warning sous `-D warnings`, 0 `.snap.new` ; m1–m14.2 octet-identiques). Divergences documentées : REPLACE toujours appliqué (écrase), GUESSINGROWS ignoré (Polars infère), GETNAMES=NO → VAR1..VARN, mnémoniques délimiteurs (TAB/SPACE/COMMA/PIPE/SEMICOLON + nnX hex). À VÉRIFIER au DoD : sérialisation CSV des missings spéciaux (NaN payload → "NaN" Polars vs "." SAS)
- [x] M14.4 — `LIBNAME ... CSV`/`XLSX` bibliothèque virtuelle (optionnel) : impl `LibraryProvider` fichier-table (Sonnet, faible). `engine: Option<String>` ajouté à `GlobalStmt::Libname` (AST) ; `parse_libname` consomme l'identifiant optionnel entre libref et path (ident non-`clear` → engine uppercased, string → `engine: None`) ; `CsvLibrary` implémente `LibraryProvider` (lecture/écriture/exists/list/delete/rename via Polars `CsvReadOptions`/`CsvWriter`) ; `LibraryManager::assign_csv` ; `executor::exec_global` bascule sur le moteur : `CSV` → `assign_csv` + NOTE "Engine: CSV", `XLSX/EXCEL/XLS` → ERROR deferral, `_` → parquet inchangé. +16 tests (1103 total, 0 warning `-D warnings`, 0 `.snap.new` ; m1–m14.3 octet-identiques).
- [x] Fixtures `m14/` (read_csv, datalines, put_report, import_export) + snapshots vérifiés à la main. DoD : m1–m13 octet-identiques. datalines : 3 obs list/colonne + MISSOVER ; put_report : PUT LOG pi/hold-@/\_all\_/FILE PRINT ; import_export : **PROC EXPORT puis PROC IMPORT** (round-trip CSV réel, if age≥14 → 9 obs) ; read_csv : DATALINES→LIBNAME CSV→WORK, 4 étudiants. **Corrections orchestrateur au DoD** : (1) fidélité — la NOTE "N records were read from the infile" n'est plus émise pour les données instream DATALINES/CARDS (réservée aux fichiers externes, comme SAS) ; champ `TextInput.is_file` ; (2) `Session::resolve_path` — les chemins relatifs d'INFILE/FILE/PROC IMPORT/EXPORT résolvent désormais sous `base_dir` (cohérent avec LIBNAME, déterministe sous le harnais) ; (3) import_export.sas réécrit pour exercer réellement PROC EXPORT+IMPORT (la version livrée par l'agent les contournait via LIBNAME CSV) ; (4) titres ASCII (le rendu listing affichait du mojibake UTF-8 — **bug latent encodage listing à traiter ultérieurement**). 1103 tests (0 warning -D warnings), 0 .snap.new. **M14 TERMINÉ.**

## M15 — bibliothèque de fonctions (~44 → ~150)
Table-driven (`DISPATCH` dans `functions.rs`), numérique maison. Un lot ⫽ par famille.
- [x] ⫽ M15.1 — caractère : FIND, FINDC, COUNT, COUNTC, VERIFY, TRANSLATE, REVERSE, REPEAT, PROPCASE, COMPBL, SUBSTRN, CHAR, RANK, BYTE, WHICHC, CATQ (Sonnet, moyen)
- [x] ⫽ M15.2 — mathématiques : CEIL, FLOOR, SIGN, SIN/COS/TAN/ARSIN/ARCOS/ATAN/ATAN2, SINH/COSH/TANH, FACT, COMB, PERM, GAMMA, LGAMMA, DIGAMMA, BETA, ROUNDZ, RANGE, LARGEST/SMALLEST, ORDINAL (Sonnet, moyen)
- [x] ⫽ M15.3 — date/heure : DATEPART, TIMEPART, DATETIME, HMS, DHMS, YRDIF, DATDIF, JULDATE, DATEJUL, HOUR/MINUTE/SECOND, NLDATE, INTFMT, INTSHIFT (Opus, moyen)
- [x] M15.4 — probabilités : PROBNORM, PROBT, PROBF, PROBCHI, PROBBETA, PROBGAM, CDF, PDF, QUANTILE, SDF, LOGCDF, PROBBNML, POISSON (Fable, élevé — numérique maison, anticipe `src/stat/`)
- [x] M15.5 — aléatoire : RAND, RANUNI, RANNOR, RANEXP, RANBIN, CALL STREAMINIT/RANUNI ; PRNG MT19937 maison, graine figée sous `--deterministic`, fidélité flux SAS documentée comme approximation (Fable, élevé)
- [x] M15.6 — CALL routines : CALL MISSING, CALL EXECUTE (file différée post-step), CALL SORTN/SORTC, CALL SYMPUTX, CALL CATS/SCAN, CALL LABEL, CALL VNAME (Opus, moyen)
- [x] M15.7 — LAG/LAGn/DIF/DIFn : FIFO par site d'appel (clé = identité d'expression), `EvalCtx.lag_fifos` (vérifier l'état réel d'abord) (Opus, moyen)
- [x] Fixtures `m15/` + snapshots. DoD

## M16 — constructions étape DATA
- [x] M16.1 — `SELECT`/`WHEN`/`OTHERWISE` (formes sélecteur et booléen), `DsStmt::Select` (Opus, moyen) : AST `DsStmt::Select { selector, whens: Vec<WhenClause>, otherwise }` + `WhenClause { values, body }`. Forme SÉLECTEUR `select (expr);` — expr évaluée UNE fois (`exec_select`), chaque `when (v1, v2, ...)` compare via la sémantique `=` de SAS (`eval::sas_values_equal` = `normalize_comparison` + `sas_cmp`, donc `. = .` vrai, char insensible aux blancs finaux, conversion char↔num notée). Forme BOOLÉENNE `select;` — chaque `when (cond)` est une condition booléenne (`truthy()`), liste de valeurs multiple rejetée au parsing. Corps = UN statement (`do; ... end;` pour plusieurs) ; corps vide `when (1) ;` licite (no-op, pas de fall-through). PREMIÈRE clause vraie exécute son corps puis le SELECT se termine (no fall-through) ; sans match ni OTHERWISE → erreur runtime fidèle SAS. Compilation : `walk_stmt` vérifie les références (sélecteur, valeurs/conditions, corps WHEN+OTHERWISE). Parser : `parse_select`/`parse_when_values`/`parse_select_branch` ; END manquant, WHEN après OTHERWISE, OTHERWISE multiple, liste WHEN vide → erreurs propres. +22 tests (16 exec + 6 parser, 1414 total, 0 warning `-D warnings`, snapshot inchangé). Différé (M16.3) : `do x = liste;` ; comparaison chaînée `1<=x<=10` (limite parser expr, contournée par `and`).
- [x] M16.2 — tableaux multi-dimensionnels + valeurs initiales + `_TEMPORARY_`/`_NUMERIC_`/`_CHARACTER_`/`_ALL_` + DIM/HBOUND/LBOUND, index linéaire row-major (Fable, élevé) : AST — `Expr::Index { name, indices: Vec<Expr> }` (mono- ou multi-indices), `DsStmt::Array { dims: Option<Vec<usize>>, char_len, vars, initial: Vec<Expr>, temporary, special: Option<ArraySpecial> }`, `DsStmt::AssignIndexed { array, indices: Vec<Expr>, expr }`, nouvel enum `ArraySpecial{Numeric,Character,All}`. Parser — `parse_index` accepte des indices séparés par virgules ; `parse_array` lit `{n}`/`{n,m,...}`/`{*}` (`*` réservé au 1-D), `$ len`, `_TEMPORARY_`, listes spéciales (exclusives des noms), valeurs initiales `(…)` (virgules OU espaces). Compilation — `ArrayDef { slots, dims }` (borne inf = 1, produit dims = nb slots), `ArrayDef::linear_index` (row-major ; index unique sur multi-dim = accès linéaire) ; `_TEMPORARY_` → `PdvVar.temporary` (hors sortie + retenu implicite, exclu de PUT `_all_`/KEEP-DROP/uninitialized) ; `_NUMERIC_`/`_CHARACTER_`/`_ALL_` → variables PDV du type voulu connues au point du statement ; valeurs initiales constantes évaluées à la compilation (`const_eval_initial`, coercition vers le type de l'array) → `initial_values` + slots retenus. Éval — `eval_array_ref`/`resolve_subscript` multi-indices via `linear_index` (missing/hors-bornes/nb d'indices invalide → ERROR "Array subscript out of range." qui stoppe l'étape) ; `DIM`/`HBOUND`(borne sup de la dim n, défaut 1)/`LBOUND`(=1) interceptés avant la délégation de fonction, dimension invalide → ERROR. fastpath déjà gardé sur `!arrays.is_empty()`. +21 tests (14 exec M16.2 + parser maj), 1432 total, 0 warning `-D warnings`, 0 `.snap.new` (m1–m15 octet-identiques). Différés : bornes inférieures explicites `{0:5}` (LBOUND figé à 1, conforme à la consigne), `DO OVER` (M16.3).
- [ ] M16.3 — `DO` sur liste de valeurs (gardes `:579,622`), `DO OVER`, littéraux date en RETAIN (`:999`) (Opus, moyen)
- [ ] M16.4 — SET multiples (garde `datastep/mod.rs:334`), `SET ... POINT=`/`NOBS=` (accès direct), `SET ... END=` (Fable, élevé)
- [ ] M16.5 — `UPDATE` (master/transaction), `MODIFY` (réécriture en place) (Fable, élevé)
- [ ] M16.6 — `LINK`/`RETURN`, labels/`GOTO`, `RETAIN _ALL_` (Opus, moyen)
- [ ] Fixtures `m16/` + snapshots. DoD

## M17 — objets hash
- [ ] M17.1 — `DECLARE HASH h(...)` + `defineKey/defineData/defineDone` ; `HashObject{keys,data_vars,rows}` dans le Runner, dispatch méthodes via `EvalCtx` (Fable, élevé)
- [ ] M17.2 — méthodes find/check/add/replace/remove/clear/output/num_items ; `DECLARE HITER` + first/next/last/prev ; options ordered:/duplicate:/multidata: (Fable, élevé)
- [ ] Fixtures `m17/` + snapshots. DoD

## M18 — formats & informats
- [ ] ⫽ M18.1 — étoffer `format_builtin`/`informat_builtin` : COMMAX, DOLLARX, EURO, NEGPAREN, HEX, BINARY, OCTAL, ROMAN, WORDS, FRACT, SCIENTIFIC, dates (WEEKDATE, DOWNAME, MONNAME, QTR, YYQ, JULIAN, B8601*/E8601* ISO), $QUOTE, $HEX, $UPCASE (Sonnet, moyen-élevé)
- [ ] M18.2 — INVALUE (informats utilisateur) : lever `procs/format.rs:78`, `FormatCatalog.user_informats`, lookup avant builtin (Sonnet, moyen)
- [ ] M18.3 — PICTURE : `enum FormatKind{Value,Picture,Invalue}` dans `userdef.rs`, templates `99/99/9999`, directives PREFIX/MULT/FILL (Opus, moyen)
- [ ] Fixtures `m18/` + snapshots. DoD

## M19 — macro : fonctions différées
- [ ] M19.1 — `%unquote`, `%cmpres`/`%qcmpres`, `%symexist`, `%sysmexist`, `%sysget`, `%sysevalf` (éval flottante) ; triggers + `consume_*` dans `preprocess.rs` (Fable, moyen-élevé)
- [ ] M19.2 — `%include` + bibliothèques autocall (`SASAUTOS`, recherche + compilation paresseuse) (Opus, moyen)
- [ ] M19.3 — options de trace `MPRINT`/`MLOGIC`/`SYMBOLGEN` (écho conditionnel au log) ; `CALL EXECUTE` côté macro ; `%put` avancé (Opus, moyen)
- [ ] Fixtures `m19/` + snapshots. DoD

## M20 — PROC SQL : compléments
- [ ] M20.1 — `LIKE` complet (regex) ; `EXCEPT/INTERSECT ALL` exacts (Opus, moyen)
- [ ] M20.2 — sous-requêtes corrélées/non corrélées (WHERE/HAVING/SELECT), `EXISTS` (Fable, élevé)
- [ ] M20.3 — dictionary tables (`DICTIONARY.TABLES/COLUMNS/MACROS`, vues `sashelp.v*`) alimentées par l'état de session (Opus, moyen)
- [ ] M20.4 — vues SQL (`CREATE VIEW`), `UPDATE ... SET`, sous-requêtes dans `INSERT` (Opus, moyen)
- [ ] Fixtures `m20/` + snapshots. DoD

## M21 — complétion des procs existants + procs utilitaires
- [ ] ⫽ M21.1 — `PROC COMPARE`, `PROC PRINTTO`, `PROC OPTIONS`, `PROC CATALOG` (Sonnet, moyen)
- [ ] M21.2 — FREQ : Fisher exact, AGREE (kappa), MEASURES (odds ratio, RR), TREND (Cochran-Armitage), CHISQ 1 voie, `TABLES ... / OUT=` (Opus, élevé)
- [ ] M21.3 — UNIVARIATE : tests de normalité (Shapiro-Wilk, Kolmogorov, Anderson-Darling), HISTOGRAM/QQPLOT (→ images M29), percentiles étendus (Fable, élevé)
- [ ] M21.4 — TABULATE/REPORT avancés : 3ᵉ dim, croisements multi-VAR/stats, ALL, PCTN/PCTSUM (TABULATE) ; ACROSS, COMPUTE, BREAK/RBREAK, LINE, WHERE, OUT= (REPORT) (Opus, élevé)
- [ ] M21.5 — RANK méthodes FRACTION/PERCENT/NORMAL/SAVAGE + BY ; CORR Spearman/Kendall + OUT=/OUTP= + WEIGHT (Opus, moyen-élevé)
- [ ] Fixtures `m21/` + snapshots. DoD

## PHASE B — ODS (prérequis capture + graphiques)

## M22 — couche de routage ODS + capture
- [ ] M22.1 — trait `OutputDestination { page_header; write_table; write_line; blank }` dans `src/output/` ; `TextListing` = comportement actuel, `Session.listing: Box<dyn OutputDestination>` + registre multi-destinations. Invariant : listing texte par défaut byte-identique (Fable, élevé)
- [ ] M22.2 — parseur du statement `ODS` (ouvrir/fermer destinations) + options globales NOCENTER/DATE/NUMBER (Opus, moyen)
- [ ] M22.3 — ODS OUTPUT → datasets (mapping nom-de-table ODS → `OUT=`) ; utile pour tester les procs stat à venir (Opus, moyen-élevé)
- [ ] M22.4 — destination HTML (tables CSS, fichier `.html`) (Sonnet, moyen)
- [ ] Fixtures `m22/` + snapshots (texte inchangé ; HTML/dataset capturés vérifiés). DoD

## M23 — ODS RTF / PDF / Excel
- [ ] ⫽ M23.1 — RTF (séquences de contrôle Word) (Opus, moyen)
- [ ] ⫽ M23.2 — PDF via `printpdf` (pagination, tables), feature `pdf` (Opus, moyen-élevé)
- [ ] ⫽ M23.3 — Excel via `rust_xlsxwriter` (`ODS EXCEL`, feuilles par proc, styles de base) (Sonnet, moyen)
- [ ] Fixtures `m23/` + snapshots (assertion structure/existence des fichiers). DoD

## PHASE C — modélisation statistique

## M24 — fondation numérique stat + procs de base
- [ ] M24.1 — module `src/stat/` (maison) : promotion des helpers `common.rs` (Beta/Gamma/t/χ²) + normale (CDF/quantile), F, gamma, bêta ; `linalg.rs` (Cholesky, QR/Householder, moindres carrés, inversion, valeurs/vecteurs propres symétriques par Jacobi), tout testé contre valeurs documentées (Fable, élevé)
- [ ] M24.2 — `PROC TTEST` (1 échantillon, 2 échantillons groupés/appariés, Satterthwaite, CLASS, PAIRED) (Opus, élevé)
- [ ] M24.3 — `PROC NPAR1WAY` (Wilcoxon, Kruskal-Wallis, scores) (Opus, moyen-élevé)
- [ ] Fixtures `m24/` + snapshots vérifiés contre SAS. DoD

## M25 — modèle linéaire
- [ ] M25.1 — `PROC REG` (OLS via QR, MODEL, R²/F/t, OUTPUT OUT= résidus/prédits, TEST, intervalles) (Fable, élevé)
- [ ] M25.2 — `PROC ANOVA` (plans équilibrés, CLASS, MEANS, types de SC) (Opus, élevé)
- [ ] M25.3 — `PROC GLM` (codage CLASS, SC type I/III, LSMEANS, contrastes, ESTIMATE) (Fable, élevé)
- [ ] Fixtures `m25/` + snapshots. DoD

## M26 — modèles catégoriels
- [ ] M26.1 — `PROC LOGISTIC` (logistique binaire, Newton-Raphson/IRLS, odds ratios, CLASS, LINK=) (Fable, élevé)
- [ ] M26.2 — `PROC GENMOD` (GLM exponentiels : Poisson, binomial, gamma ; fonctions de lien ; DIST=) (Fable, élevé)
- [ ] Fixtures `m26/` + snapshots. DoD

## M27 — multivarié
- [ ] M27.1 — `PROC PRINCOMP` (ACP via valeurs propres covariance/corrélation) (Opus, élevé)
- [ ] M27.2 — `PROC FACTOR` (extraction, rotation VARIMAX) (Fable, élevé)
- [ ] M27.3 — `PROC CLUSTER` + `PROC FASTCLUS` (k-means), `PROC DISTANCE` (Opus, élevé)
- [ ] M27.4 — `PROC DISCRIM` (analyse discriminante linéaire) (Opus, élevé)
- [ ] Fixtures `m27/` + snapshots. DoD

## M28 — modèles mixtes (le plus difficile, en dernier)
- [ ] M28.1 — `PROC MIXED` (effets fixes + aléatoires, REML/ML itératif, structures VC/CS/AR(1)/UN, SOLUTION, LSMEANS) (Fable, très élevé)
- [ ] M28.2 — `PROC GLIMMIX` (modèles mixtes généralisés, pseudo-vraisemblance) (Fable, très élevé)
- [ ] Fixtures `m28/` + snapshots. DoD

## PHASE D — graphiques (images via `plotters`, dépend de M22)

## M29 — ODS GRAPHICS + PROC SGPLOT
- [ ] M29.1 — infra ODS GRAPHICS : destination image vers `plotters` (PNG + SVG), nommage/dimensions/DPI ; snapshots = log/listing + assertion existence + (format, largeur×hauteur), feature `graphics` (Fable, élevé)
- [ ] M29.2 — `PROC SGPLOT` : SCATTER, SERIES, VBAR/HBAR, HISTOGRAM, DENSITY, VBOX, REG/LOESS, XAXIS/YAXIS, BY (Opus, élevé)
- [ ] M29.3 — branchement des plots UNIVARIATE (HISTOGRAM/QQPLOT) et REG (diagnostics) sur l'infra image (Opus, moyen)
- [ ] Fixtures `m29/` + snapshots. DoD

## M30 — graphiques legacy
- [ ] ⫽ M30.1 — `PROC GPLOT` (+ SYMBOL/AXIS), `PROC GCHART` (VBAR/HBAR/PIE) (Opus, moyen-élevé)
- [ ] ⫽ M30.2 — `PROC PLOT` (rendu image cohérent avec ODS GRAPHICS) (Sonnet, moyen)
- [ ] Fixtures `m30/` + snapshots. DoD → **couverture cible atteinte**
