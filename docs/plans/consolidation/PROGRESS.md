# PROGRESS — consolidation

Updated 2026-09-23 · s0 · base consolidation · ⬜ todo 🔄 wip ✅ done ❌ failed ⏸ blocked ⏭ dropped

## J01 — Base verte, CI bloquante, harnais CLI, roadmap réconciliée (4/6)

| Part | Title | Tier | Status | Tries | Note |
|------|-------|------|--------|-------|------|
| J01-P1 | Baseline lint et graphics verte | T5 | ✅ | 1 | merge 147e839 (worker daa4b39) ; 5/5 acceptations rejouées vertes |
| J01-P2 | CI GitHub Actions bloquante et scripts de vérification | T4 | ✅ | 1 | merge 82ed882 (worker 0107bec) ; 6/6 acc. vertes (acc.5 check.sh lint rejouée sur l'arbre intégré après fix P2b) |
| J01-P2b | Fix baseline clippy s3 (doc_lazy_continuation) | T5 | ✅ | 1 | merge b6d11e7 (worker 3a18721) ; 4/4 acc. rejouées vertes par l'orchestrateur |
| J01-P3 | Harnais de tests CLI réels | T4 | ✅ | 1 | merge 779a27d (worker c00f5f8) ; 2/2 acceptations, 8 tests cli verts |
| J01-P4 | Roadmap réconciliée et règles de développement par agents | T5 | ✅ | 1 | merge 102c3ae (worker fe5a646) ; 5/5 acceptations vertes |
| J01-P5 | Protection de main par le check CI | T5 | ⬜ | 0 | prête (P2+P4 mergés) |
| J01-P6 | Review J01 | T2 | ⬜ | 0 | attend P5 ; effort max requis au dispatch |

## J02 — Diagnostics fiables : codes retour, erreurs macro, contrat « reconnu mais ignoré » (0/8)

| Part | Title | Tier | Status | Tries | Note |
|------|-------|------|--------|-------|------|
| J02-P1 | Échecs d'écriture, code retour et pertes ODS | T3 | ⬜ | 0 | |
| J02-P2 | Erreurs macro comptées, %ABORT, références non résolues | T2 | ⬜ | 0 | |
| J02-P3 | Contrat « reconnu mais ignoré » : instructions de procédure | T2 | ⬜ | 0 | |
| J02-P4 | Options ignorées en silence : procs Base et PROC SQL | T3 | ⬜ | 0 | |
| J02-P5 | Replis silencieux de modélisation | T2 | ⬜ | 0 | |
| J02-P6 | Convergence et singularité véridiques | T3 | ⬜ | 0 | |
| J02-P7 | Documentation de couverture honnête (J02) | T5 | ⬜ | 0 | |
| J02-P8 | Review J02 | T2 | ⬜ | 0 | |

## J03 — Divergences numériques et encodage (0/8)

| Part | Title | Tier | Status | Tries | Note |
|------|-------|------|--------|-------|------|
| J03-P1 | Oracle indépendant des statistiques pondérées | T4 | ⬜ | 0 | |
| J03-P2 | Statistiques pondérées MEANS/SUMMARY et UNIVARIATE | T2 | ⬜ | 0 | |
| J03-P3 | Contrat d'encodage, lexer UTF-8 et BOM | T3 | ⬜ | 0 | |
| J03-P4 | Formats et informats sans découpe d'octets | T3 | ⬜ | 0 | |
| J03-P5 | Largeurs de mise en page et longueurs inférées en caractères | T4 | ⬜ | 0 | |
| J03-P6 | Divergences de l'étape DATA | T3 | ⬜ | 0 | |
| J03-P7 | Documentation de couverture (J03) | T5 | ⬜ | 0 | |
| J03-P8 | Review J03 | T2 | ⬜ | 0 | |

## J04 — Intégrité du stockage et des métadonnées (0/6)

| Part | Title | Tier | Status | Tries | Note |
|------|-------|------|--------|-------|------|
| J04-P1 | Protocole d'écriture atomique parquet + sidecar | T2 | ⬜ | 0 | |
| J04-P2 | Suppression, renommage et échange sans orphelins | T3 | ⬜ | 0 | |
| J04-P3 | Sidecar corrompu ou invalide : diagnostic explicite | T3 | ⬜ | 0 | |
| J04-P4 | Métadonnées après transformations SQL | T3 | ⬜ | 0 | |
| J04-P5 | Tests d'interruption et de corruption simulées, doc de récupération | T4 | ⬜ | 0 | |
| J04-P6 | Review J04 | T2 | ⬜ | 0 | |

## J05 — Validation industrielle : conformité, propriétés, différentiel (0/7)

| Part | Title | Tier | Status | Tries | Note |
|------|-------|------|--------|-------|------|
| J05-P1 | Structure du corpus de conformité et exécuteur | T3 | ⬜ | 0 | |
| J05-P2 | Cas de conformité Base issus de la documentation SAS | T4 | ⬜ | 0 | |
| J05-P3 | Cas de conformité statistiques | T4 | ⬜ | 0 | |
| J05-P4 | Tests de propriétés | T3 | ⬜ | 0 | |
| J05-P5 | Différentiel chemin vectorisé / ligne à ligne | T3 | ⬜ | 0 | |
| J05-P6 | CI de conformité et couverture validée publique | T4 | ⬜ | 0 | |
| J05-P7 | Review J05 | T2 | ⬜ | 0 | |

## J06 — Utilisabilité : API Session, Python, distribution, documentation (0/8)

| Part | Title | Tier | Status | Tries | Note |
|------|-------|------|--------|-------|------|
| J06-P1 | Façade publique `sasrs::api` (ADR) | T2 | ⬜ | 0 | |
| J06-P2 | Diagnostics structurés et fichiers produits | T3 | ⬜ | 0 | |
| J06-P3 | Exemples autonomes testés | T4 | ⬜ | 0 | |
| J06-P4 | Wrapper Python durci et testé | T3 | ⬜ | 0 | |
| J06-P5 | Distribution versionnée et manifeste de build | T3 | ⬜ | 0 | |
| J06-P6 | Plan d'une API Python native | T4 | ⬜ | 0 | |
| J06-P7 | Parcours nouvel utilisateur | T5 | ⬜ | 0 | |
| J06-P8 | Review J06 | T2 | ⬜ | 0 | |

## J07 — Compatibilité à forte valeur (1) : MEANS, COMPARE, TRANSPOSE, PRINTTO, informats (0/8)

| Part | Title | Tier | Status | Tries | Note |
|------|-------|------|--------|-------|------|
| J07-P1 | Oracles de conformité écrits avant l'implémentation | T4 | ⬜ | 0 | |
| J07-P2 | PROC MEANS/SUMMARY : options de production | T3 | ⬜ | 0 | |
| J07-P3 | PROC COMPARE : outil de validation de migration | T3 | ⬜ | 0 | |
| J07-P4 | PROC TRANSPOSE complet | T3 | ⬜ | 0 | |
| J07-P5 | PROC PRINTTO : routage réel | T3 | ⬜ | 0 | |
| J07-P6 | Informats persistés dans les métadonnées | T3 | ⬜ | 0 | |
| J07-P7 | Documentation de couverture (J07) | T5 | ⬜ | 0 | |
| J07-P8 | Review J07 | T2 | ⬜ | 0 | |

## J08 — XLSX, BY, ODS OUTPUT, feuille de route avancée, validation de bout en bout (0/7)

| Part | Title | Tier | Status | Tries | Note |
|------|-------|------|--------|-------|------|
| J08-P1 | PROC IMPORT/EXPORT DBMS=XLSX | T3 | ⬜ | 0 | |
| J08-P2 | BY généralisé (CORR, TABULATE) | T3 | ⬜ | 0 | |
| J08-P3 | Objets ODS OUTPUT supplémentaires | T3 | ⬜ | 0 | |
| J08-P4 | Étude des adaptateurs SAS7BDAT/XPT | T4 | ⬜ | 0 | |
| J08-P5 | Feuille de route avancée par comportement borné | T1 | ⬜ | 0 | |
| J08-P6 | Validation de bout en bout et contrôle des promesses de couverture | T4 | ⬜ | 0 | |
| J08-P7 | Review J08 | T2 | ⬜ | 0 | |

## Next session

- Ready: J01-P1 (L, seule), J01-P3, J01-P4
- Orchestrator: sonnet/high
- Decision: none

## Log

- 2026-09-23 s0: plan created (issue #11 et sous-issues #5–#10 ; M46–M66 remappés ; baseline : lint et tests graphics rouges → J01-P1)
