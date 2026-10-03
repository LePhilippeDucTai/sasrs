# DECISIONS — conformance-fixes

Generated from Mission Control; do not edit.

## c77c6ebc-b7af-476e-aadd-c33fd22cbcd6 · resolved
Faut-il accepter la correction de l'oracle expected/pct.csv (base/sql-join-remerge) vers les valeurs issues de la sémantique SQL SAS documentée du remerge ?
Choice: accept-oracle-fix

## 680238b2-95d4-4161-9ef7-9d1abf039aa5 · resolved
Finding bloquant de la revue J01-P4 : le commit b9924dd a ajouté un snapshot sans ligne canonique « Snapshot: <fixture> — <raison> » (CONTRIBUTING §4). L'historique poussé ne peut être ni amendé ni réécrit. Quelle correction ?
Choice: corrective-unit

## 5264b8a7-da31-44e3-8597-645982e67c22 · resolved
L'implémentation de OUTPUT OUT= CHISQ (_PCHI_, _PCHI_DF_, _P_PCHI) exige d'ajouter un champ « output » à FreqAst et son branchement dans execute(), tous deux dans src/procs/freq/mod.rs (le calcul et l'écriture du dataset peuvent résider dans output.rs/stats.rs). Étendre le périmètre de J02-P1 pour permettre ce correctif ?
Choice: extend-scope-mod-rs

## 8be00f84-c24f-4bc6-849d-4a7c6f7cefd0 · resolved
Quelle valeur d'oracle bénir pour expected/stats.csv du cas base/freq-chisq-output, sachant que l'attendu (3.4722222222222223 / 0.062407418568705825) contredit la formule citée par sa propre provenance (3.3333333333333335 / 0.06788915486182903) et que l'implémenteur n'a pas le droit de modifier les attendus d'un cas known-divergence ?
Choice: revise_plan

## a43c8c14-41fa-4775-b039-1ab2e43f1987 · resolved
Quelle source de vérité pour les noms de colonnes du dataset OUTPUT OUT= CHISQ (plan : _PCHI_/_PCHI_DF_/_PCH_P ; expected : _PCHI_/_PCHI_DF_/P_PCHI ; doc SAS FREQ : PCHI/DF_PCHI/P_PCHI) ?
Choice: proceed

## e692090b-74c0-42de-99fd-1dcbd54ad589 · resolved
Étendre le périmètre inscriptible de J02-P1 à src/procs/freq/tests/ (littéraux FreqAst à mettre à jour mécaniquement : mod.rs, parse.rs, crosstab.rs) ?
Choice: extend-scope-freq-tests

## dbe58c3f-2d08-4afb-9323-1d76e34176de · resolved
Qui corrige l'entrée obsolète ("glm", "output out=bad") du test de contrat src/procs/common/tests.rs, hors périmètre J02-P2, pour que le check « test » des unités de revue (J02-P5, J03-P1, J03-P3) repasse ?
Choice: extend-scope

## 3865498b-db3b-4829-af82-5c4259bb5aeb · resolved
Comment remedier au finding bloquant sur les lignes Snapshot: manquantes (commits 79b3c31 et f9d3543 déjà intégrés, historique non réécrivable) ?
Choice: retroactive-registry

## 829bd238-f505-4302-8025-ace260f25045 · resolved
Finding bloquant revue J02-P5 : commits 79b3c31/f9d3543 ont modifié des .snap sans ligne « Snapshot: » (CONTRIBUTING §4) ; historique poussé non réinscriptible. Quelle correction ?
Choice: corrective-unit

