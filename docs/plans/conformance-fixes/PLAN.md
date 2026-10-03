# PLAN — conformance-fixes

Traiter les 8 issues techniques ouvertes #13–#20 (conformance SAS 9.4 et déterminisme) en faisant passer les cas known-divergence correspondants du corpus conformance/cases au statut validated (le test --test conformance impose alors leur succès), plus la clôture justifiée des méta-issues #10 et #11 (décisions du coordinateur, pas des unités de code). Chaque unité est vérifiée par des checks locaux exécutables (cargo dans le conteneur distrobox ombre-mingw, python3 de l'hôte), chaque jalon par une revue indépendante. Exécution au plus un exécutant simultané (directive Philippe 28/09 : GLM 5.3 max via dsh).

Protocol: 3 · Plan: `9602840f-21fb-4d37-a463-b782085bdfba` · Revision: 6

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

Tier: T2 · Depends: J01-P1, J01-P2, J01-P3, J01-P5 · Checks: fmt, clippy, clippy-s3, test, test-conformance, conformance-status, coverage-claims, ci-structure

### J01-P5 — Correction revue J01 : registre rétroactif de provenance des snapshots
Finding bloquant de la revue J01-P4 (attempt 75e50b0d-ef04-4810-bc7a-b9e28c585dbb) : le commit b9924dd a ajouté tests/snapshots/snapshot__fixtures@j01__update_by_only.sas.snap sans la ligne canonique « Snapshot: <fixture> — <raison> » dans le message de commit (CONTRIBUTING §4). L'historique poussé ne doit être ni réécrit ni amendé. Correction durable : créer docs/snapshots-provenance.md, registre des justifications rétroactives : une section par snapshot concerné au format « Snapshot: <fixture> — <raison> », avec commit d'origine, référence de la revue et de la décision. Y documenter tests/snapshots/snapshot__fixtures@j01__update_by_only.sas.snap (commit b9924dd, non-régression UPDATE BY-only, issue #13, revue J01-P4, décision 680238b2). Préciser en en-tête que ce registre est une mesure exceptionnelle : tout nouveau commit modifiant un .snap doit porter sa ligne Snapshot: dans son message.

Tier: T5 · Depends: none · Checks: fmt

## J02 — Procédures statistiques : OUTPUT et layout (issues #16–#19)

### J02-P1 — FREQ CHISQ via OUTPUT OUT= (issue #16)
Mécanique d'acceptation commune : (1) corriger le comportement dans src/ (borné aux préfixes de l'unité) ; (2) passer le(s) cas conformance concerné(s) à "status": "validated", remplir leur champ "issue" avec le numéro GitHub correspondant, ajuster expected/ et les attentes de log si la sortie change (uniquement de façon justifiée par la provenance doc SAS déjà citée dans le corpus) ; (3) régénérer conformance/STATUS.md via python3 scripts/conformance_report.py et vérifier --check ; (4) la promotion n'est jamais un simple retournement de statut sans correctif — le harnais détecte les cas « à promouvoir ». Cargo uniquement via distrobox enter ombre-mingw ; INSTA_UPDATE=no ; arbre propre requis. Besoin d'un fichier hors périmètre → s'arrêter en blocked avec une décision. CHISQ via OUTPUT OUT= expose _PCHI_, _PCHI_DF_, _PCH_P (p-value) conformément à la doc FREQ. Cas base/freq-chisq-output → validated.

Tier: T3 · Depends: J01-P3, J02-P6 · Checks: fmt, clippy, test-conformance, conformance-status

### J02-P2 — OUTPUT dans FREQ/GLM, SKEWNESS/KURTOSIS UNIVARIATE (issue #17)
Mécanique d'acceptation commune : (1) corriger le comportement dans src/ (borné aux préfixes de l'unité) ; (2) passer le(s) cas conformance concerné(s) à "status": "validated", remplir leur champ "issue" avec le numéro GitHub correspondant, ajuster expected/ et les attentes de log si la sortie change (uniquement de façon justifiée par la provenance doc SAS déjà citée dans le corpus) ; (3) régénérer conformance/STATUS.md via python3 scripts/conformance_report.py et vérifier --check ; (4) la promotion n'est jamais un simple retournement de statut sans correctif — le harnais détecte les cas « à promouvoir ». Cargo uniquement via distrobox enter ombre-mingw ; INSTA_UPDATE=no ; arbre propre requis. Besoin d'un fichier hors périmètre → s'arrêter en blocked avec une décision. Instruction OUTPUT supportée dans PROC FREQ et PROC GLM ; UNIVARIATE accepte SKEWNESS/KURTOSIS dans OUTPUT. Cas stat/freq-fisher-2x2, stat/glm-oneway-predicted, stat/univariate-moments-output → validated.

Tier: T3 · Depends: J02-P1, J02-P6 · Checks: fmt, clippy, test-conformance, conformance-status

### J02-P3 — CORR OUTP=/OUTS= layout (issue #18)
Mécanique d'acceptation commune : (1) corriger le comportement dans src/ (borné aux préfixes de l'unité) ; (2) passer le(s) cas conformance concerné(s) à "status": "validated", remplir leur champ "issue" avec le numéro GitHub correspondant, ajuster expected/ et les attentes de log si la sortie change (uniquement de façon justifiée par la provenance doc SAS déjà citée dans le corpus) ; (3) régénérer conformance/STATUS.md via python3 scripts/conformance_report.py et vérifier --check ; (4) la promotion n'est jamais un simple retournement de statut sans correctif — le harnais détecte les cas « à promouvoir ». Cargo uniquement via distrobox enter ombre-mingw ; INSTA_UPDATE=no ; arbre propre requis. Besoin d'un fichier hors périmètre → s'arrêter en blocked avec une décision. Layout OUTP=/OUTS= conforme à la doc CORR : observations TYPE= N/MEAN/STD/SUM/MIN/MAX + CORR. Les valeurs de corrélation déjà conformes (0.9600051599448531 Pearson, 0.9428571428571428 Spearman) doivent être conservées bit-à-bit. Cas stat/corr-pearson-outp, stat/corr-spearman-outs → validated.

Tier: T3 · Depends: J02-P2, J02-P6 · Checks: fmt, clippy, test-conformance, conformance-status

### J02-P4 — NPAR1WAY Z de Wilcoxon sans correction (issue #19)
Mécanique d'acceptation commune : (1) corriger le comportement dans src/ (borné aux préfixes de l'unité) ; (2) passer le(s) cas conformance concerné(s) à "status": "validated", remplir leur champ "issue" avec le numéro GitHub correspondant, ajuster expected/ et les attentes de log si la sortie change (uniquement de façon justifiée par la provenance doc SAS déjà citée dans le corpus) ; (3) régénérer conformance/STATUS.md via python3 scripts/conformance_report.py et vérifier --check ; (4) la promotion n'est jamais un simple retournement de statut sans correctif — le harnais détecte les cas « à promouvoir ». Cargo uniquement via distrobox enter ombre-mingw ; INSTA_UPDATE=no ; arbre propre requis. Besoin d'un fichier hors périmètre → s'arrêter en blocked avec une décision. Z de Wilcoxon sans correction de continuité 0.5 pour coller à l'exemple doc (Z=2.893187811789223, p=0.0038135318825) ; si une option SAS active la correction, l'honorer explicitement, sinon la retirer. Tout snapshot touché doit être justifié ; si la doc documente aussi une variante corrigée, l'exposer sous un nom distinct plutôt que de changer silencieusement l'existant. Cas stat/npar1way-wilcoxon-out → validated.

Tier: T3 · Depends: J02-P3, J02-P6 · Checks: fmt, clippy, test-conformance, conformance-status

### J02-P5 — Revue J02
Revue indépendante du diff intégré du jalon (verify-before-done, code-design, test-design) : rejouer tous les checks sur le commit épinglé ; vérifier que chaque promotion de cas est justifiée par la provenance doc SAS, que chaque snapshot modifié est justifié (ligne Snapshot: dans le commit), qu'aucun silent-behavior n'est introduit et qu'aucune documentation ne promet plus que le code. Ne rien modifier ; tout finding bloquant devient une unité corrective. Porte en plus la vérification des tolérances numériques des attendus (valeurs bit-à-bit citées dans les issues #18/#19).

Tier: T2 · Depends: J02-P1, J02-P2, J02-P3, J02-P4, J02-P6, J02-P7, J02-P8 · Checks: fmt, clippy, clippy-s3, test, test-conformance, conformance-status, coverage-claims, ci-structure

### J02-P6 — Correction d'oracle J02 : freq-chisq-output (décision 8be00f84)
Correction de l'oracle du cas conformance/cases/base/freq-chisq-output, arithmétiquement faux (finding worker J02-P1 attempt f1b5cdc6, décision 8be00f84) : expected/stats.csv doit porter chi²=_PCHI_=3.3333333333333335 et p=P_PCHI=0.06788915486182903 pour la table [[8,2],[4,6]] (N=20 ; E=(6,4,6,4) ; Σ(O−E)²/E=3.3333333333333335 ; dénominateur N(AD−BC)²/((A+B)(C+D)(A+C)(B+D)) = 10·10·12·8 = 9600, et non 9212/9216). Corrige la chaîne de provenance du case.json (arithmétique 9216→9600, formule doc SAS déjà citée). Le statut reste known-divergence (la promotion est faite par J02-P1, PAS par cette unité — gel conformance/README : l'implémenteur de la correction ne modifie pas son propre oracle ; cette unité est indépendante du correctif). Régénère conformance/STATUS.md (python3 scripts/conformance_report.py) et vérifie --check.

Tier: T5 · Depends: J01-P1, J01-P2, J01-P3 · Checks: conformance-status

### J02-P7 — Correction revue J02 : entrée de contrat obsolète (glm, output out=bad)
Finding should-fix du worker J02-P2 (attempt 5bd3c1e7, décision dbe58c3f) : depuis que PROC GLM supporte l'instruction OUTPUT (issue #17), l'entrée obsolète ("glm", "output out=bad") du test de contrat dans src/procs/common/tests.rs échoue (le statement n'est plus une erreur attendue). Mets à jour ce test de contrat : retire ou remplace l'entrée pour refléter le comportement actuel (OUTPUT accepté dans GLM ; garder si pertinent une entrée d'erreur réellement toujours valide). Le check test (cargo test --locked -p sasrs) doit repasser. Ne change rien d'autre.

Tier: T5 · Depends: J02-P2 · Checks: test

### J02-P8 — Correction revue J02 : registre rétroactif snapshots + champ issue npar1way (décisions 3865498b/829bd238)
Findings de la revue J02-P5 (attempt 7b4deaa6, décisions 3865498b/829bd238) : (a) BLOCKING — les commits 79b3c31 (tests/snapshots/snapshot__fixtures@j08__by_corr.sas.snap) et f9d3543 (snapshots npar1way) ont modifié des .snap sans ligne canonique « Snapshot: <fixture> — <raison> » (CONTRIBUTING §4). L'historique poussé n'est pas réinscriptible : étends le registre rétroactif docs/snapshots-provenance.md (créé par J01-P5) avec une entrée « Snapshot: » par fixture concernée, commit d'origine, référence revue et décisions. (b) SHOULD-FIX — conformance/cases/stat/npar1way-wilcoxon-out/case.json : normalise le champ « issue » de la promotion (« 19 » → « sasrs #19 », cohérent avec les autres promotions J02). Régénère conformance/STATUS.md et vérifie --check. Ne change rien d'autre.

Tier: T5 · Depends: J02-P4, J02-P7, J01-P5 · Checks: conformance-status

## J03 — Déterminisme + clôture (issue #20, méta #10/#11)

### J03-P1 — Déterminisme UNION et sidecar (issue #20)
(a) UNION (non-ALL) : tri déterministe des lignes après unique() — tri total et stable (clé définie sur toutes les colonnes, ordre des colonnes fixe) sinon l'instabilité persiste sur les ex æquo ; (b) sidecar <t>.parquet.sasmeta.json : sérialisation à clés triées (BTreeMap) au lieu de la HashMap. Acceptance exécutable spécifique : créer tests/determinism/ contenant un reproducer SAS (UNION non-ALL + écriture d'une table avec sidecar) et le script run_check.sh qui exécute le binaire sasrs (cargo run --locked -p sasrs, via distrobox enter ombre-mingw) 5 fois dans des répertoires temporaires distincts et compare byte-à-byte via cmp tous les artefacts produits (log, print, table parquet et sidecar) — exit 0 seulement si tous identiques. Le check j03-determinism appelle bash tests/determinism/run_check.sh. En plus : cargo test --test differential passe. Aucune promotion de cas conformance pour cette unité (aucun cas ne couvre #20).

Tier: T4 · Depends: J02-P4 · Checks: fmt, clippy, test, test-conformance, conformance-status, j03-determinism

### J03-P2 — Clôture documentaire #10/#11 (méta)
Documentation seulement : s'assurer que tous les case.json des cas promus référencent leur numéro d'issue GitHub dans le champ "issue" (les unités J01/J02 le font déjà ; compléter si besoin), régénérer conformance/STATUS.md (python3 scripts/conformance_report.py) et vérifier --check. Ne touche ni .mission-control/ ni docs/plans/. La clôture GitHub elle-même est une action du coordinateur, pas de cette unité. Le cas stat/reg-simple-lineart reste known-divergence (hors mandat, sans issue ouverte) : ne pas le modifier.

Tier: T5 · Depends: J01-P1, J01-P2, J01-P3, J02-P1, J02-P2, J02-P3, J02-P4, J03-P1, J02-P6, J02-P8 · Checks: conformance-status

### J03-P3 — Revue finale
Revue indépendante du diff intégré du jalon (verify-before-done, code-design, test-design) : rejouer tous les checks sur le commit épinglé ; vérifier que chaque promotion de cas est justifiée par la provenance doc SAS, que chaque snapshot modifié est justifié (ligne Snapshot: dans le commit), qu'aucun silent-behavior n'est introduit et qu'aucune documentation ne promet plus que le code. Ne rien modifier ; tout finding bloquant devient une unité corrective. Vérifie le Goal complet : 11 divergences initiales → 0 restante pour #13–#20 (le cas stat/reg-simple-lineart restant known-divergence sans issue ouverte doit être signalé dans le rapport de revue, non corrigé hors mandat) ; STATUS.md cohérent ; aucune promesse de couverture au-delà du validé.

Tier: T2 · Depends: J01-P4, J02-P5, J03-P1, J03-P2 · Checks: fmt, clippy, clippy-s3, test, test-conformance, conformance-status, coverage-claims, ci-structure, j03-determinism

