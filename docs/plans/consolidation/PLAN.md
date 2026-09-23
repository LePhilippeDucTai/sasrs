# PLAN — consolidation

Goal: fiabiliser et industrialiser `sasrs` avant toute extension (issue #11 et sous-issues #5–#10) : plus aucun résultat silencieusement faux, erreurs et stockage fiables, validation indépendante arbitrée par une CI bloquante, parcours nouvel utilisateur réel, puis compatibilité SAS à forte valeur ; la suite avancée (ex-M46–M66) est replanifiée par comportement borné.
Base branch: consolidation · Remote: origin · Language: fr · Created: 2026-09-23 · Planned with: best/max

## Scope

- In: #5 divergences et contrats silencieux (codes retour, diagnostics, quantiles pondérés, « reconnu mais ignoré ») ; #6 stockage parquet + sidecar, métadonnées, contrat d'encodage ; #7 CI GitHub Actions bloquante, corpus de conformité, tests de propriétés et différentiels ; #8 façade Session, diagnostics structurés, Python, distribution versionnée, documentation ; #9 MEANS/SUMMARY, COMPARE, TRANSPOSE, PRINTTO, informats, ODS OUTPUT, XLSX, BY, étude SAS7BDAT/XPT ; #10 feuille de route avancée ; réconciliation de `PLAN.md`/`PROGRESS.md` racine (jalons M46–M66 remappés).
- Out: implémentation de LOGISTIC/GENMOD/GLM/ANOVA/MIXED/GLIMMIX/multivarié/IML/S3/graphiques au-delà du durcissement (→ `docs/roadmap/avancee.md`, J08-P5) ; TABULATE `PCTN<>`, REPORT FLOW/COMPUTE, DATASETS APPEND/REPAIR, CATALOG, OPTIONS détaillé (ex-M46–M49, M51 → même feuille de route) ; lecture/écriture SAS7BDAT/XPT (étude seulement) ; stockage autre que Parquet.

## Architecture

- Pipeline inchangé : source → `macros` (texte→texte) → `lexer` → `parser::StatementStream` → `executor` (DATA step, procs, SQL) ; état dans `Session` (libs, log, destination ODS).
- Contrat « reconnu mais ignoré » (J02-P3, `docs/support-contract.md`) : instruction ou option reconnue mais non honorée → ERROR si le résultat numérique ou un dataset peut changer, WARNING si l'effet est d'affichage seulement ; instruction inconnue → ERROR façon SAS 180-322 ; jamais de repli silencieux.
- Encodage (D-001) : `VarMeta.length` et toute troncature en caractères (session SAS LATIN1/WLATIN1) ; jamais d'UTF-8 invalide ; écart avec une session SAS UTF-8 documenté dans `docs/encoding.md`.
- Stockage : couple `<t>.parquet` + `<t>.parquet.sasmeta.json` écrit selon le protocole de `docs/adr/0001-*` (J04-P1) ; toute incohérence détectée à la lecture produit un diagnostic.
- Validation : `conformance/cases/<id>/` (programme, données, sorties attendues, provenance, tolérances, nature math/SAS) exécuté par `tests/conformance.rs` ; oracles externes (doc SAS publiée, script Python indépendant) écrits par une part distincte de celle qui implémente.
- API publique : module `sasrs::api` (J06-P1) au-dessus de `Session` ; `run()` et `RunOutcome` conservés ; diagnostics structurés et fichiers produits exposés.
- CI : `.github/workflows/ci.yml`, job agrégé `ci-ok` requis sur `main` ; `scripts/check.sh` = mêmes commandes en local ; `scripts/ci-status.sh` vérifie la CI du dernier commit poussé.

## Risks

- Tout changement de snapshot doit être justifié dans le message de commit (`Snapshot: <fixture> — <raison>`) ; un snapshot n'est jamais un oracle ; le durcissement change des sorties (ERROR au lieu d'ignorer, exit 1/2) : vérifier chaque fixture touchée.
- Pièges de `PLAN.md` racine §Checklist : `Value::sas_cmp`, `missing::nullify_specials`, jamais `get_row`, troncature char, pluriel invariable des NOTEs ; ordre de sommation du numérique maison (digits REG/MIXED).
- Watchdog Hermes externe (`rebuild-python-release.sh`, cron 5 min) : il pousse des commits `python wrapper: re-sync SHA-256` sur `main` et remplace l'asset en place ; il entre en conflit avec J06-P4/P5 et doit être coupé par l'utilisateur quand les releases versionnées existent (R-001).
- Builds `--features graphics` et `--features s3` hors `cargo test` par défaut : toujours dans Build/CI ; les fixtures qui produisent des images sont exclues du snapshot sous graphics.
- Hôte sans éditeur de liens C : toute commande cargo passe par le conteneur distrobox `dataflowrs-dev` ; compilation Polars longue.

## Checks

- Setup: none · marker: none
- Test: `cargo test -p sasrs` · Build: `cargo build --features graphics && cargo build --features s3` · Lint: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`
- Baseline 2026-09-23: red: `cargo test -p sasrs` vert (2 964 tests), fmt vert, build graphics et tests `--features s3 --lib library` verts ; lint rouge (clippy `-D warnings` : 52 `use super::super::*` inutilisés, rustc 1.98.1) ; `cargo test --features graphics` rouge (snapshot `m36/plots` non exclu). Réparé par J01-P1.

## Milestones

| Id  | Milestone | Parts | Depends on | Orchestrator | File |
|-----|-----------|-------|------------|--------------|------|
| J01 | Base verte, CI bloquante, harnais CLI, roadmap réconciliée | 6 | — | sonnet/high | jalons/J01-base-ci.md |
| J02 | Diagnostics fiables : codes retour, erreurs macro, contrat « reconnu mais ignoré » | 8 | J01 | sonnet/high | jalons/J02-diagnostics.md |
| J03 | Divergences numériques et encodage | 8 | J02 | sonnet/high | jalons/J03-divergences-encodage.md |
| J04 | Intégrité du stockage et des métadonnées | 6 | J03 | sonnet/high | jalons/J04-stockage.md |
| J05 | Validation industrielle : conformité, propriétés, différentiel | 7 | J04 | sonnet/high | jalons/J05-validation.md |
| J06 | Utilisabilité : API Session, Python, distribution, documentation | 8 | J05 | sonnet/high | jalons/J06-utilisabilite.md |
| J07 | Compatibilité à forte valeur (1) : MEANS, COMPARE, TRANSPOSE, PRINTTO, informats | 8 | J06 | sonnet/high | jalons/J07-compatibilite.md |
| J08 | XLSX, BY, ODS OUTPUT, feuille de route avancée, validation de bout en bout | 7 | J07 | best/max | jalons/J08-avance-e2e.md |

## Tiers

| Tier | Model  | Effort | Use for | Executor |
|------|--------|--------|---------|----------|
| T1 | best   | max    | planning, architecture, cross-cutting decisions, plan updates | orchestrator inline; philippele-skills:planner |
| T2 | opus   | xhigh  | complex refactor, tricky logic, concurrency, migrations, milestone review | philippele-skills:worker-t2 / reviewer |
| T3 | opus   | high   | standard implementation with design latitude | philippele-skills:worker-t3 |
| T4 | sonnet | high   | well-specified implementation, tests | philippele-skills:worker-t4 |
| T5 | sonnet | medium | mechanical changes, docs, renames, config | philippele-skills:worker-t5 |
| T6 | haiku  | low    | trivial edits, search, inventory | philippele-skills:worker-t6 |

## Conventions

- Writers: orchestrator → PROGRESS.md, DECISION.md, DECISIONS_LOG.md, LONG_TERM_RECO.md; planner → PLAN.md, jalons/; workers → source only.
- Every milestone ends with a `kind: review` part run by philippele-skills:reviewer; the reviewer also runs the Checks and reads Risks.
- Wave cap: 3 parts in flight (machine : Steam Deck, compilation Polars) ; review, size L, and T1 parts run alone.
- Git: workers commit in their worktree; one merge commit per validated part `<id>: <title>`; push consolidation to origin at session end; never force, never amend.
- Failed part: try 2 at the same tier with the failure evidence, try 3 one tier higher, then ❌ and a queued decision; a `blocked` report or a first merge conflict does not count.
- Plan updates: only when remaining parts are invalidated or the user asks for a change; planner rewrites files in place; ids never renumbered; a dropped part is ⏭ and its dependents get their Scope rewritten.
- Environnement : toute commande cargo (acceptations, Checks) s'exécute comme `distrobox enter dataflowrs-dev -- bash -lc 'cd <worktree> && <commande>'` avec `CARGO_TARGET_DIR=/home/deck/.cache/sasrs/target-<id de part>` (posé dans le brief, supprimé après le merge) ; `git`, `gh` et `python3` restent sur l'hôte ; commandes au premier plan, `timeout: 600000`, relancer telle quelle une compilation interrompue.
- Acceptations filtrées : la forme `… 2>&1 | grep -qE 'test result: ok\. [1-9]'` garantit qu'au moins un test a tourné ; les noms de tests imposés dans les Scopes sont contractuels.
- Fixtures nouvelles sous `tests/fixtures/j0N/<préfixe>_*.sas` (snapshots `tests/snapshots/snapshot__fixtures@j0N__<préfixe>_*`) ; modifier un snapshot existant exige la justification de commit ; deux parts parallèles qui modifient le même snapshot existant se résolvent comme un conflit de merge.
- Oracles : l'agent qui implémente n'écrit pas le seul oracle qui le valide ; valeurs attendues = doc SAS publiée citée (URL), sortie SAS réelle, ou script indépendant d'une part antérieure ; un cas `known-divergence` du corpus ne voit jamais ses valeurs attendues modifiées par l'implémenteur, seulement son statut.
- Couverture publique : `README.md` ne change que dans les parts « documentation » de chaque jalon ; états distingués : implémenté · validé contre référence · approximation documentée · non supporté (définis dans `CONTRIBUTING.md`, J01-P4).
- Revue à partir de J02 : `scripts/ci-status.sh consolidation` doit réussir (CI verte sur le dernier commit poussé).
