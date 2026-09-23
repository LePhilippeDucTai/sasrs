# J02 — Diagnostics fiables : codes retour, erreurs macro, contrat « reconnu mais ignoré »

Goal: un code retour et un log auxquels on peut se fier : échecs d'écriture et d'ODS comptés, erreurs macro réellement émises, `%ABORT` honoré, et aucune instruction, option ou valeur reconnue mais non respectée sans ERROR/WARNING explicite (issue #5).
Depends on: J01 · Orchestrator: sonnet/high

## J02-P0 — Corrective review J01 : durcissement de l'appareil de confiance CI

```yaml
id: J02-P0
kind: implement
tier: T5
size: S
depends_on: []
files:
  - scripts/ci-status.sh
  - scripts/check.sh
  - .github/workflows/ci.yml
  - scripts/check_ci_structure.py
  - .github/CODEOWNERS
acceptance:
  - "bash -n scripts/ci-status.sh && bash -n scripts/check.sh"
  - "grep -qE 'git (fetch origin|ls-remote)' scripts/ci-status.sh"
  - "test \"$(grep -c -- '--locked' .github/workflows/ci.yml)\" -ge 6 && grep -q -- '--locked' scripts/check.sh"
  - "python3 scripts/check_ci_structure.py"
  - "test -f .github/CODEOWNERS && grep -q '@LePhilippeDucTai' .github/CODEOWNERS"
  - "scripts/ci-status.sh --help | grep -q consolidation"
```

### Scope
- **F2** (`ci-status.sh`) : faire un `git fetch origin <branche>` (ou `git ls-remote` sur la ref) AVANT la comparaison headSha == sha local — la revue a démontré un faux vert (M4-A) sur refs locales périmées. Conserver tous les comportements fail-closed existants (sha ≠, conclusion failure, run in_progress). Corriger **F9** au passage : le séparateur statut/conclusion qui colle « aucune » (cas in_progress).
- **F4** : ajouter `--locked` à toute invocation cargo qui l'accepte dans `ci.yml` (les 6 lignes clippy/test ; PAS `cargo fmt --check`) et dans `scripts/check.sh` (clippy, test, build).
- **F5** : nouveau `scripts/check_ci_structure.py` (PyYAML, shebang python3) : parse `.github/workflows/ci.yml` et échoue (exit ≠ 0, message explicite) si `ci-ok` est absent, si un job de `needs` n'existe pas, ou si `ci-ok` porte un `continue-on-error`. Le job `ci-ok` de `ci.yml` reçoit un checkout `actions/checkout@v4` + un step `python3 scripts/check_ci_structure.py` AVANT la vérification d'agrégation (l'arbitre valide la structure du workflow avant de déclarer vert). Preuve par la négative obligatoire dans le rapport : sur une copie scratch, retirer un job de `needs` → le script doit sortir ≠ 0.
- **F1 (volet déposé dans le dépôt)** : `.github/CODEOWNERS` avec `@LePhilippeDucTai` sur `.github/workflows/`, `scripts/`. Interdit d'activer `required_pull_request_reviews` ou tout autre réglage de dépôt via gh api (décision sensible différée à l'utilisateur, D-003 — dépôt mono-auteur : l'auto-approbation est impossible, ça bloquerait toutes les PR).
- Interdits : toucher toute autre ligne de `ci.yml` ou `check.sh` (commandes, jobs, env) ; modifier `Cargo.toml`/`Cargo.lock` ; affaiblir un check existant.
- Note d'environnement (F10) : sur l'hôte, le `python3` de PATH (linuxbrew) n'a pas PyYAML — rejouer l'acceptation 4 avec `/usr/bin/python3` ; dans la CI, le python3 du runner l'a.

### Context
- Review J01-P6 try 2 (`/tmp/sasrs-j01p6-review.log`) : GO-avec-findings ; F1/F2 majeurs, F4/F5/F9 mineurs. D-003 dans `DECISIONS_LOG.md`.

## J02-P1 — Échecs d'écriture, code retour et pertes ODS

```yaml
id: J02-P1
kind: implement
tier: T3
size: M
depends_on: [J02-P0]
files:
  - src/main.rs
  - src/lib.rs
  - src/session.rs
  - src/executor/global/ods.rs
  - src/procs/univariate/plot_graphics.rs
  - src/procs/reg/output/plots.rs
  - tests/cli.rs
acceptance:
  - "cargo test -p sasrs --test cli write_failure 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "cargo test -p sasrs --test cli ods_ 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "! grep -rn 'WARNING: Could not write' src/lib.rs src/session.rs"
```

### Scope
- `--log`/`--print` impossible à écrire : message ERROR sur stderr, contenu reversé sur stderr (jamais perdu), exit 2 ; stdout fermé : pas de panique, exit 2.
- Écriture d'un fichier ODS (CLOSE ou filet de fin de run) ou d'une image en échec : `log.error` (compté) avec le chemin complet, et non plus une NOTE « WARNING: ».
- Ouvrir une destination alors qu'une autre destination à fichier est ouverte : finaliser et écrire la précédente d'abord ; le listing texte produit avant `ODS HTML FILE=` reste dans le listing ; `ODS _ALL_ CLOSE` ferme la destination courante ; destination à fichier sans contenu → NOTE explicite.
- Panique pendant `run()` : `catch_unwind`, log partiel conservé + `ERROR: internal error …`, exit 2.
- Tests CLI (préfixes contractuels) `write_failure_*` (log, print, dossier absent, permission) et `ods_*` (fichier ODS non inscriptible, deux destinations successives).
- Ne pas : modifier le format des NOTEs de succès ; toucher aux compteurs du log (J06-P2).

### Context
- `src/main.rs:70-83`, `src/lib.rs:86-137`, `src/session.rs:360-437` (open/close destination), sites NOTE-WARNING images.
- Harnais : `tests/cli.rs` (J01-P3).

## J02-P2 — Erreurs macro comptées, %ABORT, références non résolues

```yaml
id: J02-P2
kind: implement
tier: T2
size: M
depends_on: [J02-P0]
files:
  - src/macros/
  - src/executor/mod.rs
  - tests/fixtures/j02/macro_
  - tests/snapshots/snapshot__fixtures@j02__macro_
acceptance:
  - "cargo test -p sasrs --lib macro_diag 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "grep -rq 'take_abort_request' src/executor"
  - "grep -rq 'Apparent symbolic reference' src/macros"
```

### Scope
- `macros::error::emit_error` et les sites « commentaire seulement » (%EVAL/%SYSEVALF, %DO emballé, %SYSFUNC inconnue, %INCLUDE illisible ou trop imbriqué, récursion, %GOTO sans label, paramètres positionnels en trop, mot-clé inconnu) : canal de diagnostics du `MacroEngine` vidé par l'executor dans le log → ERROR comptées, texte SAS cité (doc SAS 9.4 Macro Language Reference).
- Sites `/* NOTE: … */` (%SYSEXEC, %WINDOW, %SYSCALL non géré…) : vraie NOTE du log.
- `%ABORT` : `take_abort_request` consommé par l'executor ; arrêt du programme ; CANCEL/ABEND/RETURN n documentés et mappés sur le code retour.
- WARNINGs SAS `Apparent symbolic reference X not resolved.` et `Apparent invocation of macro X not resolved.` aux cas où SAS les émet.
- `%LENGTH` d'un argument vide : vérifier contre la doc et corriger si besoin.
- Tests unitaires préfixés `macro_diag` ; fixtures `tests/fixtures/j02/macro_*.sas`.
- Snapshots existants modifiés : justification par fixture.

### Context
- `src/macros/error.rs:32-34`, `src/macros/control/jump.rs:79-82`, `src/macros/mod.rs:350`, `src/macros/include.rs:127-148`, `src/macros/define/invoke.rs:60-344`, `src/executor/mod.rs:82-150`.

## J02-P3 — Contrat « reconnu mais ignoré » : instructions de procédure

```yaml
id: J02-P3
kind: implement
tier: T2
size: M
depends_on: [J02-P0]
files:
  - src/procs/common/parse.rs
  - src/procs/common/stmt.rs
  - src/procs/common/mod.rs
  - src/procs/common/tests.rs
  - docs/support-contract.md
  - tests/fixtures/j02/contract_
  - tests/snapshots/snapshot__fixtures@j02__contract_
acceptance:
  - "test -f docs/support-contract.md && grep -q 'ERROR' docs/support-contract.md && grep -q 'WARNING' docs/support-contract.md"
  - "cargo test -p sasrs --lib contract_ 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "ls tests/fixtures/j02/contract_*.sas"
```

### Scope
- `docs/support-contract.md` : règle (ERROR si un résultat ou un dataset peut changer ; WARNING si effet d'affichage seulement ; instruction inconnue → ERROR façon SAS 180-322 ; jamais de repli silencieux) + exemples.
- `parse_proc_body` : instruction non reconnue → ERROR au lieu de `skip_to_semi` ; helpers partagés `unsupported_statement(proc, stmt)` (ERROR) et `ignored_display_statement` (WARNING) pour BY/WEIGHT/FREQ/OUTPUT/ID/WHERE/CLASS vs FORMAT/LABEL/ATTRIB ; s'applique d'un coup aux ~29 procs appelantes (COPY/IDLABEL de TRANSPOSE, ID/FREQ de MEANS, BY des procs de modélisation…).
- Instructions globales valides dans une étape PROC (TITLE, FOOTNOTE, OPTIONS, LIBNAME, ODS, FILENAME) : continuer de fonctionner (vérifier où elles sont traitées).
- Tests unitaires préfixés `contract_` ; fixtures `tests/fixtures/j02/contract_*.sas` (BY dans GLM, WEIGHT dans LOGISTIC, instruction inventée, FORMAT dans une proc qui l'ignore).
- Snapshots existants qui changent : justification par fixture ; si un fixture existant utilisait une instruction ignorée, il garde son intention (instruction retirée du fixture ou sortie ERROR assumée, au choix justifié).

### Context
- `src/procs/common/parse.rs:11-97` (`parse_proc_options`, `parse_proc_body`), `procs/common/stmt.rs`.
- Issue #5 (« contrat explicite pour toute fonctionnalité reconnue mais ignorée »).

## J02-P4 — Options ignorées en silence : procs Base et PROC SQL

```yaml
id: J02-P4
kind: implement
tier: T3
size: M
depends_on: [J02-P3]
files:
  - src/procs/compare/parse.rs
  - src/procs/univariate/parse.rs
  - src/procs/freq/parse.rs
  - src/procs/datasets/parse.rs
  - src/procs/means/parse.rs
  - src/procs/printto.rs
  - src/procs/plot.rs
  - src/sql/parser/
acceptance:
  - "cargo test -p sasrs --lib silent_opt_ 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "! grep -q 'redirected to' src/procs/printto.rs"
```

### Scope
- Options reconnues non honorées → ERROR via les helpers de J02-P3 : COMPARE `CRITERION=`/`METHOD=`/`BRIEF`/`LISTALL`/`OUT*`/`MAXPRINT=` et instructions ID/VAR/WITH/BY (implémentées en J07-P3) ; UNIVARIATE `VARDEF=`/`PCTLDEF=` non par défaut ; FREQ options de TABLES inconnues ; DATASETS `KILL` et options inconnues ; MEANS statistique inconnue ou non calculable dans `OUTPUT` (`clm(x)=`) ; SQL instruction inconnue, `OUTOBS=`/`INOBS=` (implémenter si trivial, sinon ERROR).
- Honorer ce qui est trivial : UNIVARIATE `NOPRINT`, SQL `NOPRINT`.
- PRINTTO : supprimer la NOTE trompeuse « redirected to » ; `LOG=`/`PRINT=` → WARNING « routing not supported » jusqu'à J07-P5.
- PLOT options d'affichage (`HREF=`, `VREF=`, `HAXIS=`…) → WARNING.
- Tests unitaires préfixés `silent_opt_`.

### Context
- `compare/parse.rs:59-82`, `univariate/parse.rs:25-42`, `freq/parse.rs:171-180`, `datasets/parse.rs:116`, `means/parse.rs:268-338`, `printto.rs:88-162`, `plot.rs:130-195`, `sql/parser/stmt.rs:51-55`.
- `docs/support-contract.md` (J02-P3).

## J02-P5 — Replis silencieux de modélisation

```yaml
id: J02-P5
kind: implement
tier: T2
size: M
depends_on: [J02-P3]
files:
  - src/procs/genmod/parse.rs
  - src/procs/logistic/parse.rs
  - src/procs/logistic/design.rs
  - src/procs/mixed/parse.rs
  - src/procs/glimmix/parse.rs
  - src/procs/discrim/parse.rs
  - src/procs/common/model.rs
  - src/procs/reg/parse/
acceptance:
  - "cargo test -p sasrs --lib model_fallback_ 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- GENMOD : `DIST=`/`LINK=` inconnus → ERROR (plus de repli POISSON/lien canonique) ; `LINK=POWER(λ)` → ERROR sauf λ=-1 ; options MODEL inconnues (`OFFSET=`…) → ERROR.
- LOGISTIC : options PROC (`DESCENDING`, `ORDER=`) honorées ou ERROR ; `LINK=` inconnu → ERROR ; options MODEL inconnues → ERROR ; CLASS `(PARAM= REF=)` analysé correctement (plus de jetons pris pour des variables) : `PARAM=REF` et `REF=FIRST|LAST` honorés, le reste → ERROR.
- MIXED/GLIMMIX : `TYPE=`/`METHOD=` inconnus → ERROR ; `DDFM=` autre que CONTAIN → ERROR (au lieu d'une NOTE).
- DISCRIM : `METHOD≠NORMAL`, `POOL=NO|TEST`, `POOL=` inconnu → ERROR (plus de repli LDA).
- `common::model::parse_effect_list` : `a*b`, `a(b)` → ERROR dans les procs qui les aplatissaient.
- REG : options PROC/MODEL inconnues → ERROR.
- Tests unitaires préfixés `model_fallback_` ; syntaxe de référence = doc SAS/STAT 9.4 citée.

### Context
- `genmod/parse.rs:79-134`, `logistic/parse.rs:20-84`, `logistic/design.rs:47-51`, `mixed/parse.rs:6-59`, `glimmix/parse.rs:5-55`, `discrim/parse.rs:37-160`, `common/model.rs:82-94`, `reg/parse/options.rs:91-94`, `reg/parse/model/mod.rs:216-218`.

## J02-P6 — Convergence et singularité véridiques

```yaml
id: J02-P6
kind: implement
tier: T3
size: M
depends_on: [J02-P5]
files:
  - src/procs/genmod/
  - src/procs/glimmix/
  - src/procs/mixed/
  - src/procs/logistic/
  - src/procs/glm/
  - src/procs/anova/
acceptance:
  - "cargo test -p sasrs --lib convergence_ 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
```

### Scope
- Les listings n'affirment la convergence que si elle est atteinte : GENMOD (ligne « Convergence criterion … satisfied » imprimée après l'ajustement, épuisement du step-halving = non-convergence), GLIMMIX (PQL, Laplace : drapeau Nelder-Mead respecté), MIXED (général et legacy).
- Non-convergence → WARNING SAS (« Convergence was not attained… », « The maximum likelihood estimate may not exist… ») au lieu de NOTE ou de silence ; LOGISTIC ordinal : un pas singulier ne fait plus `break` en silence.
- Composante de variance tronquée à 0 → NOTE SAS « Estimated G matrix is not positive definite. » ; λ plafonné → NOTE.
- GLM/ANOVA plan de rang incomplet : ERROR explicite (plus de SSE/SE NaN silencieux) tant que l'inverse généralisée n'existe pas.
- Tests unitaires préfixés `convergence_` (cas non convergent, séparation quasi complète, plan singulier) ; messages cités de la doc SAS.
- Snapshots existants modifiés : justification par fixture.

### Context
- `genmod/mod.rs:225-236`, `genmod/fit.rs:106-137`, `glimmix/mod.rs:315-317`, `glimmix/pql.rs:93-111`, `mixed/general_report.rs:187`, `mixed/legacy_report.rs:142`, `mixed/plan.rs:280-284`, `logistic/binary.rs:36-104`, `logistic/ordinal.rs:225-246`, `glm/design.rs:36-45`, `anova/design.rs:12-15`.

## J02-P7 — Documentation de couverture honnête (J02)

```yaml
id: J02-P7
kind: implement
tier: T5
size: M
depends_on: [J02-P1, J02-P2, J02-P3, J02-P4, J02-P5, J02-P6]
files:
  - README.md
  - docs/support-contract.md
  - src/procs/genmod/mod.rs
  - src/procs/glimmix/mod.rs
  - src/procs/mixed/mod.rs
  - src/procs/princomp/mod.rs
  - src/procs/gplot/mod.rs
  - src/procs/gchart/mod.rs
acceptance:
  - "grep -q '^## Exit codes' README.md"
  - "grep -q 'docs/support-contract.md' README.md"
```

### Scope
- `README.md` : section `## Exit codes` (0/1/2, cas CLI) ; lien vers le contrat ; chaque ligne des tableaux de couverture confrontée au code : retirer ou qualifier les affirmations fausses relevées à la planification (FACTOR `QUARTIMAX`/OBLIMIN = ERROR, GLIMMIX `METHOD=QUAD` = ERROR, SGPLOT `GROUP=`/`RESPONSE=`/`STAT=`/`SCALE=`/`TYPE=` non rendus, PLOT `=group`, GCHART HBAR dessiné en VBAR, `ODS GRAPHICS RESET=`, PRINTTO, COMPARE, MEANS WEIGHT (quantiles non pondérés jusqu'à J03), options devenues ERROR en J02).
- `docs/support-contract.md` : tableau des comportements passés d'« ignoré » à ERROR/WARNING en J02.
- Doc-comments `//!` périmés des six modules listés (GENMOD « Gamma errors », PRINCOMP OUT=, GPLOT, GCHART PIE, GLIMMIX NOTEs promises, MIXED).
- Ne pas : modifier du code exécutable.

### Context
- Rapports d'écart : ce Scope ; `docs/support-contract.md` ; `CONTRIBUTING.md` (quatre états).

## J02-P8 — Review J02

```yaml
id: J02-P8
kind: review
tier: T2
size: S
depends_on: [J02-P1, J02-P2, J02-P3, J02-P4, J02-P5, J02-P6, J02-P7]
files: []
acceptance:
  - "cargo test -p sasrs --test cli write_failure 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "cargo test -p sasrs --test cli ods_ 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "! grep -rn 'WARNING: Could not write' src/lib.rs src/session.rs"
  - "cargo test -p sasrs --lib macro_diag 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "grep -rq 'take_abort_request' src/executor"
  - "grep -rq 'Apparent symbolic reference' src/macros"
  - "test -f docs/support-contract.md && grep -q 'ERROR' docs/support-contract.md && grep -q 'WARNING' docs/support-contract.md"
  - "cargo test -p sasrs --lib contract_ 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "ls tests/fixtures/j02/contract_*.sas"
  - "cargo test -p sasrs --lib silent_opt_ 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "! grep -q 'redirected to' src/procs/printto.rs"
  - "cargo test -p sasrs --lib model_fallback_ 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "cargo test -p sasrs --lib convergence_ 2>&1 | grep -qE 'test result: ok\\. [1-9]'"
  - "grep -q '^## Exit codes' README.md"
  - "grep -q 'docs/support-contract.md' README.md"
  - "scripts/ci-status.sh consolidation"
  - "cargo test -p sasrs"
  - "cargo build --features graphics && cargo build --features s3"
  - "cargo fmt --check && cargo clippy --all-targets -- -D warnings"
```

### Scope
Review the merged milestone diff with verify-before-done, code-review, test-design. Report; change nothing.
