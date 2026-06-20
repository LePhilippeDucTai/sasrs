# Avancement `sasrs` — curseur de la skill sasrs-impl

Ce fichier est l'état d'avancement machine-lisible du projet. La skill
`sasrs-impl` le lit pour savoir où reprendre, et le met à jour DANS LE MÊME
COMMIT que le code livré. Ne cocher une case que si : implémentation
complète (zéro `todo!()` restant dans le fichier), tests du fichier écrits,
`cargo test -p sasrs` vert.

Jalon courant : **M34**. M1–M33 terminés (roadmap d'origine : couverture SAS
quasi-intégrale — I/O fichiers plats, bibliothèque de fonctions, hash, compléments
SQL/macro/formats, complétion des procs, ODS, modélisation statistique, graphiques).
Décisions verrouillées : graphiques en images PNG/SVG via `plotters` ; dépendances mixtes
(crates pour l'I/O lourd, numérique stat **fait maison** dans `src/stat/`) ; ODS
HTML/RTF/PDF/Excel + ODS OUTPUT→datasets.

**Phase E (M31–M35)** — qualité & complétion (demandée après M30) : refactorisation en
style fonctionnel + généralisation/réduction de complexité (M31 couche de parsing PROC
partagée, M32 scission `preprocess.rs`→`src/macros/`), puis complétion maximale des options
des procs partiellement supportées (M33 Base/descriptifs, M34 stat/modélisation) et support
total des macros (M35). **Invariant porté par les jalons de refactor (M31/M32) : sortie
octet-identique → zéro `.snap.new` ; commits d'extraction « move-only ».** Les jalons de
complétion (M33–M35) n'activent du comportement que sur des options jusque-là refusées et
font rétrécir la colonne « non couvert » de README en miroir.

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
- [x] M16.3 — `DO` sur liste de valeurs, `DO OVER`, littéraux date en RETAIN (Opus, moyen) : AST — `DsStmt::DoList { index, items: Vec<DoListItem>, body }` (`DoListItem::Value(Expr)` | `Range { from, to, by }`), `DsStmt::DoOver { array, body }`. **DO liste** : `parse_iterative_do` parse le 1er segment (from + TO/BY/WHILE/UNTIL) ; PAS de virgule + ≥1 clause TO/BY/WHILE/UNTIL → `DoLoop` classique (inchangé, seul à porter WHILE/UNTIL) ; sinon (virgule OU valeur nue seule) → `DoList` (items séparés par `,`, chacun valeur explicite ou sous-liste `from to e [by k]`). WHILE/UNTIL en liste → erreur. Exec `exec_do_list` : valeurs explicites évaluées une par une, ranges énumérés comme un DO ; index garde la DERNIÈRE valeur en sortie (≠ règle TO du DoLoop). Type index inféré char ssi une valeur littérale chaîne (`do_list_index_type`). **DO OVER** : `parse_do` détecte `over <ident> ;` (lookahead `peek_nth`, `over` non réservé). Élément courant exposé via `EvalCtx.do_over: HashMap<nom UPPER, slot>` (push/pop par tour, sauvegarde/restauration pour imbrication) ; `eval_var` redirige la référence NUE au nom de l'array vers le slot courant ; `DsStmt::Assign` résout la lvalue nue de même ; l'accès indexé `arr{i}` reste statique. Compilation : `Compiler.do_over_arrays` autorise la référence nue (lecture/écriture, sans créer de variable) dans le corps ; itération sur `def.slots` (déjà row-major M16.2). **RETAIN date/datetime** : `parse_retain_init` accepte `21710d`/`43200t`/`...dt` (num + suffixe adjacent d/t/dt, valeur = nombre SAS) ET la forme quotée `'01JAN2020'd`/`'14:30't`/`'01JAN2020 14:30:00'dt` (délègue à `expr::literal_from_string`, rendu public). Correctif au passage : `parse_datetime_literal` accepte le séparateur ESPACE date/heure (`'… HH:MM:SS'dt`), pas seulement `:` (fidèle SAS). +26 tests (13 exec + 13 parser ; ancien `do_value_list_errors` retourné en `do_value_list_now_parses`), 1458 total, 0 warning `-D warnings`, 0 `.snap.new` (m1–m16.2 octet-identiques). Différés : bornes inférieures explicites d'array (figées à 1, M16.2).
- [x] M16.4 — `SET ... END=`/`NOBS=`/`POINT=` + SET multi-datasets (Fable, élevé) : nouvel AST `SetOptions{end,nobs,point}` porté par `DsStmt::Set{specs,options}` (placées APRÈS la liste de datasets, détectées par lookahead `ident =`). **END=** : variable automatique temporaire 0/1 (JAMAIS de slot PDV ni de colonne de sortie, servie par `EvalCtx.end_flag` comme FIRST./LAST. — interceptée à la compilation par le nom déclaré) ; 1 après lecture de la DERNIÈRE obs RETENUE du DERNIER dataset (lookahead non destructif `concat_has_more` respectant les WHERE=, et `next_keys.is_none()` en interclassement BY). **NOBS=** : slot PDV numérique retenu, affecté AVANT la boucle au total d'obs (somme des datasets) — disponible dès `do i=1 to n;`. **POINT=** : accès direct 1-based (`exec_set_point`) qui REMPLACE la boucle implicite ET l'output implicite (`suppress_implicit_output`) ; index global 1..total sur la concaténation multi-datasets ; missing/non entier/hors bornes [1,total] → ERROR "Error in variable p." (+`_ERROR_`) qui stoppe l'étape ; incompatible BY/MERGE (erreur propre). SET multi-datasets en concaténation déjà couvert (M1/M3) — vérifié (`set_three_datasets_concatenates_in_order`). fastpath désactivé si END=/NOBS=/POINT=. **Plusieurs SET *statements* restent refusés (hors périmètre, garde conservée).** +27 tests (22 exec + 5 parser), 1485 total, 0 warning `-D warnings`, clippy inchangé (122), 0 `.snap.new` (m1–m16.3 octet-identiques). Différés documentés : `set;` nu sans dataset, multiple SET statements.
- [x] M16.5 — `UPDATE` (master/transaction), `MODIFY` (réécriture en place) (Fable, élevé) : AST `DsStmt::Update { master, master_where, transaction, key_vars }` + `DsStmt::Modify { dataset, key_vars, point, nobs }`. Parser `parse_update`/`parse_modify`/`parse_key_option` (KEY= liste de variables ; UPDATE n'accepte que `(where=)` sur le maître ; MODIFY accepte key=/point=/nobs=). Compilation : `materialize_input(_with_meta)` (toutes colonnes au PDV, downcast unique), `build_update` (slots clé/overlay, BY optionnel via `master.by_cols`+FIRST./LAST.), `build_modify` (slots clé/point/nobs + `out_vars` pour réécriture identique) ; nouvelles structs `UpdateData`/`ModifyData` dans `StepProgram`, exclusivité avec SET/MERGE/INFILE, OUTPUT interdit avec MODIFY (erreur de compilation). Exécution : boucles dédiées `execute_update`/`execute_modify` (Runner « squelette » via `build_um_runner`). **UPDATE** : maître pilote l'itération, index transaction par clé (`key_string`, première obs gagnante, `. = .`/char trim), superpose les variables NON MANQUANTES hors clés ; WHERE= maître pré-filtré ; variables transaction-seules blanchies par obs ; FIRST./LAST. sur transitions de clé BY ; transactions sans maître IGNORÉES (v1) ; sortie implicite vers le dataset DATA. **MODIFY** : réécriture EN PLACE (`out_vars`, mêmes colonnes/ordre) ; lecture séquentielle (capture par ligne) OU POINT= (accès direct piloté par un DO, `modify_state` partagé, capture au marqueur suivant + en fin) ; NOBS= pré-boucle ; pas d'output implicite ; sorties DATA coïncidant avec la table ignorées. 26 tests (14 UPDATE dont BY/FIRST.LAST./char/multi-clés/WHERE/missing-skip/multi-trans, 10 MODIFY dont point/nobs/key/char/output-interdit, 2 exclusivité). 1511 tests + snapshot verts, 0 warning `-D warnings`, clippy 121 (≤ baseline), 0 `.snap.new`. Différés documentés : insertion des transactions non appariées (UPDATE), suppression d'obs via MODIFY (`delete;`/`remove`), OUT= explicite, mélange SET+UPDATE/MODIFY.
- [x] M16.6 — `LINK`/`RETURN`, labels/`GOTO`, `RETAIN _ALL_` (Opus, moyen) : AST `DsStmt::{Labeled{name,stmt}, Goto(String), Link(String), Return}`. **Étiquettes** : parsées par détection `ident :` (avant le dispatch par mot-clé, `peek2()==Colon`), corps = un statement (forme vide `lbl: ;` → bloc vide). **GOTO** : `goto lbl;`/`go to lbl;` ; **LINK**/**RETURN** mots-clés dédiés. Compilation : `Compiler.labels_defined` (doublon → erreur) + `goto_link_refs` validées en fin de compile contre `flow_labels` (index des `Labeled` de PREMIER NIVEAU) — étiquette inconnue OU seulement imbriquée dans un bloc DO → erreur (GOTO/LINK ne ciblent que le premier niveau ; sauter DANS un bloc indéfini en SAS, non supporté). **Exécution** : `StepProgram.flow_labels` + compteur de programme `run_step_body()` (remplace les 4 boucles `for stmt in &stmts` : implicite/UPDATE/MODIFY-point/MODIFY-seq) piloté sur `Runner.program`/`flow_labels` (`Rc`). GOTO → repositionne le PC ; RETURN au 1er niveau → fin d'itération normale (output implicite). **LINK exécuté INLINE** par `exec_link_subroutine` (du label au prochain RETURN) au point d'appel — la pile d'appels Rust EST la pile de retour → un LINK dans une boucle DO ne l'abandonne PAS (reprise correcte), imbrication OK ; un GOTO depuis une sous-routine remonte et abandonne le LINK (fidèle SAS). `Flow` étendu de `Goto/Return` (variante `Link` retirée car LINK jamais remonté). **RETAIN _ALL_** : `retain _all_;` retient les variables connues À CE POINT (`0..pdv.len()` au moment du statement — ≠ `retain;` nu qui retient tout en fin de compile) ⇒ les variables créées APRÈS ne sont PAS retenues ; init interdit ; bonus `_NUMERIC_`/`_CHARACTER_`. fastpath déjà exclu (`stmt_ok` → false sur ces statements). 23 tests (LINK/RETURN base, pile imbriquée, GOTO inconditionnel/sortie de boucle/multi-labels/arrière, labels divers, RETURN sans LINK, labels inconnus/dupliqués/imbriqués → erreurs, RETAIN _ALL_ cumul/mix/var-après/rejet-init, LINK en boucle/persistance/multi-entrées, go-to 2 mots, reset par itération). 1534 tests + snapshot verts, 0 warning `-D warnings`, clippy 121 (≤ baseline), 0 `.snap.new` (m1–m16.5 octet-identiques). Différés documentés : GOTO/LINK vers étiquette imbriquée dans un bloc (erreur propre).
- [x] Fixtures `m16/` + snapshots. DoD

## M17 — objets hash
- [x] M17.1 — `DECLARE HASH h(...)` + `defineKey/defineData/defineDone` ; `HashObject{keys,data_vars,rows}` dans le Runner, dispatch méthodes via `EvalCtx` (Fable, élevé) : AST — `DsStmt::DeclareHash { name, options: Vec<(String,String)> }` (options `clé:valeur` colon-séparées, virgule entre ; valeur = littéral chaîne/num normalisé String) et `DsStmt::HashMethod { object, method, args }` (CHOIX : statement dédié plutôt qu'`Expr::MethodCall` — un appel de méthode hash est toujours un statement du DATA step, et le parser dispatch par statement). Parser — `parse_declare` (alias `declare`/`dcl` ; seul type `hash`, `hiter`→erreur M17.2 ; parenthèses optionnelles/vides), `parse_hash_method` (détecté AVANT le dispatch par mot-clé via le motif `ident . ident (` en tête de statement — sans ambiguïté avec `lib.table`, jamais en tête d'un exécutable). Données — `HashObject { ordered, duplicate, multidata, dataset, keys, data_vars, rows: HashMap<String,Vec<Vec<Value>>>, defined }` + `hash_key()` (encodage clé = sémantique d'égalité SAS, `. == .`, char sans blancs finaux — réplique de `key_string` UPDATE/MODIFY) dans `datastep/mod.rs`. Compilation — `Compiler.hash_objects` (DECLARE résout options : ordered/duplicate→minuscules, multidata yes/y/1→bool, dataset/data conservé, option inconnue→erreur ; redéclaration écrase) ; `StepProgram.hash_objects` → `EvalCtx.hashes` au début de l'étape ; `HashMethod` validé (objet déclaré ? defineKey/defineData args = littéraux chaîne nommant une variable EXISTANTE du PDV, sinon erreur). Exécution — `exec_hash_method` : defineKey/defineData posent `keys`/`data_vars` (UPPERCASE, ordre préservé), defineDone pose `defined=true` (IDEMPOTENT), find/add/... → erreur « not yet implemented » (M17.2 ; chargement `dataset:` aussi différé). fastpath exclu si `!hash_objects.is_empty()`. Inspection test via `Session.debug_hashes` (`#[cfg(test)]` only). +19 tests (6 parser + 13 exec : DECLARE avec/sans options, ordered/duplicate/multidata/dataset, multidata:no, option inconnue, defineKey/Data single+multiple, defineDone idempotent, multi-objets indépendants, var inconnue/objet non déclaré/méthode non implémentée → erreurs). 1622 tests + snapshot verts, 0 warning `-D warnings`, 0 `.snap.new` (m1–m16 octet-identiques).
- [x] M17.2 — méthodes find/check/add/replace/remove/clear/output/num_items + find_next/find_prev ; `DECLARE HITER` + first/next/last/prev ; options ordered:/duplicate:/multidata: + chargement dataset: (Fable, élevé) : AST — nouvelle variante `Expr::HashMethod(Box<HashMethodCall>)` (forme EXPRESSION `rc = h.find();`, boxée pour garder `Expr` compact → pas de régression clippy large-variant) + `DsStmt::HashMethod(Box<HashMethodCall>)` (forme STATEMENT, rc ignoré) partageant `HashMethodCall{object,method,args:Vec<HashArg>}` ; `HashArg::{Positional(Expr),Named(name,Expr)}` (arguments nommés `key:`/`data:`/`dataset:`) ; `DsStmt::DeclareHiter{name,hash_name}` ; structs `HashIter{hash,pos}` + `HashOutput`. **Choix expression vs statement** : un appel de méthode hash mute le PDV (copie des données sur find) et les objets hash → il ne peut PAS passer par l'évaluateur immuable `eval(&Pdv)` ; il est donc INTERCEPTÉ en tête de `exec::eval_checked` (qui a `&mut self`), qui appelle `exec_hash_method_rc` et renvoie le code retour numérique. La forme statement appelle la MÊME fonction et ignore le rc. L'évaluateur immuable porte un bras de garde (fatal) jamais atteint en pratique. Parser — `parse_hash_args` (positionnel OU `name:expr`), détection `ident.ident(` en expression (`parse_primary`, avant la règle FIRST./LAST.), attribut sans parenthèses `h.num_items`, `parse_declare_hiter`. Compilation — `validate_hash_method` partagée (objet/itérateur déclaré ? defineKey/defineData créent la variable au PDV si absente, fidèle SAS) ; `preload_hash_dataset` lit `dataset:` à la compilation (`&mut Session`) et entre les colonnes au PDV ; `Compiler.hash_iters` → `StepProgram` → `EvalCtx`. Exécution — `exec_hash_method_rc` dispatch add/replace/find/check/remove/clear/find_next/find_prev/num_items/output ; clé/donnée via `hash_key_values`/`hash_data_values` (PDV ou args nommés) ; `hash_insert` honore multidata (push) et duplicate:'replace'/'error'/défaut (rc≠0, 1re valeur gardée) ; `find` copie les données + positionne le curseur multidata, `check` ne copie pas ; `defineDone` charge le `dataset:` pré-lu ; `output` met une sortie en file (drainée APRÈS la boucle par `flush_hash_outputs` via le provider) ; `key_values` conserve les `Value` clé décodées (la clé encodée perd le type) pour output + tri `ordered:` (sas_cmp ascending/descending). HITER — `exec_hiter_method` first/last/next/prev sur l'ordre de visite APLATI (ordered: → sas_cmp, sinon insertion ; multidata développé), copie clé+données au PDV, rc≠0 hors bornes. nullify_specials/sas_cmp respectés ; aucun get_row (décodage via `num_to_value`/`Value`). +27 tests (4 parser + 23 exec : add/find round-trip, find copie/check non, miss rc≠0, replace, remove+num_items, clear, statement vs expression, output + ordered asc/desc, dataset: load, multidata find_next, duplicate replace/défaut, HITER first/next & last/prev & ordered desc, hiter hash inconnu, args nommés, clés/données char) — 1645 tests + snapshot verts, 0 warning `-D warnings`, clippy 128 (= baseline), 0 `.snap.new`.
- [x] Fixtures `m17/` + snapshots. DoD : `m17/hash.sas` **vérifié à la main vs d.class** — lookup `dataset:'d.class'` (Mary→age 15, Nobody→rc 1/missing) ; build manuel scores + replace(id2→99)/remove(id3)/num_items=2 + output ordered ascending (id 1=10, 2=99) ; HITER descending k=3/2/1 (v=300/200/100). m1–m16 octet-identiques. `cargo test` vert. **M17 TERMINÉ.**

## M18 — formats & informats
- [x] ⫽ M18.1 — étoffer `format_builtin`/`informat_builtin` : COMMAX, DOLLARX, EURO, NEGPAREN, HEX, BINARY, OCTAL, ROMAN, WORDS, FRACT, SCIENTIFIC, dates (WEEKDATE, DOWNAME, MONNAME, QTR, YYQ, JULIAN, B8601*/E8601* ISO), $QUOTE, $HEX, $UPCASE (Sonnet, moyen-élevé). 69 nouveaux tests (1603 total, 0 warning). Différés : $UPCASE comme informat, informat pour HEX/BINARY/OCTAL/ROMAN, FRACT avec dénominateur exact > 64.
- [x] M18.2 — INVALUE (informats utilisateur) : lever `procs/format.rs:78`, `FormatCatalog.user_informats`, lookup avant builtin (Sonnet, moyen). `UserInformat` dans `userdef.rs` (plages de chaînes → `Value` num ou char ; `_SAME_`, `other=`, bornes LOW/HIGH, exclusives) ; `FormatCatalog::define_informat` + résolution AVANT les builtins ; `fn_input`/`fn_put` corrigés pour utiliser `ctx.format_catalog` (et non `default()`) ; `format_catalog` ajouté à `EvalCtx` (initialisé depuis la session) ; statement INPUT et fonction INPUT() résolvent les informats utilisateur. 71 nouveaux tests (1674 total, 0 warning, 0 `.snap.new`).
- [x] M18.3 — PICTURE (formats picture utilisateur) : `enum FormatKind{Value,Picture,Invalue}` documente la taxonomie dans `userdef.rs` ; nouveau `UserPicture`/`PictureRange`/`PictureDirectives` (plages numériques → template + directives), `FormatCatalog.user_pictures` + `define_picture` + résolution AVANT les builtins dans `format()` (numérique seulement, missing intercepté en amont → char missing). **Sémantique du rendu (v1 fidèle, documentée dans l'en-tête de `UserPicture`)** : `9` = sélecteur de chiffre qui IMPRIME toujours (zéros de tête inclus) ; `0` = sélecteur qui SUPPRIME les zéros de TÊTE (rempli par le `fill`, espace par défaut) — la suppression ne porte QUE sur le run de zéros avant le 1er chiffre significatif de l'entier (mis à l'échelle, padé à gauche au nb de sélecteurs) ; les caractères-message (`/`,`,`,`.`,`%`,espace…) sont copiés tels quels, mais ceux À GAUCHE du 1er chiffre imprimé sont blanchis (SAS supprime les séparateurs de tête). **MULT** : explicite gagne, sinon AUTO-dérivé `10^(sélecteurs après le dernier '.')` (donc `'009.99'` sur 1.5 → `  1.50`). **PREFIX** inséré avant le 1er glyphe imprimé (remplace le run de `fill` de tête). **Négatifs** : magnitude rendue + `-` préfixé (nuances PREFIX='-'/signe-2 NON modélisées). Arrondi half-away-from-zero. Parser PROC FORMAT : `picture name <range>='template' [(prefix=/mult=/fill=)]` ; plages numériques réutilisent `parse_single_range`, bornes chaînes (`'01'-'12'`) rejetées (numérique seul) ; `other='template'` ; directives espace-séparées `KEY=VALUE`, directive inconnue → erreur propre ; NOTE "Format NAME has been output." 27 nouveaux tests (14 renderer dans userdef.rs : 9/0, suppression, séparateurs `/`,`,`,`.`,`%`, prefix, mult auto & explicite, fill, négatif, sélection de plage, other, non-numérique→None ; 6 parser + 7 exec dans format.rs : prefix/mult/fill, width right-justify, missing, shadow builtin COMMA, PUT() bout-en-bout, FORMAT statement+PROC PRINT). 1701 tests + snapshot verts, 0 warning `-D warnings`, 0 `.snap.new` (m1–m17 octet-identiques). Différés documentés : auto-dérivation MULT simplifiée, signe négatif simplifié, pas de `DATATYPE=`. (Opus, moyen)
- [x] Fixtures `m18/` + snapshots. DoD : `m18/formats.sas` **vérifié à la main** — INVALUE numérique (grade A→4, C→2, X→. via other=.), COMMA10.2 (12,345.60), ROMAN (9→IX), formats date (21915 = 2020-01-01 → DOWNAME Wednesday, MONNAME January, QTR 1), PICTURE dollarpic (1234.5 → $1,234.50). m1–m17 octet-identiques. **Écarts de fidélité SAS découverts au DoD, documentés comme différés (hors périmètre du snapshot fidèle)** : (1) `input('M', $sz.)` avec informat caractère utilisateur renvoie missing au lieu de la chaîne — le type de retour d'INPUT() n'honore pas encore le `$` de l'informat utilisateur (à corriger en M18.x/M19) ; (2) `put(255, hex4.)`/`binary8.`/`octal3.` ne zéro-paddent pas à la largeur (`FF` au lieu de `00FF`, `101` au lieu de `00000101`, `10` au lieu de `010`) — largeur ignorée pour HEX/BINARY/OCTAL. **M18 TERMINÉ.**

## M19 — macro : fonctions différées
- [x] M19.1 — `%unquote`, `%cmpres`/`%qcmpres`, `%symexist`, `%sysmexist`, `%sysget`, `%sysevalf` (éval flottante) ; triggers + `consume_*` dans `preprocess.rs`. **%unquote** (inverse du quoting) : ré-expanse l'argument (pose les sentinelles des `%str`/`%nrstr`/`%q*` imbriqués) → `unmask` (ressuscite `&`/`%` masqués) → ré-`process_impl` (résout ces déclencheurs ressuscités) ; la passe `unmask` finale d'`expand_open_code` n'a plus rien à faire sur le fragment. **%cmpres**/**%qcmpres** : `compress_blanks` (trim + suite de blancs → 1 espace) ; `q` masque le résultat (schéma de sentinelles partagé). **%symexist** : 1/0 via `lookup` (portées+global). **%sysmexist** : 1/0 via `self.macros`. **%sysget** : `std::env::var` (var absente → chaîne vide, déterministe ; les tests posent leur var en mémoire via `set_var`, jamais de dépendance au vrai env). **%sysevalf** : évaluateur récursif-descendant FLOTTANT `FloatParser` (parallèle à `EvalParser`, réutilise `tokenize_eval` — les littéraux décimaux/exposant sont désormais tokenisés en `Word`, rejetés par `%eval` entier mais acceptés par `%sysevalf`) ; division/`**` RÉELLES ; conversions `BOOLEAN`/`CEIL`/`FLOOR`/`INTEGER` ; `7/2`→`3.5` (vs `%eval`→`3`). `consume_let` étendu pour ne pas couper sur un `;` interne de `%cmpres`/`%qcmpres`/`%unquote`/`%sysevalf`. +24 tests (1724 total), 0 warning `-D warnings`, 0 `.snap.new` (m1–m18 octet-identiques). (Fable, moyen-élevé)
- [x] M19.2 — `%include 'path';` + bibliothèques autocall (`SASAUTOS`, recherche + compilation paresseuse) : `consume_include` parse et charge le fichier (chemin relatif vs base_dir, absolu tel quel), expanse RÉCURSIF (`process_impl`), splice dans stream. Profondeur max 50 (cycle guard). `try_autocall` cherche `nom.sas` dans `sasautos_path` (FIFO), compile une seule fois (memoization `autocall_tried`). Session stocke `include_base_dir`+`sasautos_path` du MacroEngine. +15 tests (14 include + 8 autocall, dedup), 1739 total, 0 warning, m1–m18 octet-identiques. (Opus, moyen)
- [x] M19.3 — options de trace `MPRINT`/`MLOGIC`/`SYMBOLGEN` (écho conditionnel au log) ; `%put` statement ; `%call execute` côté macro (Opus, moyen) : **MPRINT** flag → écho chaque ligne non-vide produite par l'expansion d'une macro (préfixe `MPRINT(nom):`) ; utile pour déboguer les macros. **MLOGIC** flag → écho entrée/sortie de macro + paramètres + conditions `%if TRUE/FALSE` (préfixe `MLOGIC(nom):`). **SYMBOLGEN** flag → écho chaque résolution `&symbol` (indirection `&&v&i` résolue jusqu'au point fixe avant trace). **`%put text;`** → traitement immédiat : résolution `&var` + `%function`, puis écriture AU LOG (n'émet rien dans le code stream ; `%put;` vide écrit une ligne vide). **`%call execute(code);`** dans une macro → le code (résolu) est mis en file pour exécution APRÈS le segment courant, comme `CALL EXECUTE` côté DATA step (file partagée `call_execute_queue` en session). `MacroEngine` : drapeaux `mprint`/`mlogic`/`symbolgen` initialisés de `SasOptions` ; buffers `pending_log_lines` (drainé par exécuteur post-expansion, relayé au `LogWriter`) et `pending_call_execute` (drainé dans la file). `executor.rs` : `parse_macro_trace_flag` (détecte `MPRINT`/`NOMPRINT`/...) ; drain post-expansion et post-segment pour code ouvert. +10 tests (put simple/symbol/empty, mprint/mlogic/if-condition/symbolgen, call execute basic/symbols, multiple flags), 1749 total, 0 warning `-D warnings`, 0 `.snap.new` (m1–m18 octet-identiques).
- [x] Fixtures `m19/` + snapshots. DoD : `tests/fixtures/m19/macro_advanced.sas` (un seul fichier) exerce %unquote (nrstr+unquote → "Hello Alice"), %include d'un .sas qui DÉFINIT une macro puis l'invoque (get_name→work.gn), autocall SASAUTOS (mylib compilé paresseusement→work.auto v=42), %put, MPRINT/MLOGIC/SYMBOLGEN (écho MLOGIC begin/param/%IF TRUE/end + SYMBOLGEN resolves), %call execute côté macro (work.result x=99). + `tests/fixtures/m19/sasautos/mylib.sas` (bibliothèque autocall documentaire ; snapshot vide propre, globée par le harnais). **Snapshots vérifiés à la main vs SAS** : résolutions de symboles, macro expansion, écho trace, exécution différée, NOTEs et listings plausibles. Wiring ajouté (M19.2) : `OPTIONS SASAUTOS='dir';` dans l'executor (chemin résolu vs base_dir → set_sasautos_path) + 1 test unitaire. Pièges du harnais documentés dans la fixture : le processeur macro résout &/% PARTOUT (chaînes ET commentaires) et le segmenteur coupe sur les mots-clés RUN/QUIT (commentaires inclus) ; les fichiers .sas écrits à l'exécution sont donc assemblés via byte(37)/byte(38), les commentaires évitent &/%/run/quit, et les options visibles d'une macro ultérieure (SASAUTOS, flags de trace) sont posées un step plus tôt (frontière run;). `cargo test -p sas_interpreter` : 1750 tests (1749 + 1) + snapshot, 0 warning `-D warnings`, m1–m18 octet-identiques. **M19 TERMINÉ.**

## M20 — PROC SQL : compléments
- [x] M20.1 — `LIKE` complet (matcher SAS maison : `%`=0+, `_`=1, sensible casse, missing exclu) ; `EXCEPT/INTERSECT ALL` exacts (multiplicité via rang d'occurrence `cum_sum().over`) (Opus, moyen)
- [x] M20.2 — sous-requêtes (WHERE/HAVING/SELECT/ON) : scalaires `(SELECT …)`, `[NOT] IN (SELECT …)`, `[NOT] EXISTS (SELECT …)` non-corrélées matérialisées via pré-passe `resolve_subqueries` (scalaire→littéral, IN→chaîne d'égalités, EXISTS→booléen constant) ; corrélées détectées (référence à une table externe) → erreur documentée (Fable, élevé)
- [x] M20.3 — dictionary tables (`DICTIONARY.TABLES/COLUMNS/MACROS`, vues `sashelp.v*`) alimentées par l'état de session ; `src/sql/dictionary.rs` (build_tables/build_columns/build_macros via `Session` state), intercept dans `plan.rs:scan_normalized`, parser bypass libref length check pour DICTIONARY/SASHELP, persistance longueur déclarée en sidecar JSON (M20.3 débloquerait la fidelity du round-trip SAS). +8 tests unitaires (dictionary_*_*). Fixture `dictionary.sas` + snapshot. (Opus, moyen)
- [x] M20.4 — vues SQL (`CREATE VIEW`), `UPDATE ... SET`, sous-requêtes dans `INSERT` (Opus, moyen) : **CREATE VIEW / DROP VIEW** — `SqlStmt::CreateView{name,query}` + `DropView` ; vues stockées en mémoire (`Session.views`, clé UPPERCASE, espace WORK, jamais matérialisées en parquet) ; `exec_create_view` valide le nom (≤32) et écrase à la redéclaration (NOTE "defined"/"redefined") ; résolution dans `plan::scan_source` (si la table FROM/JOIN est une vue stockée → `lower_select` récursif, vues imbriquées admises) ; `DROP TABLE` et `DROP VIEW` partagent la logique (suppression de la vue si présente). **UPDATE ... SET** — `SqlStmt::Update{table,assignments,where_}` ; `parse_update` (SET obligatoire ≥1 colonne, WHERE optionnel) ; `exec_update` charge la table, calcule le masque WHERE via le chemin lazy (`normalize_specials`+`translate_predicate`), évalue chaque assignation via `with_column` (diffusion des littéraux, support `x=x+1`), réécrit les lignes ciblées avec `coerce_update_target` (char→num parse numérique, num→char BEST12.+troncature, missing conservé) ; NOTE "N rows were updated" ; colonne/table inexistante → ERROR. **Sous-requêtes dans INSERT** — INSERT SELECT avec sous-requêtes scalaires/IN/EXISTS (déjà via `resolve_subqueries`) + ajout du support des sous-requêtes en **FROM** (`FROM (SELECT ... UNION ...) AS alias`) : `FromItem.subquery`, parsing dédié, abaissement récursif dans `scan_source`, résolution récursive dans `resolve_subqueries`. +28 tests (1817 total), 0 warning `-D warnings`, 0 `.snap.new`, fixture m20 inchangée.
- [x] Fixtures `m20/` (dictionary, views_update) + snapshots **vérifiés à la main** : dictionary → DICTIONARY.TABLES/COLUMNS/MACROS + alias sashelp.v*, LIKE sur les vues dictionnaire ; views_update → CREATE VIEW girls (9 filles), UPDATE weight+5 si age≥14 (9 lignes, arithmétique +5 exacte), INSERT depuis sous-requête FROM(UNION) (4 amorce age=15 + 7 insérées age∈{11,12}=11 final), DROP VIEW. m1–m19 octet-identiques. `cargo test` vert (1817 + snapshot). **M20 TERMINÉ.**

## M21 — complétion des procs existants + procs utilitaires
- [x] ⫽ M21.1 — `PROC COMPARE`, `PROC PRINTTO`, `PROC OPTIONS`, `PROC CATALOG` (Sonnet, moyen)
  — **PROC COMPARE** (`src/procs/compare.rs`) : BASE=/COMPARE= obligatoires, NOVALUES, BRIEFSUMMARY, OUT=. Rapport listing en 4 sections (Data Set Summary, Variables Summary, Observation Summary, Values Comparison). Comparaison via `sas_cmp` (. = ., char trim trailing blanks). Tolérance numérique = 0 (pas de CRITERION= v1). OUT= dataset avec _TYPE_/_OBS_ + colonnes communes. +11 tests.
  — **PROC PRINTTO** (`src/procs/printto.rs`) : LOG=/PRINT=/NEW, reset nu (`proc printto;`). v1 : stocke les chemins dans `Session.printto_log`/`printto_print` (nouveaux champs), émet NOTE ; routage réel différé à M22 ODS (invariant snapshots préservé). +6 tests.
  — **PROC OPTIONS** (`src/procs/options.rs`) : affiche les options au LOG (pas au listing). Sans liste → toutes ; avec liste → seulement celles demandées ; option inconnue → WARNING. Options exposées : OBS, FIRSTOBS, LINESIZE, PAGESIZE, CENTER, DATE, MPRINT, MLOGIC, SYMBOLGEN. +6 tests.
  — **PROC CATALOG** (`src/procs/catalog.rs`) : run-group proc jusqu'à `quit;`. CONTENTS liste les formats utilisateur depuis `FormatCatalog`. DELETE/COPY : no-op gracieux + NOTE. `FormatCatalog::user_format_names()` ajouté. +5 tests.
  — **Total** : +43 tests (1817 → 1860). 0 warning `-D warnings`. 0 `.snap.new`. Déviations v1 documentées : tolérance CRITERION= (COMPARE), routage physique (PRINTTO), vrais .sas7bcat (CATALOG).
- [x] M21.2 — FREQ : Fisher exact, AGREE (kappa), MEASURES (odds ratio, RR), TREND (Cochran-Armitage), CHISQ 1 voie, `TABLES ... / OUT=` (Opus, élevé)
  — **Architecture** : `TableRequest` étendu de 4 flags bool (`fisher`, `agree`, `measures`, `trend`) ; `parse_tables()` reconnaît les mots-clés `fisher`/`exact`, `agree`, `measures`/`relrisk`, `trend` après `/`. Blocs imprimés conditionnellement et UNIQUEMENT sur leur option → sortie par défaut + CHISQ deux voies (m10) byte-identiques (0 `.snap.new`).
  — **CHISQ une voie** (`chisq_one_way_block`) : test d'ajustement à l'équiprobabilité, exp = N/k, χ² = Σ(obs-exp)²/exp, DF = k-1, p via `chisq_sf`. Bloc "Chi-Square Test for Equal Proportions". Validé : counts 10/20/30/40 → χ²=20, DF=3, p≈0.00017.
  — **Fisher exact** (`fisher_block`) : tables 2×2 exactes via probabilités hypergéométriques `C(r1,a)·C(r2,c1-a)/C(n,c1)` (ln-gamma, pas d'overflow factoriel). F, Left-sided Pr≤F, Right-sided Pr≥F, Table Probability P, Two-sided Pr≤P (somme des probas ≤ p_obs). Validé : [[3,1],[1,3]] → two-sided p = 0.485714 (= 34/70), right-sided 17/70. **r×c (>2×2) DIFFÉRÉ** : note propre "not supported", aucun panic.
  — **MEASURES/RELRISK** (`measures_block`) : 2×2. Odds Ratio = ad/bc, IC 95 % via ln(OR)±1.96·√(1/a+1/b+1/c+1/d). Relative Risk cohorte Col1 = [a/(a+b)]/[c/(c+d)], Col2 = [b/(a+b)]/[d/(c+d)], IC log. Cellule nulle → estimation "." (pas de div/0). Validé : [[20,10],[5,25]] → OR=10 exact, RR Col1=4, CI OR [2.9405, 34.008].
  — **AGREE** (`agree_block`) : kappa simple Cohen sur table carrée. Po=Σp_ii, Pe=Σp_i+·p_+i, κ=(Po-Pe)/(1-Pe). ASE asymptotique (Fleiss et al., la variance H1 de SAS) + IC 95 % (z=1.96). Table non carrée → "AGREE requires a square table". Validé : [[20,5],[10,15]] → Po=0.70, Pe=0.50, κ=0.40 ; 3×3 diagonale forte → κ=0.75 ; diagonale pure → κ=1.
  — **TREND** (`trend_block`) : Cochran-Armitage 2×c ou r×2 (transposition des rôles si r×2). Scores 1..k, T=Σs_i(n_1i - r1·c_i/N), Var=(r1·r2/N)·[Σc_i s_i² - (Σc_i s_i)²/N], Z=T/√Var, p uni/bilatérale via `probnorm`. Autre forme → note propre. Validé à la main : [[5,10,20],[20,10,5]] → T=15, Var=875, Z=0.5071, two-sided 0.6121.
  — **Helpers numériques** (`common.rs`, MAISON) : `probnorm` (CDF normale via erf/incomplete-gamma, validé Φ(1.96)=0.975), `ln_factorial`/`ln_choose` (coefficients binomiaux par ln-gamma, C(8,4)=70). Réutilise `ln_gamma`/`gammq` existants.
  — **OUT=** : périmètre existant (colonnes `<var>`/COUNT/PERCENT en une voie) inchangé et confirmé ; OUTPCT non ajouté (documenté comme hors v1).
  — **Total** : +23 tests (1860 → 1883 lib). 0 warning `-D warnings`. 0 `.snap.new`. Différé documenté : Fisher r×c >2×2, OUTPCT, TESTP= explicite (équiprobable v1).
- [x] M21.3 — UNIVARIATE : tests de normalité (Shapiro-Wilk, Kolmogorov, Anderson-Darling), HISTOGRAM/QQPLOT (→ images M29), percentiles étendus (Fable, élevé)
  — **Architecture** : `UnivariateAst` étendu de `normal: bool` + `graphics_deferred: usize`. Le bloc "Tests for Normality" ne s'active QUE sur l'option `normal` (PROC `proc univariate … normal;` ou clause `var x / normal;`) → rapport par défaut BYTE-IDENTIQUE (snapshots m1–m20 inchangés, 0 `.snap.new`, m5 univariate intact). `parse()` reconnaît `normal`/`normaltest` sur PROC et après `/` sur VAR.
  — **`common::phi_inv(p)`** (NOUVEAU, réutilisable) : quantile normal inverse (probit) par approximation rationnelle d'Acklam + 1 pas de Halley contre `probnorm`. Validé : phi_inv(0.5)=0, phi_inv(0.975)=1.959964, phi_inv(0.95)=1.644854, round-trip `probnorm(phi_inv(p))=p` à 1e-9.
  — **Shapiro-Wilk** (3 ≤ n ≤ 2000) : algorithme de Royston AS R94 (coefficients a_i depuis m_i=Φ⁻¹((i−3/8)/(n+1/4)), corrections polynomiales C1/C2 sur a_n et a_{n−1}, rescale du noyau). p-value par transformation normalisante de Royston (branches n=3 exacte / 4≤n≤11 / n≥12). **Validé contre SAS 9.4** sur sashelp.class height : W=0.979083, Pr<W=0.93118 (identique à SAS). W proche de 1 pour échantillon ~normal, p≈0.0003 pour échantillon avec outlier.
  — **Kolmogorov-Smirnov** (Lilliefors, paramètres estimés) : D=max(D+,D−) sur F=Φ((x−x̄)/s). p-value par approximation analytique de Dallal-Wilkinson (1986), valable région significative p≤0.10, clampée à 1.0 ("p>0.10") au-delà. Validé : D height=0.144272 (identique SAS), Pr>D=0.378 (région non-significative comme SAS >0.15).
  — **Cramér-von Mises** : W²=1/(12n)+Σ(Φ(z_i)−(2i−1)/(2n))², modif W²*=W²(1+0.5/n), p-value par régions de Stephens. Validé : W² height=0.040516 (identique SAS) ; sur {1..5} W²=0.0193421 (vérifié à la main contre la définition exacte).
  — **Anderson-Darling** : A²=−n−(1/n)Σ(2i−1)[lnΦ(z_i)+ln(1−Φ(z_{n+1−i}))], modif A²*=A²(1+0.75/n+2.25/n²), p-value Stephens. Validé : A² height=0.235778 (identique SAS) ; sur {1..5} A²=0.1435942 (vérifié à la main).
  — **HISTOGRAM/QQPLOT/PROBPLOT/CDFPLOT/PPPLOT** : statements parsés (corps sauté jusqu'au `;`, jamais d'erreur/panic), compteur `graphics_deferred` ; une NOTE unique "HISTOGRAM/QQPLOT: graphical output deferred to ODS GRAPHICS (M29)." est émise. **Rendu IMAGE DIFFÉRÉ à M29.**
  — **Robustesse** : n<3, variance nulle, échantillon dégénéré → NOTE centrée, AUCUN panic. Missing exclus (chemin existant). Décodage via `decode_column`/`value_to_num`, jamais `get_row`.
  — **Fixture** : `tests/fixtures/m21/univariate_normal.sas` + snapshot (NORMAL + histogram/qqplot deferred). +15 tests unitaires (1883 → 1898 lib). 0 warning `-D warnings`, 0 `.snap.new`.
  — **DIFFÉRÉ / déviations documentées** : (1) rendu graphique image → M29 ODS GRAPHICS ; (2) p-values KS/CvM/AD = approximations publiées (Dallal-Wilkinson, Stephens) — les **statistiques** sont identiques à SAS à 6 chiffres, les p-values dans la région non-significative diffèrent de la table interne SAS (SAS affiche ">0.15"/">0.25", nous donnons une valeur continue cohérente) ; (3) tests de normalité non calculés avec WEIGHT (chemin pondéré inchangé). **M21.3 TERMINÉ.**
- [x] M21.4 — TABULATE/REPORT avancés : 3ᵉ dim, croisements multi-VAR/stats, ALL, PCTN/PCTSUM (TABULATE) ; ACROSS, COMPUTE, BREAK/RBREAK, LINE, WHERE, OUT= (REPORT) (Opus, élevé). **TABULATE** (`tabulate.rs`, +13 tests) : 3ᵉ dimension page (`table page,row,col;` → une section par catégorie de page ; 4ᵉ dim → erreur propre) ; croisements multi-VAR/stats en colonnes (`A*(N MEAN)`, `height*mean weight*sum` ; croisement de 2 VAR/stats dans une même cellule reste erreur propre) ; `ALL` (classe universelle `Atom::All`, ne dé-contraint que sa propre dimension, totaux marginaux) ; PCTN/PCTSUM (dénominateur v1 = grand total ; cellule « . » si dénominateur nul ; PCTSUM sans VAR → erreur). Validé main : MEAN height ALL=62.12, PCTN M=60%/F=40%, PCTSUM E=40%/W=60%, pages grp=A/grp=B. Différé propre : dénominateurs de groupe `PCTN<row>`. **REPORT** (`report.rs`, +19 tests) : `WHERE` (évaluateur de prédicat local fidèle SAS sur l'AST datastep, `sas_cmp`/`. = .`/char trim, `in(...)`, truthiness AND/OR/NOT ; construction non supportée → missing sans panic) ; `OUT=` (corps du rapport détail/groupé + sous-totaux BREAK → dataset, types SAS préservés, RBREAK grand-total exclu, NOTE de création, `last_dataset`) ; `ACROSS` (valeurs distinctes → colonnes, cellule = stat ANALYSIS au croisement GROUP×ACROSS ; v1 = 1 across + 1 analysis, en-tête aplati `"valeur STAT"`, order=desc honoré ; OUT= sur across différé) ; `BREAK`/`RBREAK` (`break after var / summarize;` sous-total recalculé ; `rbreak after / summarize;` total général ; OL/DOL/SKIP/PAGE acceptés cosmétiques) ; `COMPUTE`/`ENDCOMP`+`LINE` (affectations simples `col=expr;` par ligne + `compute after; line ...; endcomp;` texte libre ; COMPUTE complexe `_Cn_`/fonctions/formats → erreur parse propre). Validé main : sommes par sexe F=36/M=29 total 65, WHERE age>12 → F=13/M=29, ACROSS crosstab 10/20/30/40, COMPUTE age*2. **Total** : +32 tests (1898 → 1927 lib + snapshots). 0 warning `-D warnings`. 0 `.snap.new` (m1-m21 byte-identiques ; m9/tabulate+report inchangés). 0 `todo!()`. NB : implémentés en parallèle (2 agents Opus, fichiers indépendants), validés ensemble par l'orchestrateur.
- [x] M21.5 — RANK méthodes FRACTION/PERCENT/NORMAL/SAVAGE + BY ; CORR Spearman/Kendall + OUT=/OUTP= + WEIGHT (Opus, moyen-élevé). **RANK** (`rank.rs`) : méthodes FRACTION (r/n), NPLUS1 (r/(n+1)), PERCENT (100·r/n), NORMAL=BLOM/TUKEY/VW (score normal Φ⁻¹ via `common::phi_inv`), SAVAGE (scores exponentiels Σ1/j−1) ; transformation appliquée APRÈS TIES ; deux méthodes mutuellement exclusives → erreur. `BY` honoré (rangs recalculés indépendamment par groupe via `common::by_groups`, tri vérifié sas_cmp, erreur "not sorted" sinon). Validé main : FRACTION [10,20,30,40]→0.25/0.5/0.75/1.0, NPLUS1→0.2/0.4/0.6/0.8, NORMAL=BLOM y=(r-0.375)/4.25, BY 2 groupes indépendants. **CORR** (`corr.rs`) : Spearman (Pearson sur rangs moyens midrank, p via t ddl=n−2), Kendall tau-b ((n_c−n_d)/√((n0−n1)(n0−n2)) avec correction ties, p via approx normale `z=3τ√(n(n−1))/√(2(2n+5))`), OUT=/OUTP=/OUTS=/OUTK= (dataset TYPE=CORR : `_TYPE_`/`_NAME_` + MEAN/STD/N + matrice CORR), WEIGHT (Pearson pondéré : moyennes/(co)variances pondérées, exclut w≤0/manquant). Validé main : Spearman monotone→1.0, [(1,1),(2,3),(3,2),(4,4)]→0.8, ties→0.83333 ; Kendall même données→0.66667, tie-x→5/√30=0.91287 ; pondéré x[1,2,3]/y[1,2,9]/w[1,1,10]→0.96183 vs 0.91766 non pondéré. Différés : Spearman/Kendall non pondérés (WEIGHT→Pearson seul), Kendall p sans correction ties dans la variance, dénominateurs de groupe. **Corrections orchestrateur** : 2 tests rank obsolètes (`parse_by_not_implemented`, `parse_unsupported_method_errors`) retirés (features désormais implémentées), 1 test corr (`weighted_changes_result`) réécrit — son intuition était fausse (down-weighting d'un outlier extrême ne remonte PAS r car le dy² de l'outlier domine syy même à poids 1 ; l'implémentation est SAS-correcte) ; lint `unused_mut` corrigé. **Total** : +32 tests (1927 → 1959 lib). 0 warning `-D warnings`. 0 `.snap.new` (m9/rank+corr byte-identiques). 0 `todo!()`. Implémentés en parallèle (2 agents Opus, fichiers indépendants).
- [x] Fixtures `m21/` + snapshots **vérifiés à la main** : `univariate_normal` (M21.3 — Shapiro-Wilk W=0.979083/p=0.93118, KS/CvM/AD, HISTOGRAM/QQPLOT différés→NOTE M29) ; `freq_stats` (M21.2 — CHISQ 1 voie sex χ²=0.0526/p=0.8185, Fisher exact [[3,1],[1,3]] bilatéral=0.4857, MEASURES OR + RR cohorte, AGREE kappa [[4,1],[1,4]]=0.6000 ; données développées en lignes car FREQ ne pondère pas via WEIGHT — gap documenté) ; `rank_corr` (M21.5 — RANK FRACTION sur height [Philip 72→1.0, ties 62.5→0.4474], RANK BY sexe rangs indépendants, CORR Spearman 0.856/Kendall 0.714) ; `compare_report` (M21.1 PROC COMPARE 2 obs diff/Max Diff 5.0 + M21.4 REPORT WHERE age≥13/GROUP/RBREAK total : F age14 h62.98, M age14.5 h66.75, total age14.25 h64.87). **Correctif orchestrateur** : REPORT rejetait les statements globaux TITLE/FOOTNOTE en ligne (hard-error vs leniency de PRINT) → `is_global_stmt_kw` les saute désormais gracieusement (cohérent SAS : globaux valides dans un PROC step). 1959 tests + 4 snapshots m21 verts, 0 warning. **M21 TERMINÉ.**

## PHASE B — ODS (prérequis capture + graphiques)

## M22 — couche de routage ODS + capture
- [x] M22.1 — trait `OutputDestination { page_header; write_table; write_line; blank }` dans `src/output/` ; `TextListing` = comportement actuel, `Session.listing: Box<dyn OutputDestination>` + registre multi-destinations. Invariant : listing texte par défaut byte-identique. (+9 tests, 1959 → 1968, 0 warning, 0 `.snap.new`)
- [x] M22.2 — parseur du statement `ODS` (ouvrir/fermer destinations) + options globales NOCENTER/DATE/NUMBER (Opus, moyen). (+19 tests, 1968 → 1987, 0 warning, 0 `.snap.new`)
- [x] M22.3 — ODS OUTPUT → datasets (mapping nom-de-table ODS → `OUT=`) ; utile pour tester les procs stat à venir (Opus, moyen-élevé)
  — **Parser** (`global.rs`, `parse_ods_output`) : `ODS OUTPUT table=ds [table2=ds2 ...] ;` → `GlobalStmt::OdsOutput { mappings: Vec<(String, DatasetRef)>, close: bool }` ; `ODS OUTPUT CLOSE ;` → `close=true` (purge). Nom de table ODS conservé tel quel à l'AST, UPPERCASE à l'exécution (matching insensible à la casse). Erreur propre si `=` ou cible manquante.
  — **Session** : `ods_output_map: HashMap<String, DatasetRef>` (clé table ODS UPPERCASE), méthodes `set_ods_output` / `clear_ods_output` / `ods_output_target`. **Vide par défaut** → capture inactive, listing byte-identique (invariant m1–m21).
  — **Executor** : `GlobalStmt::OdsOutput` enregistre/purge les mappings.
  — **Capture branchée : PROC MEANS table "Summary"** (`means.rs`, `write_ods_summary`). Après le rapport, si `ods_output_target("Summary")` est `Some`, écrit un dataset = une obs par variable de VAR, colonne char `Variable` + une colonne numérique par stat du rapport (N/Mean/StdDev/Min/Max par défaut, libellés via `ods_summary_stat_colname`). Réutilise `compute`/`compute_weighted`/`partition_numeric`/`partition_weighted` existants. `last_dataset` mis à jour + NOTE de création. v1 : agrégation globale (CLASS/BY non partitionnés dans Summary — à brancher au cas par cas).
  — **DIFFÉRÉ / au cas par cas** : autres tables ODS (FREQ OneWayFreqs/CrossTabFreqs, UNIVARIATE Moments, etc.) non encore branchées — l'infra est en place, il suffit que chaque proc consulte `ods_output_target("<Table>")` ; partition CLASS/BY dans Summary ; ODS SELECT/EXCLUDE restent différés (parser les rejette proprement).
  — **Tests** : +5 parser (single/two/qualified mappings, CLOSE, erreur sans `=`), +4 exécution (capture Summary colonnes+valeurs, table case-insensitive, inactif→aucun dataset & listing inchangé, multi-VAR 1 obs/var). +9 total (1987 → 1996 lib). 0 warning `-D warnings`, 0 `.snap.new`, 0 `todo!()`.
- [x] M22.4 — destination HTML (tables CSS, fichier `.html`) (Sonnet, moyen)
  — **`HtmlDestination` réelle** (`src/output/mod.rs`, sortie de la macro `stub_destination!` ; RTF/PDF/Excel restent des stubs) : `new(ls)` / `with_file(ls, PathBuf)` ; `page_header`→`<h1 class="systitle">` ; `write_table`→`<table class="sas"><thead><tbody>` (cellules HTML-échappées, `Align::Right`→`style="text-align:right"`) ; `write_line`→`<p>` ; `blank` no-op ; `into_string`→document complet (DOCTYPE + CSS embarqué + body), drain idempotent ; helper `html_escape` (`&` en premier).
  — **Trait** : méthode `finalize(&mut self) -> Option<(PathBuf, String)>` (défaut `None` ; HTML renvoie `Some((path, doc))` si FILE= configuré).
  — **Flush** : `Session::close_destination` écrit le fichier HTML via `finalize()` + NOTE « Writing HTML Body file: <nom> » (file_name seul → snapshots déterministes). Filet de sécurité fin de programme dans `lib.rs::run` (FILE= sans CLOSE explicite).
  — **Câblage** : `executor::exec_ods` branche `ODS HTML FILE=` via `with_file(resolve_path)` ; sans FILE= → NOTE informative, pas de fichier.
  — **v1 documenté** : destinations EXCLUSIVES (ouvrir HTML remplace le listing courant ; pas de fan-out concurrent listing+HTML comme SAS). +8 tests unitaires.
- [x] Fixtures `m22/` + snapshots (texte inchangé ; HTML/dataset capturés vérifiés). DoD
  — **3 fixtures** `tests/fixtures/m22/` : `ods_output.sas` (capture MEANS Summary→dataset, **vérifié main** : height N19/mean62.336842/std5.127075, weight mean100.026316/std22.773933, valeurs identiques au listing) ; `ods_html.sas` (routage HTML→fichier, NOTE d'écriture, listing réactivé après CLOSE) ; `ods_options.sas` (NOCENTER/NODATE/NONUMBER acceptées **sans warning**). Snapshots vérifiés à la main.
  — **Test d'intégration** `tests/ods_html.rs` (2 tests) : relit le `report.html` généré et asserte DOCTYPE/`<table class="sas">`/`<th>`/cellule `Alfred`/`text-align:right` ; et `ODS HTML` sans FILE= → aucun fichier.
  — **Correctif orchestrateur** : titres de fixtures passés en ASCII (le lexer de littéraux chaîne double-encode l'UTF-8 non-ASCII dans le titre rendu — bug pré-existant hors périmètre M22, contourné). `tests/common/mod.rs` reçoit `#![allow(dead_code)]` (helpers partagés entre binaires de test).
  — **Total** : 2004 lib + 2 intégration + snapshots m22, 0 warning `-D warnings`, 0 `.snap.new` (m1–m21 octet-identiques). **M22 TERMINÉ.**

## M23 — ODS RTF / PDF / Excel
- [x] ⫽ M23.1 — RTF (séquences de contrôle Word) (Opus, moyen)
  — `RtfDestination` réelle : `new(ls)` / `with_file(ls, PathBuf)` ; `page_header`→`\pard\sb200\sa100\b title\b0\par` ; `write_table`→`\trowd\trgaph100\cellx<twips>...\intbl\pard\ql/\qr en-têtes \b\b0\cell...\row` (largeurs en twips calculées sur max content) ; `write_line`→`\pard text\par` ; `into_string`→wrapping RTF complet ; `finalize()`→`Some((path,rtf))` si FILE= configuré. RTF-escape : `\`→`\\`, `{`→`\{`, `}`→`\}`, non-ASCII→`\'XX`. `dest_type_label()="RTF Body"`. Tests : structure RTF, escape, finalize avec/sans fichier.
- [x] ⫽ M23.2 — PDF pur Rust (PDF 1.4 minimal, sans printpdf) (Opus, moyen-élevé)
  — `PdfDestination` réelle : accumule `PdfSection::{PageHeader,Table,Line,Blank}`. `finalize_to_bytes()`→PDF 1.4 complet (Catalog/Pages/Page/ContentStream/Font Helvetica) avec xref calculé au byte près ; stream texte via `BT/Tm/Tj/ET` ; tables columnarées par `Tm` absolu par cellule ; page overflow simple (y<50→y=742) ; pdf-escape (parenthèses, backslash, non-ASCII→`?`). `dest_type_label()="PDF"`. Tests : magic bytes `%PDF-`, finalize avec/sans fichier.
- [x] ⫽ M23.3 — Excel pur Rust (XLSX = ZIP+XML, sans rust_xlsxwriter) (Sonnet, moyen)
  — `ExcelDestination` réelle : accumule `ExcelTable` (sheet_name, pre_lines, headers, rows). `finalize_to_bytes()`→XLSX (ZIP non-compressé avec CRC-32 maison) ; XML : `[Content_Types].xml` / `_rels/.rels` / `xl/workbook.xml` / `xl/_rels/workbook.xml.rels` / `xl/worksheets/sheetN.xml` ; cellules `inlineStr` ; références colonnes A-Z/AA-AZ. XLSX magic bytes `PK` (ZIP). `dest_type_label()="Excel"`. Tests : magic bytes PK, finalize avec/sans fichier.
  — **Sans dépendance externe** : rust_xlsxwriter retiré du Cargo.toml (disque quasi plein, 98%) ; XLSX pur Rust ≡ même interface pour les tests.
- [x] Trait étendu : `finalize_to_bytes() -> Option<(PathBuf, Vec<u8>)>` (défaut None) pour formats binaires (Excel/PDF) ; `dest_type_label() -> &'static str` pour les NOTEs de log.
- [x] Session `close_destination()` : tente `finalize_to_bytes()` (binaire) puis `finalize()` (texte) ; NOTE `"Writing <label> file: <name>"`.
- [x] `lib.rs` : filet de sécurité en fin de programme pour les deux variantes.
- [x] `executor.rs` : `exec_ods` câble FILE= pour RTF/PDF/Excel via `with_file(resolve_path)`.
  — **v1 documenté** : destinations EXCLUSIVES (listing texte inactif pendant HTML/RTF/PDF/Excel) ; pas de fan-out concurrent.
- [x] Fixtures `m23/` + snapshots (assertion structure/existence des fichiers). DoD
  — 3 snapshots `tests/snapshots/snapshot__fixtures@m23__ods_{rtf,pdf,excel}.sas.snap` **vérifiés à la main** : ODS RTF/PDF/Excel → NOTE "Writing {RTF Body|PDF|Excel} file: report.{rtf,pdf,xlsx}" ; listing texte reprend après CLOSE (3 obs Alfred/Alice/Barbara). m1–m22 octet-identiques. **M23 TERMINÉ.**

## PHASE C — modélisation statistique

## M24 — fondation numérique stat + procs de base
- [x] M24.1 — module `src/stat/` (maison) : promotion des helpers `common.rs` (Beta/Gamma/t/χ²) + normale (CDF/quantile), F, gamma, bêta ; `linalg.rs` (Cholesky, QR/Householder, moindres carrés, inversion, valeurs/vecteurs propres symétriques par Jacobi), tout testé contre valeurs documentées (Opus, élevé). **M24.1 TERMINÉ** : `src/stat/dists.rs` 11 fonctions promues de common.rs + 8 distributions nouvelles (chisq/F/gamma/beta CDFs/quantiles via Newton-Raphson avec bisection fallback) ; `src/stat/linalg.rs` 6 fonctions (Cholesky, QR Householder, least_squares, invert, Jacobi eigenvalues/eigenvectors) ; 35 unit tests (21 dists + 14 linalg), 2051 tests total verts, 0 warning.
- [x] M24.2 — `PROC TTEST` (1 échantillon, 2 échantillons groupés/appariés, Satterthwaite, CLASS, PAIRED) (Opus, élevé) : parser (DATA=, H0=, ALPHA=, CI=, EQUAL=YES|NO, SIDES=2|U|L, sub-statements VAR/CLASS/PAIRED/BY/OUTPUT) + executor (3 modes — 1-sample, 2-sample CLASS avec Pooled ET Satterthwaite + F-test égalité variances, Paired) ; sas_cmp pour niveaux CLASS ; p bilatérale via `student_t_cdf` ; ODS OUTPUT TTest → dataset. +8 tests (2051→2059 total, 0 warning nouveau, 0 `.snap.new`).
- [x] M24.3 — `PROC NPAR1WAY` (Wilcoxon, Kruskal-Wallis, scores) (Opus, moyen-élevé) : parser (DATA=, OUT=, ALPHA=, WILCOXON/KRUSKAL flags, sub-statements CLASS requis/VAR/OUTPUT ; BY → erreur propre) + executor (midranks pour ex æquo, tie_factor = 1 − Σ(t³−t)/(n³−n) ; Wilcoxon rank-sum k=2 : Z+p via probnorm ; Kruskal-Wallis k≥2 : H/tie_factor+df+p via chisq_cdf ; listing "One-Way Analysis of <var>" ; sas_cmp pour niveaux CLASS) ; OUT= parsé mais dataset non émis (différé). +3 tests (2059→2062 total, 0 warning nouveau, 0 `.snap.new`).
- [x] Fixtures `m24/` + snapshots vérifiés contre SAS. DoD
  — `ttest.sas` (1-sample height H0=60 : t=1.9867/df=18/p=0.0624 ; 2-sample height+weight by sex Pooled+Satterthwaite+F-equality) ; `npar1way.sas` (Wilcoxon height W=73/E=90/σ=12.24/Z=−1.389/p=0.165, weight W=71/Z=−1.554/p=0.120 — vérifiés à la main rangs midrank et tie_factor). m1–m23 octet-identiques. **M24 TERMINÉ.**

## M25 — modèle linéaire
- [x] M25.1 — `PROC REG` (OLS via QR, MODEL, R²/F/t, OUTPUT OUT= résidus/prédits, TEST, intervalles) (Fable, élevé)
  — `src/procs/reg.rs` (parser + executor) : DATA=, MODEL dep = x1 x2 .../NOINT(déf.)/NOPRINT, OUTPUT OUT= PREDICTED= RESIDUAL= ; listwise deletion des missings ; ANOVA table (Model/Error/Corrected Total, SSM/SSE/SST, MSM/MSE, F, Pr>F via `f_cdf`) ; fit statistics (Root MSE, R², Adj R², C.V., Dep Mean) ; Parameter Estimates (Intercept + régresseurs, β̂, SE via (X'X)⁻¹ diagonal × MSE, t, Pr>|t|) ; OUTPUT dataset complete-cases + colonnes PREDICTED/RESIDUAL. `src/stat/linalg.rs` : `transpose`/`matrix_mult`/`matrix_vec_mult` rendus `pub(crate)` (résout les warnings dead-code de M24.1). Validé sur d.class `model weight=height` : R²=0.7705 (=0.87779²), β₁=3.89903, t=7.55, p<.0001. Différés documentés : NOINT (error propre), TEST statement, CLI/CLM intervalles, BY. +9 tests unitaires (2062→2071 lib), fixture `tests/fixtures/m25/reg.sas` + snapshot.
- [x] M25.2 — `PROC ANOVA` (plans équilibrés, CLASS, MEANS, types de SC) (Opus, élevé)
  — `src/procs/anova.rs` (parser + executor) : DATA=, CLASS (liste de variables, distinct levels via sas_cmp), MODEL dependants = effets (liste de dépendants côté gauche, effets côté droit), MEANS (table des moyennes de cellules par niveau CLASS) ; listwise deletion (dep+class non-manquants) ; ANOVA table (Model/Error/Corrected Total, SSM=Σnᵢ(ȳᵢ−ȳ)², SSE=ΣΣ(yᵢⱼ−ȳᵢ)², F/Pr>F via f_cdf) ; fit statistics (R-Square, Coeff Var, Root MSE, dep Mean) ; Type I SS et Type III SS (identiques pour one-way) ; table MEANS (N/Mean/Std Dev par niveau). Validé oracle t²=F sur d.class : height F=2.11/p=0.1645, weight F=3.73/p=0.0702 (=(-1.4526)² et (-1.9322)² du PROC TTEST M24.2). Différés documentés : effets d'interaction (a*b), CLASS multiples en modèle multi-voies. +5 tests unitaires (2072 lib), fixture `tests/fixtures/m25/anova.sas` + snapshot.
- [x] M25.3 — `PROC GLM` (codage CLASS, SC type I/III, LSMEANS, contrastes, ESTIMATE) (Fable, élevé)
  — `src/procs/glm.rs` (parser + executor) : DATA=, CLASS, MODEL dep1 dep2 = eff /SOLUTION /NOPRINT, LSMEANS eff /se, ESTIMATE 'label' eff c1 c2, CONTRAST 'label' eff c1 c2, MEANS ; codage reference-cell (dernier niveau = 0 / "B") pour Parameter Estimates ; LSMEANS = moyennes de cellules avec SE=√(MSE/nᵢ) et test H₀:μᵢ=0 ; ESTIMATE = combinaison linéaire Σcᵢȳᵢ / SE=√(MSE×Σcᵢ²/nᵢ) / t-test ; CONTRAST = F=Estimate²/(MSE×Σcᵢ²/nᵢ) / df=1. Validé oracle t²=F : height F=2.11/p=0.1645, ESTIMATE 'F vs M' t=−1.45/p=0.1645, weight F=3.73/p=0.0702, ESTIMATE t=−1.93/p=0.0702. Différés documentés : multi-facteurs Type III≠Type I, interactions, sweep g-inverse. +7 tests unitaires (2079 lib), fixture `tests/fixtures/m25/glm.sas` + snapshot.
- [x] Fixtures `m25/` + snapshots. DoD — reg.sas/anova.sas/glm.sas + 3 snapshots vérifiés. **M25 TERMINÉ.**

## M26 — modèles catégoriels
- [x] M26.1 — `PROC LOGISTIC` (logistique binaire, Newton-Raphson/IRLS, odds ratios, CLASS, LINK=) (Fable, élevé)
  — `src/procs/logistic.rs` (parser + executor) : DATA=, FREQ statement, MODEL y(descending event='val') = x1 x2 / noprint ; NR/IRLS convergence GCONV=1e-8 (max 50 iter) ; 7 sections listing (Model Information, Response Profile, Model Convergence Status, Model Fit Statistics, Testing Global Null Hypothesis BETA=0, Analysis of ML Estimates, Odds Ratio Estimates) ; DESCENDING/EVENT= pour sélection de l'événement ; pondération par FREQ ; CLASS variables détectées → erreur propre "not yet implemented" ; sas_cmp pour tri des niveaux. **Oracle validé sur 2×2 FREQ table (n=60 pondéré)** : β₁=2.3026 ✓, SE(β₁)=0.6245 ✓, OR=10.0000 ✓, CI=[2.9405,34.0083] ✓, -2LogL=66.8990 ✓, LR χ²=16.2787 ✓, Wald χ²(β₁)=13.5946 (analytiquement exact). AIC/SC null utilise n_total pondéré = 60. +10 tests unitaires (2079→2089 lib). Fixture `tests/fixtures/m26/logistic.sas` + snapshot. 0 warning nouvelles, 0 `.snap.new` (m1–m25 octet-identiques).
- [x] M26.2 — `PROC GENMOD` (GLM exponentiels : Poisson, binomial, normal via Newton-Raphson/IRLS) (Sonnet, élevé)
  — `src/procs/genmod.rs` (parser + executor) : DATA=, FREQ statement, MODEL y(descending event='val') = x1 x2 / dist= link= noprint ; IRLS convergence GCONV=1e-8 (max 50 iter) ; 4 sections listing (Model Information, Response Profile [Binomial], Model Convergence Status, Criteria For Assessing Goodness Of Fit [Deviance/Pearson/LL/AIC/AICC/BIC], Analysis Of Maximum Likelihood Parameter Estimates [β/SE/Wald CI/Wald χ²/p/Scale]) ; liens canoniques par défaut (Poisson→Log, Binomial→Logit, Normal→Identity) ; pondération par FREQ ; DIST=GAMMA différé (parse OK, execute → error) ; CLASS variables → erreur propre. **Oracles validés** : Poisson β₁=0.9163 ✓, SE(β₁)=0.4830 ✓, Deviance=1.4492 ✓, Pearson=1.4000 ✓, LL=7.3005 ✓, AIC=-10.6009 ✓ ; Binomial β₁=2.3026 ✓, SE(β₁)=0.6245 ✓, Wald=13.5946 ✓ (cross-check LOGISTIC) ; Normal β₀=2.0000 ✓, β₁=3.0000 ✓, Scale=1.0000 ✓, SE(β₁)=0.8165 ✓. +12 tests unitaires (2089→2101 lib). Fixture `tests/fixtures/m26/genmod.sas` + snapshot. 0 warnings nouvelles, 0 `.snap.new`.
- [x] Fixtures `m26/` + snapshots. DoD — logistic.sas + genmod.sas + 2 snapshots vérifiés. **M26 TERMINÉ.**

## M27 — multivarié
- [x] M27.1 — `PROC PRINCOMP` (ACP via valeurs propres covariance/corrélation) (Opus, élevé)
- [x] M27.2 — `PROC FACTOR` (extraction, rotation VARIMAX) (Fable, élevé)
- [x] M27.3 — `PROC CLUSTER` + `PROC FASTCLUS` (k-means), `PROC DISTANCE` (Opus, élevé)
- [x] M27.4 — `PROC DISCRIM` (analyse discriminante linéaire) (Opus, élevé)
- [x] Fixtures `m27/` + snapshots. DoD

## M28 — modèles mixtes (le plus difficile, en dernier)
- [x] M28.1 — `PROC MIXED` (effets fixes + aléatoires, REML/ML itératif, structures VC/CS/AR(1)/UN, SOLUTION, LSMEANS) (Fable, très élevé)
  — `src/procs/mixed.rs` (parser + executor) : DATA=, METHOD=REML (défaut)/ML, CLASS, MODEL dep = / SOLUTION, RANDOM INTERCEPT / SUBJECT= TYPE=VC/CS ; algorithme EM/closed-form REML et ML pour random intercept model ; design balancé → solution fermée (méthode des moments) ; déséquilibré → recherche golden-section sur λ=σ²_u/σ²_e ; effets fixes β̂ et SE via (X'V⁻¹X)⁻¹ ; ddfm=Contain (df = nb sujets − nb params fixes) ; 8 sections listing (Model Info, Class Level Info, Dimensions, Nobs, Iteration History, Covariance Parameter Estimates, Fit Statistics, Solution for Fixed Effects) ; TYPE=AR(1)/UN → erreur propre ; LSMEANS/ESTIMATE/CONTRAST/REPEATED/COVTEST → NOTE non implémenté. **Oracles validés** : design balancé 2 sujets×2 obs (y=1,3,5,7) : σ²_u_REML=7.0000 ✓, σ²_e=2.0000 ✓, μ̂=4.0000 ✓, SE=2.0000 ✓, df=1 ✓, p=0.2952 ✓ ; ML : σ²_u_ML=3.0000 ✓ (≠ REML → preuve restriction REML). Fixture `tests/fixtures/m28/mixed.sas` + snapshot.
- [x] M28.2 — `PROC GLIMMIX` (modèles mixtes généralisés, pseudo-vraisemblance) (Fable, très élevé)
  — `src/procs/glimmix.rs` : RSPL (Residual Pseudo-Likelihood = PQL de Breslow-Clayton 1993) ; DIST=NORMAL/POISSON/BINARY (BINOMIAL) + LINK=IDENTITY/LOG/LOGIT ; RANDOM INTERCEPT / SUBJECT=var TYPE=VC ; FREQ statement ; 3-way dispatch : NORMAL/IDENTITY+random → solveur MIXED (RSPL=REML, solution exacte) ; POISSON+LOG → IRLS pondéré (poids=μ, réponse de travail z=η+(y-μ)/μ) ; BINARY+LOGIT → IRLS logistique (poids=μ(1-μ), z=η+(y-μ)/(μ(1-μ))) ; METHOD=RSPL (défaut) ; METHOD=LAPLACE/QUAD → erreur propre ; DIST=GAMMA → erreur propre ; TYPE=AR(1)/UN → erreur propre ; ESTIMATE/CONTRAST/LSMEANS/WEIGHT → NOTE non implémenté. **Oracles cross-validés** : Poisson β₀=0.6931 SE=0.4082 ✓, β₁=0.9163 SE=0.4830 ✓ (= GENMOD) ; Binary+FREQ β₀=-0.9163 SE=0.3742 ✓, β₁=2.3026 SE=0.6245 ✓, DF=58 ✓ (= LOGISTIC) ; Normal+random σ²_u=7.0000 ✓, σ²_e=2.0000 ✓, μ̂=4.0000 SE=2.0000 ✓, -2RLPL=14.0588 ✓ (= MIXED). Fixture `tests/fixtures/m28/glimmix.sas` + snapshot. 11 tests unitaires.
- [x] Fixtures `m28/` + snapshots. DoD : `tests/fixtures/m28/mixed.sas` + `tests/fixtures/m28/glimmix.sas` + 2 snapshots vérifiés à la main (oracle REML σ²_u=7, ML σ²_u=3 ; GLIMMIX 3-way cross-check). `cargo test -p sasrs` : 2169 tests, 0 échec.

## M28a — PROC IML (Interactive Matrix Language)
Langage matriciel pour calcul scientifique et développement d'algorithmes personnalisés. Exécution d'énoncés matriciels dans un environnement interactif, sortie vers datasets (OUTMATRIX).

- [x] M28a.1 — création/indexation/opérations matricielles : `X = {...}` (création littérale), `A[i,j]` (indexation), `A + B`, `A * B`, `A @ B` (multiplication matricielle), `A'` (transpose), `DIM(A)` (dimensions), `NROW`/`NCOL` (Opus, moyen)
- [x] M28a.2 — structures de contrôle + fonctions statistiques : `IF cond THEN ... ; ELSE ... ;`, `DO i = ... TO ...; ... END;`, `DO WHILE/UNTIL (cond); ... END;`, `PRINT`, `MEAN`, `SUM`, `STD`, `MIN`, `MAX` (Opus, moyen)
- [x] M28a.3 — algèbre linéaire et décompositions : `SOLVE(A, b)` (système linéaire), `INV(A)` (inversion), `EIGVAL(A)` (valeurs propres, symétrique), `CHOL(A)` (Cholesky, retourne U upper), `CALL QR(Q, R, A)` (factorisation QR), `CALL SVDCD(U, D, V, A)` (SVD via ATA-Jacobi). Différés : `EIGVEC`, `DET`, `CALL EIGEN` (Fable, élevé)
- [x] M28a.4 — I/O et persistance : `CREATE outds FROM X[COLNAME=]` (écriture matrice → dataset), `APPEND FROM Y` (ajout), `CLOSE outds`, `USE` + `READ ALL VAR {..} INTO mat` (lecture). Différés : `READ NEXT`, `WHERE`, `LOAD`/`STORE`/`SHOW` (Sonnet, moyen)
- [x] Fixtures `m28a/` + snapshots. DoD

## PHASE D — graphiques (images via `plotters`, dépend de M22)

## M29 — ODS GRAPHICS + PROC SGPLOT
- [x] M29.1 — infra ODS GRAPHICS : struct `OdsGraphics` (enabled/width/height/imagefmt/output_dir/file_stem) sur la Session ; statement `ODS GRAPHICS [ON|OFF] [/ WIDTH= HEIGHT= IMAGEFMT=(PNG|SVG) IMAGENAME= RESET=]` parsé et exécuté (NOTE par-statement : les dimensions ne s'affichent que si fournies dans CE statement). Moteur de rendu `graphics::render::draw_to_file` (plotters, PNG+SVG, backend ttf) sous `#[cfg(feature = "graphics")]` — SCATTER/SERIES/VBAR/HISTOGRAM minimalistes, ne panique jamais (données vides OK). **Feature `graphics` OFF par défaut : build par défaut byte-identique** (aucune image, juste les NOTE). Snapshot `m29/ods_graphics_basic` = log/listing (PAS d'octets image) ; assertion existence + taille>0 dans les tests de rendu. **Différé M29.2** : les NOTE au moment du RENDU ("image output deferred" sans feature / "Output 'file.png' (WxH) written to ..." avec feature) seront câblées au site d'appel de PROC SGPLOT — `draw_to_file` renvoie déjà `(w,h)` comme point d'ancrage. (Fable, élevé). **M29.1 TERMINÉ.**
- [x] M29.2 — `PROC SGPLOT` (`src/procs/sgplot.rs`, câblé dans `procs/mod.rs`) : parser tolérant pour SCATTER, SERIES, VBAR/HBAR (RESPONSE=/STAT=FREQ|SUM|MEAN), HISTOGRAM (BINWIDTH=/SCALE=), DENSITY, VBOX (CATEGORY=), REG (DEGREE=), LOESS (SMOOTH=), XAXIS/YAXIS (LABEL=/VALUES=(min to max by step)/TYPE=LINEAR|LOG|DISCRETE), BY ; options MARKERATTRS/LINEATTRS parsées puis ignorées (v1). **Exécution gradée par état** : (1) `ods_graphics.enabled==false` → NOTE "ODS GRAPHICS is not enabled…", EXIT 0 ; (2) BY/LOESS/DENSITY → NOTE de déférence dédiée (AVANT le gate feature → testable build par défaut) ; (3) build par défaut → NOTE "ODS GRAPHICS: image deferred (compile with --features graphics)." ; (4) `--features graphics` → image PNG/SVG matérialisée via `graphics::render::draw_to_file`, nommage séquentiel `{stem}_{N}.{ext}` (stem = IMAGENAME= sinon `sgplot`, compteur `Session.graphics_image_count`), NOTE "Output 'sgplot_1.png' (800x600) written." ; v1 = premier plot stmt seulement (NOTE si plusieurs) ; HBAR/VBOX/REG sous graphics → NOTE différée (render.rs ne gère que SCATTER/SERIES/VBAR/HISTOGRAM) ; colonne absente/non numérique → ERROR propre. 21 tests unitaires (19 build par défaut, +3 graphics-only −1 no-feature-only), fixture `m29/sgplot_basic.sas` + snapshot (build par défaut), harnais snapshot saute les fixtures `sgplot*` sous `--features graphics` (image réelle → log divergent). **Invariant 0 warning des deux builds** : champs AST graphics-only annotés `#[cfg_attr(not(feature="graphics"), allow(dead_code))]`. (Opus, élevé). **M29.2 TERMINÉ.**
- [x] M29.3 — branchement des plots UNIVARIATE (HISTOGRAM/QQPLOT) et REG (diagnostics) sur l'infra image (Opus, moyen). **UNIVARIATE** : l'AST porte désormais `plots: Vec<UnivariatePlot>` (kind {HISTOGRAM, QQPLOT, PROBPLOT, CDFPLOT, PPPLOT} + variable cible) au lieu d'un simple compteur ; le parser capture le type ET la variable (1er identifiant après le mot-clé) avant `skip_to_semi`. **Exécution gradée par état** : (1) `ods_graphics.enabled==false` → NOTE unique "HISTOGRAM/QQPLOT: graphical output deferred to ODS GRAPHICS (M29)." (byte-identique à l'avant-M29.3) ; (2) `enabled==true` + build par défaut → une NOTE "ODS GRAPHICS: image deferred (compile with --features graphics)." PAR plot ; (3) `enabled==true` + `--features graphics` → image par plot : HISTOGRAM → `PlotType::Histogram { bins: 10 }` (axe x = données de la variable) ; QQPLOT → `PlotType::Scatter` des quantiles empiriques (données triées) vs théoriques normaux `phi_inv((i-0.375)/(n+0.25))` ; PROBPLOT/CDFPLOT/PPPLOT → NOTE de déférence dédiée "{KW}: plot deferred in PROC UNIVARIATE." (avant le gate feature → testable build par défaut). Nommage séquentiel `{stem}_{N}.{ext}` (stem = IMAGENAME= sinon `univar`, compteur partagé `Session.graphics_image_count`), NOTE "Output 'univar_1.png' (800x600) written.". **REG** : statement `PLOTS ...;` parsé (corps sauté) → flag `plots_requested` → NOTE "PLOTS options deferred in PROC REG." ; diagnostic AUTOMATIQUE résidus-vs-prédites après chaque MODEL, piloté par `ods_graphics.enabled` (sans statement PLOTS) → build par défaut NOTE "REG diagnostics: image deferred (compile with --features graphics)." / `--features graphics` → `PlotType::Scatter` (x=predicted, y=residual), `reg_{N}.png`. **Invariant 0 warning des deux builds** : code de rendu sous `#[cfg(feature="graphics")]`, helpers `let _ = …;` pour l'argument inutilisé en build par défaut, AST graphics-only inchangé (univariate.rs porte déjà `#![allow(dead_code, unused_variables)]` ; reg.rs garde la NOTE PLOTS et l'aiguillage `enabled` ungated). +9 tests unitaires (UNIVARIATE : déférence sans ODS, "image deferred" HISTOGRAM/QQPLOT sans feature, PROBPLOT différé, +2 graphics-only création image ; REG : parse PLOTS, pas de diagnostic sans ODS, "image deferred" sans feature, +1 graphics-only). (Opus, moyen). **M29.3 TERMINÉ.**
- [x] Fixtures `m29/` + snapshots. DoD : fixture `m29/sgplot_univar_reg.sas` (UNIVARIATE HISTOGRAM sans ODS, puis HISTOGRAM+QQPLOT sous ODS, puis PROC REG intercept-only `model x =` sous ODS) + snapshot build par défaut (log : 1 déférence sans ODS, 2 "image deferred" UNIVARIATE, 1 "REG diagnostics: image deferred", EXIT 0 ; listing : rapports UNIVARIATE complets car NOPRINT est ignoré, ANOVA REG intercept-only avec F/Pr>F = NaN attendu pour model_df=0). Préfixe `sgplot*` → harnais snapshot saute la fixture sous `--features graphics` (image réelle → log divergent). 2241 tests build par défaut + 2246 sous `--features graphics`, 0 warning nouveau. **M29 TERMINÉ.**

## M30 — graphiques legacy
- [x] ⫽ M30.1 — `PROC GPLOT` (`src/procs/gplot.rs`) : statement `PLOT y*x;` / `PLOT y*x=group;` / `PLOT (y1 y2)*x;` ; exécution gradée ODS (not-enabled→NOTE / default→NOTE "image deferred" / --features graphics→`gplot_{N}.png` via Scatter). `PROC GCHART` (`src/procs/gchart.rs`) : `VBAR`/`HBAR` (FREQ/SUM/MEAN via SUMVAR= / TYPE=) → VBar render ; `PIE` → NOTE "PIE chart deferred". SYMBOL/AXIS statements parsés + NOTE. 14 tests unitaires. (Opus, moyen-élevé). **M30.1 TERMINÉ.**
- [x] ⫽ M30.2 — `PROC PLOT` (`src/procs/plot.rs`) : rendu ASCII scatter dans le listing (grille 20×60, symboles A/B/C pour superpositions, axes étiquetés min/max) quand ODS GRAPHICS OFF ; délègue à image (`plot_{N}.png` via Scatter) quand ODS GRAPHICS ON. Parser : `plot y*x;` / `plot y*x='sym';` / `plot (y1 y2)*x;` / `plot y*x=group;`. 7 tests unitaires. (Sonnet, moyen). **M30.2 TERMINÉ.**
- [x] Fixtures `m30/` + snapshots. DoD : `m30/gplot_gchart.sas` (GPLOT not-enabled + ODS ON deferred ; GCHART VBar deferred + PIE deferred, EXIT 0) ; `m30/proc_plot.sas` (PLOT ASCII listing rendu 6 points + ODS ON deferred, EXIT 0). 2262 tests build par défaut + --features graphics verts. **M30 TERMINÉ. Couverture cible atteinte.**


## M31 — refactor #1 : couche de parsing PROC partagée (pur, octet-identique)
Phase E. Refactorisation fonctionnelle : extraire des combinateurs de parsing réutilisables
dans `src/procs/common.rs` et y migrer les ~40 procs. **Pur — aucune sortie ne change :
zéro `.snap.new` à CHAQUE commit, `cargo test -p sasrs` vert, 0 warning.** Commits d'extraction
« move-only ». Design détaillé : voir PLAN.md §Phase E / M31.

Garde-fou byte-identité : `unknown_option_error` reproduit EXACTEMENT le message+span actuels
(`"Unexpected option '{BAD}' on PROC {NAME} statement."`) ; les procs au message divergent
(ex. `means` « Unexpected token… ») gardent leur branche inline dans la closure.

- [x] M31.1 — combinateurs additifs dans `src/procs/common.rs` (aucun appelant encore) :
  `parse_proc_options(ts, proc, FnMut(&mut StatementStream,&str)->Result<bool>)`,
  `parse_proc_body(ts, FnMut(...))` (skip semis, stop run/quit, recovery `skip_to_semi`),
  `expect_eq`, `parse_dataset_opt`/`parse_out_opt`, `unknown_option_error(ts, proc)`,
  `resolve_last_dataset` (décodage `data=`/`_LAST_`). + tests unitaires des combinateurs (Opus, moyen).
  **FAIT** : 7 combinateurs `#[allow(dead_code)]` (additif pur, aucun appelant) reproduisant
  verbatim les boucles de `print.rs` + `expect_eq`/`_LAST_` ; 14 tests (`parsing_tests`).
  2276 lib passés, 0 `.snap.new` (octet-identique), 0 warning nouveau.
- [x] M31.2 — relocaliser dans `common.rs` : `parse_by` (ex-`means::parse_by_list`),
  `parse_single_var`/`parse_weight`/`parse_class`/`parse_var_list` ; re-export `pub(crate)`
  depuis `means` pour les appelants existants (Sonnet, faible).
  **FAIT** : `parse_single_var` + `parse_by` déplacés VERBATIM dans `common.rs` ;
  `parse_weight`/`parse_var_list`/`parse_class` ajoutés (génériques, `parse_var_list` =
  `parse_name_list()`+`expect_semi()` comme `print.rs`) ; `means.rs` ré-exporte
  `parse_by as parse_by_list` + `parse_single_var` (`univariate`/`rank` inchangés).
  2276 lib verts, 0 `.snap.new`, 0 warning nouveau.
- [x] ⫽ M31.3 — migrer `src/procs/print.rs` (canari) sur les combinateurs (Sonnet, faible).
  **FAIT** : boucles d'options/corps → `parse_proc_options`/`parse_proc_body` (closures),
  `data=`→`parse_dataset_opt`, `var`→`parse_var_list`, résolution `_LAST_`→`resolve_last_dataset` ;
  import `TokenKind` retiré (inutile). −59 lignes nettes. 2276 lib, 0 `.snap.new`, 0 warning.
- [x] ⫽ M31.4 — migrer `src/procs/sort.rs` (Sonnet, faible). **FAIT** : options→`parse_proc_options` (data/out/nodupkey/noduprecs|nodup), BY→`parse_by`, `resolve_input`→`resolve_last_dataset` ; `expect_eq`/`resolve_input` locaux + import `TokenKind` supprimés (−~120 lignes). 2276 lib, 0 `.snap.new`, 0 warning.
- [x] ⫽ M31.5 — migrer Tier B : `contents`, `transpose`, `append`, `rank`, `printto`,
  `options`, `catalog` — un proc par commit (Sonnet, faible).
  **FAIT (6/7)** : `contents`/`transpose`/`append` migrés (options + corps + `resolve_last_dataset`,
  `expect_eq`/`resolve_input`/`TokenKind` locaux supprimés) ; `rank`/`printto`/`options` migrés
  **corps seulement** (leurs boucles d'options divergent : message « Unexpected token… » pour rank,
  reset bare-`;`/skip pour printto, collecte d'idents pour options → gardées inline, byte-identité).
  **`catalog` exclu (par conception)** : `run;` y est un *no-op continue* (pas un stop) et les deux
  boucles sautent les tokens inconnus → aucune cible byte-identique via les combinateurs ; laissé tel quel.
  −212 lignes nettes. 2276 lib, 0 `.snap.new`, 0 warning nouveau.
- [x] M31.6 — migrer Tier C : `means`, `freq`, `univariate`, `corr`, `ttest`, `npar1way` —
  un proc par commit, branches d'erreur bespoke gardées inline (Opus, moyen).
  **FAIT (6/6)** : boucles de **corps** → `parse_proc_body` (CLASS/VAR/WEIGHT/BY via
  `parse_class`/`parse_var_list`/`parse_weight`/`parse_by` ; OUTPUT/PAIRED/TABLES/graphics/`/`-clause
  gardés inline dans la closure) ; `resolve_input`→`resolve_last_dataset`. **Boucles d'options
  gardées inline** pour les 6 (messages bespoke « Unexpected token… »/« Unknown PROC … option »
  ou skip indulgent d'univariate). −184 lignes nettes. 2276 lib, 0 `.snap.new`, 0 warning nouveau.
- [x] M31.7 — migrer Tier D : `reg`, `glm`, `anova`, `genmod`, `logistic`, `mixed`, `glimmix`,
  `factor`, `princomp`, `discrim`, `cluster`, `fastclus`, `distance`, `report`, `tabulate`,
  `sgplot`, `iml` — boucle d'options + `DATA=`/`OUT=` + erreur seulement, un proc par commit (Opus, moyen).
  **FAIT (16/17)** : `resolve_input`→`resolve_last_dataset`, `DATA=`/`OUT=`→`parse_dataset_opt`/`parse_out_opt`,
  boucles de **corps**→`parse_proc_body` (statements MODEL/CLASS/RANDOM/COLUMN/DEFINE/TABLE/SCATTER…
  gardés inline dans la closure). `report` : boucle d'options **complète**→`parse_proc_options`
  (message canonique) ; les 15 autres gardent leur boucle d'options inline (skip indulgent ou
  « Unexpected token… ») mais migrent `DATA=`/`OUT=`. `report` body gardé inline (statement inconnu =
  ERREUR, pas `skip_to_semi`). **`iml` exclu** (sous-langage matriciel autonome : pas de `data=`/loops
  standard). −656 lignes nettes. 2276 lib, 0 `.snap.new`, 0 warning nouveau.
- [x] ⫽ M31.8 — balayage : remplacer les copies privées `expect_eq`/`resolve_input` par
  `common::*`, supprimer les doublons morts ; un proc par commit (Sonnet, faible).
  **FAIT** : 4/4 `resolve_input` supprimés (gplot/gchart/plot/export → `resolve_last_dataset`) ;
  14 `expect_eq` locaux entièrement retirés (sites `ts.next(); expect_eq` → `common::expect_eq`) ;
  4 partiels (export/import/means/sgplot : `expect_eq` gardé pour des sites à `ts.next()` partagé
  hors `match`, sites canoniques convertis). 20 fichiers, −313 lignes nettes. 2276 lib, 0 `.snap.new`, 0 warning.
- [x] DoD M31 : `cargo test -p sasrs` vert, **zéro `.snap.new`** (fixtures m1–m30 octet-identiques),
  `cargo build` 0 warning ; mettre les fichiers refactorés à jour dans la table PLAN.md ;
  passer « Jalon courant : **M32** ».
  **M31 TERMINÉ** : couche de parsing PROC partagée (`common.rs`) + migration de ~30 procs
  (Tier B/C/D) + balayage. Bilan : ~−1500 lignes de boilerplate dupliqué supprimées, 2276 tests
  verts, sortie octet-identique (0 `.snap.new`) du début à la fin. Restent inline par conception :
  boucles d'options à message bespoke, `iml` (sous-langage), `catalog` (`run;` no-op), `report` body.

## M32 — refactor #2 : scission `preprocess.rs` → `src/macros/` (pur, octet-identique)
Phase E. Scinder le monolithe `src/preprocess.rs` (~4757 lignes) en module `src/macros/`.
Façade publique conservée (`preprocess` reste un re-export → 3 sites d'import inchangés :
`lib.rs`, `sql::dictionary`, `session`). Surface stable : `MacroEngine::new`, `expand_open_code`
(fast-path identité **laissé intact dans `mod.rs`**), accesseurs `set_/get_/take_*`, trait
`TextStage`. Déplacements verbatim (seul `Self::foo`→`module::foo` change), **zéro `.snap.new`
à chaque commit**. Design : voir PLAN.md §Phase E / M32.

- [x] M32.1 — créer `src/macros/mod.rs` (struct `MacroEngine` + façade publique + `expand_open_code`
  + impl `TextStage`) ; `src/preprocess.rs` devient le shim de re-export. Build inchangé (Opus, moyen).
  **FAIT** : `git mv src/preprocess.rs src/macros/mod.rs` (contenu verbatim), `pub mod macros;` dans
  lib.rs, nouveau `src/preprocess.rs` = `pub use crate::macros::{TextStage, MacroEngine, MacroDef,
  MacroParam, MacroError, MacroStage, RawSegmenter};`. Sites d'import (`session`/`executor`) inchangés.
  2276 lib, 0 `.snap.new`, 0 warning. (`mod.rs` reste monolithique ; découpage en M32.2+.)
- [x] ⫽ M32.2 — extraire `src/macros/error.rs` (`MacroError`, `emit_error`) (Sonnet, faible).
  **FAIT** : `MacroError` + `impl`/`Display` + `emit_error` déplacés ; `pub use error::MacroError;`
  dans mod.rs (shim préservé). Technique blocs `impl MacroEngine` en sous-module → 0 changement d'appel.
- [x] ⫽ M32.3 — extraire `src/macros/scan.rs` (helpers char libres : `read_name`,
  `read_balanced_parens`, `find_kw`, `split_top_level_commas`, …) (Sonnet, moyen).
  **FAIT** : 13 scanners purs `&[char]`/index déplacés (verbatim, `pub(super)`) ; sites `Self::` inchangés.
  Laissés inline : `read_name_arg` + tous les `consume_*` (stateful `&mut self`).
- [x] ⫽ M32.4 — extraire `src/macros/symbols.rs` (`lookup`/`assign`/`resolve_value`/
  `resolve_refs_once`/pile de portées/auto-vars) (Sonnet, moyen).
  **FAIT** : `set_symbol_global`/`get_symbol`/`global_symbols`/`symbols_snapshot` (pub) +
  `lookup`/`assign`/`resolve_value`/`symbolgen_trace`/`resolve_refs_once`/`seed_automatic_vars`
  (`pub(super)`) déplacés. **Lot ⫽ M32.2-4** : `mod.rs` 4757→4227 (−530). 2276 lib, 0 `.snap.new`, 0 warning.
- [x] M32.5 — extraire `src/macros/quoting.rs` (sentinelles + `mask_char`/`unmask`) PUIS introduire
  `apply_quoting(text, mask_set, reevaluate)` et y router `%str/%nrstr/%bquote/%nrbquote/%superq`
  + variantes `%q*` (commit de fusion dédié, validé par les tests quoting/superq/bquote) (Opus, élevé).
  **FAIT** : PART 1 move (`MASK_BASE`/`STR_MASKED`/`NRSTR_EXTRA` + `mask_char`/`mask_special`/`unmask`)
  byte-identique ; PART 2 fusion `apply_quoting(eng, text, MaskSet, reevaluate)` = `mask_special` (réutilise
  `mask_char` → sentinelles bit-identiques) + `process_impl` si reevaluate. 7 points d'entrée routés
  (%str/%nrstr/%bquote/%nrbquote/%superq/%q*/%qcmpres) ; ordre de sondage intact. 32 tests quoting +
  snapshots m11/m12 octet-identiques. `mod.rs` 4227→4153. 2276 lib, 0 `.snap.new`, 0 warning.
- [x] M32.6 — extraire `src/macros/eval.rs` ; **unifier `tokenize_eval`** (un tokenizer → évaluateurs
  entier `%eval` ET flottant `%sysevalf` séparés). Garde : test division int vs float (Opus, élevé).
  **FAIT** : `tokenize_eval` était **déjà partagé** (`macro_eval`/EvalParser ET `eval_float`/FloatParser
  l'appellent) → unification déjà acquise, M32.6 = **move pur**. Déplacés dans `eval.rs` : `EvalTok`,
  `EvalParser`+`impl` (`ipow`), `FloatParser`+`impl`, et `impl MacroEngine` { `tokenize_eval`, `macro_eval`,
  `eval_float`, `eval_condition`, `eval_condition_int`, `format_float`, `format_sysevalf` }. Consumers
  `consume_eval`/`consume_sysevalf` laissés en `mod.rs`. 0 changement d'appel. Garde
  `sysevalf_vs_eval_integer_division` verte (%eval(7/2)=3, %sysevalf(7/2)=3.5). `mod.rs` 4153→3458.
  2276 lib, 0 `.snap.new`, 0 warning.
- [x] M32.7 — extraire `src/macros/functions.rs` + registre `functions::lookup` (string-fns +
  `%sysfunc`/whitelist) ; remplacer le `match` géant et la table inline de `process_impl`
  (même ordre de sondage `%q*` avant nu) (Opus, élevé).
  **FAIT** : move de `consume_sysfunc`/`SYSFUNC_WHITELIST`/`value_to_text`/`consume_cmpres`/
  `compress_blanks`/`read_name_arg`/`consume_symexist`/`sysmexist`/`sysget`/`consume_macro_fn`/
  `eval_macro_fn` vers `functions.rs`. Registre `STRING_FNS: &[MacroFn{name, eval: fn(&[String])->Option<String>}]`
  + `lookup(name)` remplace le `match` géant (entrées par nom logique q-strippé : upcase/lowcase/substr/
  scan/index/length, logique d'arm reproduite à l'octet, quirk `%length("")`→0 préservé). Masquage `%q*`
  reste piloté par le bool par-appel du dispatcher (pas de champ `masked` → zéro dead-code). Ordre de
  sondage intact. `mod.rs` 3458→3159. 2276 lib, 0 `.snap.new`, 0 warning.
- [x] ⫽ M32.8 — extraire `src/macros/control.rs` (`consume_if`/`consume_do`/itératif/conditionnel),
  `src/macros/define.rs` (`consume_macro_def`/invocation/params) et `src/macros/include.rs`
  (`%include`/autocall/`%put`/CALL EXECUTE) — déplacements verbatim, un fichier par commit (Sonnet, moyen).
  **FAIT (lot ⫽)** : control.rs (`consume_if`/`consume_do`/`consume_iterative_do`/`consume_conditional_do`/
  `set_loop_var` + `MAX_LOOP_ITERS`) ; define.rs (`MacroDef`/`MacroParam` **publics → `pub use define::{…}`**,
  `consume_macro_def`/`parse_param_list`/`consume_scope_decl`/`expand_invocation`/`parse_arg_list`/
  `bind_params`/`consume_macro_call` + `MAX_MACRO_DEPTH`) ; include.rs (`consume_include`/`try_autocall`/
  `consume_put` + `MAX_INCLUDE_DEPTH`). 0 changement d'appel. `mod.rs` 3159→2130. 2276 lib, 0 `.snap.new`, 0 warning.
- [x] M32.9 — extraire `src/macros/expand.rs` (boucle `process_impl`) en dernier ;
  état final : `mod.rs` = façade seule (Opus, moyen).
  **FAIT** : `process_impl` (+ indirection `&`/`&&var`) + `consume_let`/`consume_eval`/`eval_sysfunc`/
  `consume_quote`/`consume_unquote`/`consume_sysevalf`/`consume_superq`/`consume_bquote` déplacés dans
  `expand.rs` (`pub(super)`, 0 changement d'appel). `mod.rs` (1497) = façade : `TextStage`+impl, struct
  `MacroEngine`, `new`/`log_line`/`expand_open_code`, `RawSegmenter`, tests. 2276 lib, 0 `.snap.new`, 0 warning.
- [x] DoD M32 : `cargo test -p sasrs` vert, **zéro `.snap.new`**, 0 warning ; PLAN.md §Macro pointant
  sur `src/macros/` ; passer « Jalon courant : **M33** ».
  **M32 TERMINÉ** : `preprocess.rs` (4757 l.) → module `src/macros/` de 12 fichiers (mod façade 1497 +
  error/scan/symbols/quoting/eval/functions/control/define/include/expand) + shim `preprocess`.
  2 fusions de généralisation (apply_quoting, registre lookup) ; tokenize_eval déjà partagé. Sortie
  octet-identique de bout en bout.

## M33 — complétion options : PROCs Base & descriptifs
Phase E. Vider au maximum la colonne droite « non couvert » des tableaux README des procs
descriptifs/gestion. Une case = un proc / un lot d'options cohérent ; fixtures
`tests/fixtures/m33/` + snapshot vérifié à la main ; mise à jour README (🟡→✅ quand complété).
Pattern : nouvelle branche dans la closure `parse_proc_options`/`parse_proc_body` (combinateurs M31),
exécution correspondante, NOTE/ERROR propres pour le résiduel différé.

- [x] M33.1 — `PROC FREQ` : `BY`, `WEIGHT`, `/LIST`, tables ≥3 voies, Fisher r×c (n×m), CHISQ 1 voie (Opus, élevé).
  **FAIT (4/5 + CHISQ 1-voie déjà existant)** : `WEIGHT` (fréq = somme des poids, propagé dans Percent/
  Row/Col Pct + CHISQ Pearson/LR + chi² 1-voie ; exclusion missing/≤0), `BY` (`common::by_groups`/
  `resolve_by_cols`), `/LIST` (1 ligne/cellule + cumulés), **n-voies** `a*b*c` (two-way stratifié,
  « Controlling for … »). **Fisher r×c (>2×2) différé** (note propre, reste en README non-couvert).
  Fréquences en f64 → chemin non pondéré byte-identique (m5/m10 inchangés). Fixture
  `tests/fixtures/m33/freq_options.sas` + snapshot vérifié vs sashelp.class (pondéré 18/20, BY F 4/5 M 5/5,
  LIST cum 4/9/14/19, 3-voies). +9 tests (2285 lib). README FREQ : 🟡 (reste Fisher >2×2).
- [x] M33.2 — `PROC UNIVARIATE` : rendu `PROBPLOT`/`CDFPLOT`/`PPPLOT`, quantiles & extrêmes pondérés (Opus, moyen).
  **FAIT** : quantiles pondérés `weighted_quantile_def5` (analogue pondéré de la déf. 5 : W=Σwᵢ, cible
  t=p·W, 1er i avec Wᵢ≥t ; Wᵢ==t → moyenne x(i),x(i+1) ; se réduit exactement à la déf. 5 si poids=1) →
  table Quantiles + Median/Q1/Q3/Range/IQR pondérés (note « non calculé » retirée) ; extrêmes affichés sous
  WEIGHT (valeurs brutes). `PROBPLOT`/`CDFPLOT`/`PPPLOT` câblés sur l'infra image M29.3 (ODS off → note
  unique inchangée ; build défaut → « image deferred » par plot ; `--features graphics` → `univar_{N}`).
  Fixture `tests/fixtures/m33/univariate_weighted.sas` + snapshot vérifié (x=1..4,w=1..4 : Méd 3, Q1 2, Q3 4,
  10%=1.5). +3 tests (2288 lib). README UNIVARIATE : reste 🟡 (skew/kurt pondérés omis). Aucun snapshot existant déplacé.
- [x] M33.3 — `PROC MEANS`/`SUMMARY` : `WAYS`, `TYPES`, `PRINTALLTYPES`, mots-clés percentiles (P1..P99/QRANGE) (Opus, moyen).
  **FAIT** : percentiles `P1..P99`/`Q1`/`Q3`/`QRANGE` via `quantile_def5` de UNIVARIATE (passé `pub(crate)`,
  réutilisé sans copie) — table imprimée + OUTPUT OUT= + ODS ; `WAYS n`/`TYPES (..)` → masques `_TYPE_`
  autorisés (`allowed_types`, pilote table ET OUT=) ; `PRINTALLTYPES` imprime chaque `_TYPE_` sélectionné
  (`emit_report_type`), défaut = highest `_TYPE_` seul (inchangé). Percentiles pondérés = non pondérés
  (simplif. documentée, cohérent MEDIAN existant). Fixture `tests/fixtures/m33/means_options.sas` + snapshot
  vérifié vs sashelp.class (height P25=57.5/Méd=62.8/P75=66.5/P95=72/QRANGE=9). +11 tests (2299 lib).
  README MEANS : reste 🟡 (MAXDEC=/NWAY/MISSING/ORDER=/ID).
- [x] M33.4 — `PROC TABULATE` : `OUT=`, `FORMAT=`/labels d'en-tête, dénominateurs `PCTN<...>`, 4ᵉ dimension (Fable/Opus, élevé).
  **FAIT** : labels d'en-tête (`name='label'`/`stat='label'` + LABEL stocké via VarMeta), `format=`
  (niveau table) + `*f=<fmt>` (par cellule) via `src/formats`, `OUT=lib.ds` (1 obs/cellule :
  `<class…> _TYPE_ _PAGE_ _TABLE_ <var>_<Stat>`, réutilise le pattern `means.rs`). **`PCTN<...>`/`PCTSUM<...>`
  différés** (note propre, reste README non-couvert). **4ᵉ dim** : SAS plafonne à 3 dims → l'erreur est le
  comportement SAS correct (retirée du non-couvert, message reformulé). Fixture + snapshot vérifié vs
  sashelp.class (mean F=60.59/M=63.91, weight sum F=811.0/M=1089.5, OUT= 2 obs). +7 tests (2306 lib).
  README TABULATE : reste 🟡 (PCTN<...>).
- [x] M33.5 — `PROC REPORT` : `DEFINE FORMAT=/WIDTH=/FLOW/SPACING`, `COMPUTE` complexe (`_Cn_`, fonctions, formats) (Fable/Opus, élevé).
  **FAIT** : `DEFINE / FORMAT=` (via `src/formats`, comme TABULATE M33.4), `WIDTH=` (troncature/padding,
  num droite/char gauche), `SPACING=` (espaces avant colonne, défaut 2 ; rendu par `write_table_layout`
  activé seulement si WIDTH=/SPACING= présents) ; `COMPUTE` avec réfs colonnes `_Cn_`/nommées
  (`compute_row_context`) + `LINE @col fmt.` (pointeur + format). **`FLOW` différé** (interaction
  wrap/hauteur de ligne, erreur propre) ; COMPUTE « riche » (réassignation via toute la lib de fonctions)
  partiel. Fixture + snapshot vérifié vs sashelp.class (height 60.59/63.91, weight WIDTH=10/SPACING=5,
  ratio `_c3_/_c2_`=1.487/1.705). +8 tests (2314 lib). README REPORT : reste 🟡 (FLOW, COMPUTE riche).
- [x] ⫽ M33.6 — `PROC PRINT` : `BY`, `ID`, `SUM`, `DOUBLE`/`N` (Sonnet, moyen).
  **FAIT (→ ✅)** : `BY` (sections par groupe, `common::by_groups`), `ID` (remplace `Obs`), `SUM`
  (sous-totaux par groupe BY + grand total), `DOUBLE`, `N`. `write_table_ext` ajouté (chemin sans option
  byte-identique). Oracles vérifiés (totaux 1184.4/1900.5 ; F 545.3/811, M 639.1/1089.5).
- [x] ⫽ M33.7 — `PROC CONTENTS` : `OUT=`, `DETAILS`, `SHORT` (Sonnet, moyen).
  **FAIT (→ ✅)** : `OUT=` (1 ligne/var : NAME/TYPE 1=num·2=char/LENGTH/VARNUM/LABEL/FORMAT), `SHORT`
  (liste plate), `DETAILS`/`NODETAILS` (lignes # obs/var). OUT= n'éteint pas le rapport (= SAS). META 5 obs×6 vars.
- [x] ⫽ M33.8 — `PROC DATASETS` : `COPY`, `MODIFY`/`RENAME` var, `EXCHANGE`, `SAVE`, `CONTENTS` (Sonnet, moyen).
  **FAIT** : `COPY OUT= [IN=] [;SELECT]`, `EXCHANGE a=b`, `SAVE m…`, `MODIFY m; RENAME old=new; LABEL`
  (réutilise les ops `LibraryProvider`). `APPEND`/`REPAIR`/`CONTENTS`-interne différés. Reste 🟡.
  **Lot ⫽ M33.6-8** : fixtures + 3 snapshots vérifiés vs sashelp.class. +17 tests (2331 lib), 0 warning.
- [x] M33.9 — `PROC SORT` compléments (`TAGSORT`, `SORTSEQ=`, `KEY=`) ; `PROC APPEND` (`NOWARN`/`APPENDVER=`) (Sonnet, faible)
  **FAIT** : TAGSORT (no-op hint, accepté, ignoré), SORTSEQ=ASCII (no-op) / SORTSEQ=LINGUISTIC (fallback sas_cmp + NOTE),
  KEY=var [/ DESCENDING] (remplace BY si présent) ; NOWARN (supprime WARNING FORCE), APPENDVER=Vn (no-op).
  Fixture `tests/fixtures/m33/sort_append_options.sas` + snapshot vérifié à la main.
  +17 tests (2348 lib), 0 warning. README SORT/APPEND fully covered, colonnes « non couvert » vides.
- [x] DoD M33 : fixtures `m33/` + snapshots vérifiés ; README à jour (colonnes « non couvert » rétrécies) ;
  passer « Jalon courant : **M34** ».
  **M33 TERMINÉ** : 9 incréments de complétion sur les procs Base/descriptifs. FREQ (WEIGHT/BY//LIST/n-voies),
  UNIVARIATE (quantiles/extrêmes pondérés + plots), MEANS (percentiles/WAYS/TYPES/PRINTALLTYPES), TABULATE
  (labels/FORMAT=/OUT=), REPORT (FORMAT=/WIDTH=/SPACING=/_Cn_), PRINT (BY/ID/SUM/DOUBLE/N → ✅), CONTENTS
  (OUT=/SHORT/DETAILS → ✅), DATASETS (COPY/EXCHANGE/SAVE/MODIFY), SORT (TAGSORT/SORTSEQ=/KEY= → ✅), APPEND
  (NOWARN/APPENDVER= → ✅). 8 fixtures `m33/` + snapshots vérifiés à la main. 2276 → 2348 tests (+72), chemins
  par défaut byte-identiques, 0 warning nouveau. Différés documentés : Fisher r×c, PCTN<...>, REPORT FLOW.

## M34 — complétion options : PROCs statistiques & modélisation
Phase E. Compléter les options différées des procs stat/modélisation (colonnes README).
Oracles vérifiés vs SAS 9.4 documenté ; numérique fait maison (`src/stat/`). Fixtures
`tests/fixtures/m34/` + snapshots. Une case = un proc / un lot cohérent.

- [x] M34.1 — `PROC CORR` : corrélation partielle (`PARTIAL`), `HOEFFDING`, Spearman/Kendall pondérés (Opus, élevé).
  **FAIT** : (a) `PARTIAL` — résidualisation moindres carrés `stat::linalg::least_squares` sur `[1, vars
  partielles]`, listwise-complete, Pearson sur résidus, df = n−k−2 (r(height,weight|age)=0.70467 = SAS,
  p=0.0011) ; (b) `HOEFFDING` — D exact (≡ SAS sashelp.class : height×weight 0.31609, height×age 0.18856,
  weight×age 0.20579) + `Prob > D` (approximation asymptotique Blum-Kiefer-Rosenblatt/Imhof, n≥5) ;
  (c) WEIGHT étendu à Spearman (rangs moyens pondérés) & Kendall (paires pondérées wᵢ·wⱼ) — ≡ méthode
  ordinaire sur données répliquées (tests). 2 fixtures m34 (corr_options + corr_hoeffding) + snapshots
  vérifiés. **Différés (README non-couvert) : partial Spearman/Kendall, Prob>D tabulée exacte petit n.**
  +13 tests (2367 lib), 0 warning. (PARTIAL implémenté en direct pendant la panne de l'outil Agent.)
- [x] ⫽ M34.2 — `PROC TTEST` : `BY`, p unilatéral câblé (`SIDES=`), colonnes CI (Sonnet, moyen).
  **FAIT (→ ✅)** : `BY` (analyse par groupe via `common::by_groups`/`resolve_by_cols`, 1-sample/2-sample
  CLASS/PAIRED) ; `SIDES=U|L|2` câblé (en-tête `Pr > t`/`Pr < t`, `sided_p`) ; colonnes CI gated par `CI=`
  (chemin défaut byte-identique) — CL Mean (t) + CL Std (χ², `chisq_quantile` via `common::chisq_sf`),
  2-sample → Mean Diff + CL Diff. Fixture + snapshot vérifié vs sashelp.class (BY sex F t=0.352/M t=2.504 ;
  SIDES=U t=1.9867/Pr>t=0.0312 ; CI=95 Mean [59.866,64.808], Std [3.874,7.582]). +6 tests (2356 lib). README TTEST → ✅.
- [x] ⫽ M34.3 — `PROC NPAR1WAY` : `BY`, `OUT=`, scores Median/Savage/Normal, Wilcoxon exact (Opus, moyen).
  **FAIT (→ ✅)** : (a) `BY` (partition `common::by_groups`, en-têtes `name=value`, niveaux CLASS
  recalculés par groupe) ; (b) cadre générique de scores rang-linéaires (`raw_score`/`tie_averaged_scores`/
  `score_analysis`/`score_two_sample`/`score_one_way`) couvrant `MEDIAN`, `SAVAGE`, `NORMAL`/`VW` —
  table 2-échantillons (Statistic/Mean/Std/Z continuité-corrigé/Pr>|Z|) + analyse à un facteur χ²
  (df=k−1) ; Wilcoxon reste le cas rang (closed-form `analyze`, inchangé) ; (c) Wilcoxon `EXACT`
  (k=2) — distribution de permutation exacte par DP `dp[count][sum2]` sur rangs ×2, `Pr<=S` /
  `Pr>=|S−Mean|`, plafond `EXACT_N_CAP=30` (NOTE au-delà) ; (d) `OUTPUT OUT=` — 1 obs/VAR/groupe BY,
  colonnes `_WIL_/Z_WIL/P2_WIL/P1_WIL`, `XP1_WIL/XP2_WIL` (exact), `_KW_/DF_KW/P_KW`, et par score
  `_MED_/_SAV_/_VW_` (+`Z_*`/`P2_*`/`P_*`/`DF_*`). Oracles vérifiés (Wilcoxon ≡ m24 73/90/12.2367/−1.3484 ;
  χ² à un facteur ≡ Z₀² non corrigé ; exact 0.1754 = 2×0.0877). 2 fixtures m34 (scores + by_out) +
  snapshots. **Différés (README non-couvert) : exact pour Median/Savage/Normal (rang seul), colonnes BY
  OUT= stockées en chaîne formatée.** +9 tests (2376 lib), 0 warning, chemin défaut octet-identique.
- [ ] M34.4 — `PROC REG` : `NOINT`, `SELECTION=` (FORWARD/BACKWARD/STEPWISE), MODEL multiples (Opus, élevé)
- [ ] M34.5 — `PROC ANOVA` & `PROC GLM` : effets d'interaction (`a*b`), CLASS multiples (Fable, très élevé)
- [ ] M34.6 — `PROC LOGISTIC` : `CLASS` (codage référence/effet), `LINK=` (probit/cloglog),
  logistique ordinale/nominale, `OUTPUT OUT=` (Fable, très élevé)
- [ ] M34.7 — `PROC GENMOD` : `CLASS`, `DIST=GAMMA` (+ lien canonique), `SCALE=` (Opus, élevé)
- [ ] M34.8 — `PROC MIXED` & `PROC GLIMMIX` : `TYPE=AR(1)/UN`, `NOINT`, effets fixes CLASS/continus,
  `LINK=PROBIT/CLOGLOG`, `METHOD=LAPLACE` (GLIMMIX) (Fable, très élevé)
- [ ] ⫽ M34.9 — `PROC PRINCOMP`/`FACTOR`/`DISCRIM` : `OUT=` scoring (composantes/scores/classification) ;
  `FACTOR` rotations obliques (Opus, élevé)
- [ ] ⫽ M34.10 — `PROC CLUSTER` `OUTTREE=` ; `PROC IML` : `SHAPE`, `DET`, `CALL EIGEN`/`EIGVEC`,
  sous-scripts intervalle `a:b` (Opus, élevé)
- [ ] M34.11 — graphiques (sous `--features graphics`) : `SGPLOT` rendu `LOESS`/`DENSITY` réel,
  `GCHART` `PIE`, `GPLOT` PLOT multiples + `SYMBOL`/`AXIS` honorés (Opus, moyen)
- [ ] DoD M34 : fixtures `m34/` + snapshots vérifiés (oracles) ; README à jour ;
  passer « Jalon courant : **M35** ».

## M35 — macro : complétion totale
Phase E. Combler les derniers écarts macro pour un support intégral. Processeur toujours actif ;
invariant : snapshots m1–m34 octet-identiques (nouveau comportement seulement sur nouvelles
directives/fonctions). Fixtures `tests/fixtures/m35/`. Tableau « Macro language » du README → ✅.

- [ ] M35.1 — `%SYSFUNC`/`%QSYSFUNC` : remplacer la liste blanche (~18 fns) par une délégation à
  TOUTE la bibliothèque `datastep::functions::call` (typage args num/char, support `fmt.`),
  erreurs propres pour les fonctions réellement absentes (Opus, élevé)
- [ ] M35.2 — `%INCLUDE` : filerefs (`%include myref;`), chemins non quotés, `*`/stdin ;
  résolution via `FILENAME` (Opus, moyen)
- [ ] ⫽ M35.3 — conformité fine : `%LENGTH("")`→1, écarts documentés résorbés ; variables auto
  restantes (`&SYSPROCESSNAME`, `&SQLOBS`, `&SYSCC`, `&SYSERR`, `&SYSLAST`, …) (Sonnet, moyen)
- [ ] M35.4 — audit exhaustif macro : revue de chaque statement/fonction macro SAS
  (`%ABORT`, `%RETURN`, `%GOTO`/`%label`, `%SYSCALL`, `%SYSEXEC`, `%WINDOW`/`%DISPLAY`…) —
  implémenter le faisable, erreur propre + documentation pour le résiduel hors périmètre (Opus, élevé)
- [ ] DoD M35 : fixtures `m35/` + snapshots vérifiés ; tableau Macro README en ✅ (résiduel documenté) ;
  passer « Jalon courant : **TERMINÉ (Phase E)** ».
