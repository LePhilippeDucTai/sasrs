# PLAN — conformance-fixes

Traiter les 8 issues techniques ouvertes #13–#20 (conformance SAS 9.4 et déterminisme) en faisant passer les cas known-divergence correspondants du corpus conformance/cases au statut validated (le test --test conformance impose alors leur succès), plus la clôture justifiée des méta-issues #10 et #11 (décisions du coordinateur, pas des unités de code). Chaque unité est vérifiée par des checks locaux exécutables (cargo dans le conteneur distrobox ombre-mingw, python3 de l'hôte), chaque jalon par une revue indépendante. Exécution au plus un exécutant simultané (directive Philippe 28/09 : GLM 5.3 max via dsh).

Protocol: 3 · Plan: `9602840f-21fb-4d37-a463-b782085bdfba` · Revision: 1

Base: `milestone/conformance-fixes`

## J01 — Base SAS : UPDATE, MEANS, SQL (issues #13, #14, #15)

### J01-P1 — UPDATE sans KEY= (issue #13)
Mécanique d'acceptation commune : (1) corriger le comportement dans src/ (borné aux préfixes de l'unité) ; (2) passer le(s) cas conformance concerné(s) à "status": "validated", remplir leur champ "issue" avec le numéro GitHub correspondant, ajuster expected/ et les attentes de log si la sortie change (uniquement de façon justifiée par la provenance doc SAS déjà citée dans le corpus) ; (3) régénérer conformance/STATUS.md via python3 scripts/conformance_report.py et vérifier --check ; (4) la promotion n'est jamais un simple retournement de statut sans correctif — le harnais détecte les cas « à promouvoir ». Cargo uniquement via distrobox enter ombre-mingw ; INSTA_UPDATE=no ; arbre propre requis. Besoin d'un fichier hors périmètre → s'arrêter en blocked avec une décision. UPDATE sans KEY= : accepter UPDATE avec seulement BY (supprimer l'erreur « An UPDATE statement requires a KEY= option » quand BY= présent ; KEY= reste supporté). Cas base/update-master → validated. Reproducer : conformance/cases/base/update-master/program.sas.

Tier: T3 · Depends: none · Checks: fmt, clippy, test-conformance, conformance-status

### J01-P2 — MEANS OUTPUT liste de stats et OUT= par défaut (issue #14)
Mécanique d'acceptation commune : (1) corriger le comportement dans src/ (borné aux préfixes de l'unité) ; (2) passer le(s) cas conformance concerné(s) à "status": "validated", remplir leur champ "issue" avec le numéro GitHub correspondant, ajuster expected/ et les attentes de log si la sortie change (uniquement de façon justifiée par la provenance doc SAS déjà citée dans le corpus) ; (3) régénérer conformance/STATUS.md via python3 scripts/conformance_report.py et vérifier --check ; (4) la promotion n'est jamais un simple retournement de statut sans correctif — le harnais détecte les cas « à promouvoir ». Cargo uniquement via distrobox enter ombre-mingw ; INSTA_UPDATE=no ; arbre propre requis. Besoin d'un fichier hors périmètre → s'arrêter en blocked avec une décision. OUTPUT accepte une liste de stats stat=<var> (ex. sum=total_amount) et OUT= sans variable d'analyse produit les variables CLASS + stats par défaut conformes à la doc MEANS. Cas base/means-class-output → validated.

Tier: T3 · Depends: J01-P1 · Checks: fmt, clippy, test-conformance, conformance-status

### J01-P3 — SQL WHERE en jointure et remerge GROUP BY (issue #15)
Mécanique d'acceptation commune : (1) corriger le comportement dans src/ (borné aux préfixes de l'unité) ; (2) passer le(s) cas conformance concerné(s) à "status": "validated", remplir leur champ "issue" avec le numéro GitHub correspondant, ajuster expected/ et les attentes de log si la sortie change (uniquement de façon justifiée par la provenance doc SAS déjà citée dans le corpus) ; (3) régénérer conformance/STATUS.md via python3 scripts/conformance_report.py et vérifier --check ; (4) la promotion n'est jamais un simple retournement de statut sans correctif — le harnais détecte les cas « à promouvoir ». Cargo uniquement via distrobox enter ombre-mingw ; INSTA_UPDATE=no ; arbre propre requis. Besoin d'un fichier hors périmètre → s'arrêter en blocked avec une décision. (a) prédicat WHERE appliqué comme condition de jointure dans « from a, b where … » (anti-produit cartésien) ; (b) remerge GROUP BY : référencer une colonne non agrégée du FROM dans le SELECT d'un GROUP BY doit résoudre contre la source (sémantique SAS SQL). L'oracle est le jeu d'attendus expected/ issu de la doc — borne stricte, pas de généralisation au-delà du cas. Cas base/sql-join-remerge → validated.

Tier: T3 · Depends: J01-P2 · Checks: fmt, clippy, test-conformance, conformance-status

### J01-P4 — Revue J01
Revue indépendante du diff intégré du jalon (verify-before-done, code-design, test-design) : rejouer tous les checks sur le commit épinglé ; vérifier que chaque promotion de cas est justifiée par la provenance doc SAS, que chaque snapshot modifié est justifié (ligne Snapshot: dans le commit), qu'aucun silent-behavior n'est introduit et qu'aucune documentation ne promet plus que le code. Ne rien modifier ; tout finding bloquant devient une unité corrective. Porte en plus la tolérance sur les promotions J01 : chaque cas promu (update-master, means-class-output, sql-join-remerge) doit rester justifié par sa provenance doc SAS.

Tier: T2 · Depends: J01-P1, J01-P2, J01-P3 · Checks: fmt, clippy, clippy-s3, test, test-conformance, conformance-status, coverage-claims, ci-structure

## J02 — Procédures statistiques : OUTPUT et layout (issues #16–#19)

### J02-P1 — FREQ CHISQ via OUTPUT OUT= (issue #16)
Mécanique d'acceptation commune : (1) corriger le comportement dans src/ (borné aux préfixes de l'unité) ; (2) passer le(s) cas conformance concerné(s) à "status": "validated", remplir leur champ "issue" avec le numéro GitHub correspondant, ajuster expected/ et les attentes de log si la sortie change (uniquement de façon justifiée par la provenance doc SAS déjà citée dans le corpus) ; (3) régénérer conformance/STATUS.md via python3 scripts/conformance_report.py et vérifier --check ; (4) la promotion n'est jamais un simple retournement de statut sans correctif — le harnais détecte les cas « à promouvoir ». Cargo uniquement via distrobox enter ombre-mingw ; INSTA_UPDATE=no ; arbre propre requis. Besoin d'un fichier hors périmètre → s'arrêter en blocked avec une décision. CHISQ via OUTPUT OUT= expose _PCHI_, _PCHI_DF_, _PCH_P (p-value) conformément à la doc FREQ. Cas base/freq-chisq-output → validated.

Tier: T3 · Depends: J01-P3 · Checks: fmt, clippy, test-conformance, conformance-status

### J02-P2 — OUTPUT dans FREQ/GLM, SKEWNESS/KURTOSIS UNIVARIATE (issue #17)
Mécanique d'acceptation commune : (1) corriger le comportement dans src/ (borné aux préfixes de l'unité) ; (2) passer le(s) cas conformance concerné(s) à "status": "validated", remplir leur champ "issue" avec le numéro GitHub correspondant, ajuster expected/ et les attentes de log si la sortie change (uniquement de façon justifiée par la provenance doc SAS déjà citée dans le corpus) ; (3) régénérer conformance/STATUS.md via python3 scripts/conformance_report.py et vérifier --check ; (4) la promotion n'est jamais un simple retournement de statut sans correctif — le harnais détecte les cas « à promouvoir ». Cargo uniquement via distrobox enter ombre-mingw ; INSTA_UPDATE=no ; arbre propre requis. Besoin d'un fichier hors périmètre → s'arrêter en blocked avec une décision. Instruction OUTPUT supportée dans PROC FREQ et PROC GLM ; UNIVARIATE accepte SKEWNESS/KURTOSIS dans OUTPUT. Cas stat/freq-fisher-2x2, stat/glm-oneway-predicted, stat/univariate-moments-output → validated.

Tier: T3 · Depends: J02-P1 · Checks: fmt, clippy, test-conformance, conformance-status

### J02-P3 — CORR OUTP=/OUTS= layout (issue #18)
Mécanique d'acceptation commune : (1) corriger le comportement dans src/ (borné aux préfixes de l'unité) ; (2) passer le(s) cas conformance concerné(s) à "status": "validated", remplir leur champ "issue" avec le numéro GitHub correspondant, ajuster expected/ et les attentes de log si la sortie change (uniquement de façon justifiée par la provenance doc SAS déjà citée dans le corpus) ; (3) régénérer conformance/STATUS.md via python3 scripts/conformance_report.py et vérifier --check ; (4) la promotion n'est jamais un simple retournement de statut sans correctif — le harnais détecte les cas « à promouvoir ». Cargo uniquement via distrobox enter ombre-mingw ; INSTA_UPDATE=no ; arbre propre requis. Besoin d'un fichier hors périmètre → s'arrêter en blocked avec une décision. Layout OUTP=/OUTS= conforme à la doc CORR : observations TYPE= N/MEAN/STD/SUM/MIN/MAX + CORR. Les valeurs de corrélation déjà conformes (0.9600051599448531 Pearson, 0.9428571428571428 Spearman) doivent être conservées bit-à-bit. Cas stat/corr-pearson-outp, stat/corr-spearman-outs → validated.

Tier: T3 · Depends: J02-P2 · Checks: fmt, clippy, test-conformance, conformance-status

### J02-P4 — NPAR1WAY Z de Wilcoxon sans correction (issue #19)
Mécanique d'acceptation commune : (1) corriger le comportement dans src/ (borné aux préfixes de l'unité) ; (2) passer le(s) cas conformance concerné(s) à "status": "validated", remplir leur champ "issue" avec le numéro GitHub correspondant, ajuster expected/ et les attentes de log si la sortie change (uniquement de façon justifiée par la provenance doc SAS déjà citée dans le corpus) ; (3) régénérer conformance/STATUS.md via python3 scripts/conformance_report.py et vérifier --check ; (4) la promotion n'est jamais un simple retournement de statut sans correctif — le harnais détecte les cas « à promouvoir ». Cargo uniquement via distrobox enter ombre-mingw ; INSTA_UPDATE=no ; arbre propre requis. Besoin d'un fichier hors périmètre → s'arrêter en blocked avec une décision. Z de Wilcoxon sans correction de continuité 0.5 pour coller à l'exemple doc (Z=2.893187811789223, p=0.0038135318825) ; si une option SAS active la correction, l'honorer explicitement, sinon la retirer. Tout snapshot touché doit être justifié ; si la doc documente aussi une variante corrigée, l'exposer sous un nom distinct plutôt que de changer silencieusement l'existant. Cas stat/npar1way-wilcoxon-out → validated.

Tier: T3 · Depends: J02-P3 · Checks: fmt, clippy, test-conformance, conformance-status

### J02-P5 — Revue J02
Revue indépendante du diff intégré du jalon (verify-before-done, code-design, test-design) : rejouer tous les checks sur le commit épinglé ; vérifier que chaque promotion de cas est justifiée par la provenance doc SAS, que chaque snapshot modifié est justifié (ligne Snapshot: dans le commit), qu'aucun silent-behavior n'est introduit et qu'aucune documentation ne promet plus que le code. Ne rien modifier ; tout finding bloquant devient une unité corrective. Porte en plus la vérification des tolérances numériques des attendus (valeurs bit-à-bit citées dans les issues #18/#19).

Tier: T2 · Depends: J02-P1, J02-P2, J02-P3, J02-P4 · Checks: fmt, clippy, clippy-s3, test, test-conformance, conformance-status, coverage-claims, ci-structure

## J03 — Déterminisme + clôture (issue #20, méta #10/#11)

### J03-P1 — Déterminisme UNION et sidecar (issue #20)
(a) UNION (non-ALL) : tri déterministe des lignes après unique() — tri total et stable (clé définie sur toutes les colonnes, ordre des colonnes fixe) sinon l'instabilité persiste sur les ex æquo ; (b) sidecar <t>.parquet.sasmeta.json : sérialisation à clés triées (BTreeMap) au lieu de la HashMap. Acceptance exécutable spécifique : créer tests/determinism/ contenant un reproducer SAS (UNION non-ALL + écriture d'une table avec sidecar) et le script run_check.sh qui exécute le binaire sasrs (cargo run --locked -p sasrs, via distrobox enter ombre-mingw) 5 fois dans des répertoires temporaires distincts et compare byte-à-byte via cmp tous les artefacts produits (log, print, table parquet et sidecar) — exit 0 seulement si tous identiques. Le check j03-determinism appelle bash tests/determinism/run_check.sh. En plus : cargo test --test differential passe. Aucune promotion de cas conformance pour cette unité (aucun cas ne couvre #20).

Tier: T4 · Depends: J02-P4 · Checks: fmt, clippy, test, test-conformance, conformance-status, j03-determinism

### J03-P2 — Clôture documentaire #10/#11 (méta)
Documentation seulement : s'assurer que tous les case.json des cas promus référencent leur numéro d'issue GitHub dans le champ "issue" (les unités J01/J02 le font déjà ; compléter si besoin), régénérer conformance/STATUS.md (python3 scripts/conformance_report.py) et vérifier --check. Ne touche ni .mission-control/ ni docs/plans/. La clôture GitHub elle-même est une action du coordinateur, pas de cette unité. Le cas stat/reg-simple-lineart reste known-divergence (hors mandat, sans issue ouverte) : ne pas le modifier.

Tier: T5 · Depends: J01-P1, J01-P2, J01-P3, J02-P1, J02-P2, J02-P3, J02-P4, J03-P1 · Checks: conformance-status

### J03-P3 — Revue finale
Revue indépendante du diff intégré du jalon (verify-before-done, code-design, test-design) : rejouer tous les checks sur le commit épinglé ; vérifier que chaque promotion de cas est justifiée par la provenance doc SAS, que chaque snapshot modifié est justifié (ligne Snapshot: dans le commit), qu'aucun silent-behavior n'est introduit et qu'aucune documentation ne promet plus que le code. Ne rien modifier ; tout finding bloquant devient une unité corrective. Vérifie le Goal complet : 11 divergences initiales → 0 restante pour #13–#20 (le cas stat/reg-simple-lineart restant known-divergence sans issue ouverte doit être signalé dans le rapport de revue, non corrigé hors mandat) ; STATUS.md cohérent ; aucune promesse de couverture au-delà du validé.

Tier: T2 · Depends: J01-P4, J02-P5, J03-P1, J03-P2 · Checks: fmt, clippy, clippy-s3, test, test-conformance, conformance-status, coverage-claims, ci-structure, j03-determinism

