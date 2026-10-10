# Contrat de support des instructions de procédure

Une instruction reconnue mais non honorée ne doit jamais provoquer de repli
silencieux. Le contrat de sévérité est fixé par `CONTRIBUTING.md`, §5 ; les tests
`contract_*` en sont les tests de non-régression. Il décrit le support de sasrs,
pas une promesse que toutes les procédures de SAS sont implémentées.

| Effet de l'instruction ignorée | Diagnostic | Exécution de la PROC |
| --- | --- | --- |
| Peut changer un résultat numérique, un dataset ou le contrôle de flux | **ERROR** | Rejet de l'étape avant son exécution |
| Limité à une personnalisation d'affichage non implémentée | **WARNING** | Poursuite, affichage par défaut |
| Instruction inconnue ou non valide dans ce contexte | **ERROR**, code **180-322** | Rejet de l'étape avant son exécution |

Une instruction effectivement implémentée conserve sa sémantique : par exemple
BY dans SORT, WEIGHT dans MEANS, OUTPUT dans LOGISTIC et WHERE dans PRINT. Le
contrat est appliqué par procédure, après recherche d'un handler implémenté.
Aucun dataset partiel ne doit être créé par une PROC rejetée au parsing. Le
programme peut continuer à l'étape suivante. Un WARNING donne un code de sortie
1 ; une ERROR donne 2 (hors demande explicite de sortie par `%ABORT`).

## Options et instructions passées d'« ignoré » à ERROR/WARNING en J02

Avant le jalon J02, plusieurs demandes reconnues étaient consommées sans
diagnostic, voire avec une NOTE trompeuse. Elles produisent désormais un
diagnostic honnête (réf. merges `2ad236e`, `965e578`, `d4eec6f`) :

| PROC / statement | Demande | Avant | Depuis J02 |
| --- | --- | --- | --- |
| COMPARE | option inconnue | ignorée en silence | **ERROR** — depuis J07, toutes les options reconnues sont réellement implémentées : `CRITERION=`/`METHOD=`, `OUTBASE=`/`OUTCOMP=`/`OUTDIF=`/`OUTNOEQUAL=`/`OUTPERCENT=`, `BRIEF`(SUMMARY), `LISTALL`, `NOVALUES`, `NOPRINT` et `MAXPRINT=n\|(n,p)` (plafonne la section « Value Comparison Results » à n différences par observation et p observations avec différences, défauts 50/50 ; NOTE en cas de troncature), ainsi que les instructions `ID`, `VAR`/`WITH`, `BY` (cas `compat/compare/*` validés, cf. [`conformance/STATUS.md`](../conformance/STATUS.md)) |
| UNIVARIATE | `VARDEF=` / `PCTLDEF=` autres que DF / définition 5 | ignorées | **ERROR** |
| FREQ | option inconnue d'un statement `TABLES` | skip silencieux | **ERROR** |
| DATASETS | `KILL` ; options inconnues de l'en-tête et de `COPY` | ignorées | **ERROR** |
| MEANS/SUMMARY | statistique non calculable nommée dans `OUTPUT` (ex. `clm(x)=`) | colonne missing silencieuse | **ERROR** |
| SQL | `OUTOBS=` / `INOBS=` ; instruction inconnue | ignorées | **ERROR** |
| PRINTTO | (levé en J07 — voir §J07) | NOTE trompeuse « redirected to » | **WARNING** jusqu'à J07 (« routing not supported ») ; depuis J07, `LOG=`/`PRINT=` routent réellement le journal / le listing vers le fichier, `NEW` remplace le contenu existant et un `PROC PRINTTO;` nu retablit les destinations par défaut (cas `compat/printto/*` validés) |
| PLOT | options d'affichage après `/` (`HREF=`, `VREF=`, `HAXIS=`, …) ; `=group` | skip silencieux / désynchronisation du run-group | **WARNING** par option ; `=group` → NOTE (une seule couleur de symbole) |
| GENMOD | `DIST=`/`LINK=` inconnue, option MODEL inconnue (ex. `OFFSET=`) | repli silencieux sur NORMAL/IDENTITY | **ERROR** |
| LOGISTIC | `ORDER=` et options PROC inconnues ; `LINK=` inconnue ; options MODEL inconnues ; `PARAM=` autre que REF | repli silencieux | **ERROR** |
| MIXED / GLIMMIX | `METHOD=` / `TYPE=` inconnus ; `DDFM=` autre que CONTAIN | repli silencieux (GLIMMIX avalait `DDFM=` entier) | **ERROR** |
| GLIMMIX | `METHOD=QUAD` | NOTE de différé | **ERROR** (utiliser RSPL ou LAPLACE) |
| GLM / ANOVA | design de rang incomplet (X'X singulière) | lignes NaN imprimées en silence | **ERROR** |
| FACTOR | `ROTATE=QUARTIMAX` / `ROTATE=OBLIMIN` | repli silencieux sur la rotation par défaut | **ERROR** (utiliser VARIMAX, PROMAX ou NONE) |
| DISCRIM | `METHOD=` autre que NORMAL, `POOL=NO\|TEST`, `POOL=` inconnu | repli silencieux LDA + NOTE | **ERROR** |
| GENMOD / LOGISTIC / MIXED / GLIMMIX | non-convergence, séparation, G non définie positive | « converged » imprimé sans vérification, NOTE | **WARNING** SAS-fidèle, message de non-convergence véridique |

## Replis silencieux supprimés (roadmap-avancee J02)

Constructions que le parseur ou l'exécution ignoraient ou approximaient sans
diagnostic (audit `d0b4d90`). Chaque ligne est figée par un test
`ra_<unité>_<sujet>` ; « Levée par » nomme l'unité du plan roadmap-avancee qui
remplacera l'ERROR/WARNING provisoire par l'implémentation (— : correction
directe, rien à lever).

| Unité | PROC / construction | Avant | Après | Levée par |
| --- | --- | --- | --- | --- |
| J02-P1 | LOGISTIC / GENMOD : option de réponse `y(DESC)` | ignorée (seul `DESCENDING` lu) | honorée comme `DESCENDING` (doc SAS 9.4, MODEL « Response Variable Options ») | — |
| J02-P1 | LOGISTIC / GENMOD : `y(EVENT=FIRST\|LAST)` | avalée, événement par défaut | honorée : premier / dernier niveau ordonné (après `DESCENDING`) | — |
| J02-P1 | LOGISTIC / GENMOD : `y(ORDER=…)`, `y(REF=…)`, valeur `EVENT=` non citée, autre option de réponse | avalées | **ERROR** au parsing | non planifiée (`REF=` LOGISTIC : J05-P5) |
| J02-P1 | LOGISTIC : `CLASS … / PARAM= REF=` | options lues comme noms de variables, codage REF=LAST | même sémantique que la forme parenthésée (`PARAM=REF\|REFERENCE`, `REF=FIRST\|LAST`, l'option parenthésée prime) ; autres options → même **ERROR** | J05-P2 (PARAM=EFFECT/GLM, `REF='niveau'`) |
| J02-P1 | LOGISTIC : CLASS sans `PARAM=` | codage REF appliqué en silence (défaut SAS : EFFECT) | **WARNING** « coded with PARAM=REF … estimates differ from SAS, the odds ratios are identical » (code 1) | J05-P2 |
| J02-P1 | LOGISTIC `OUTPUT` : `LOWER=`, `UPPER=`, `STDXBETA=`, `RESCHI=`, `RESDEV=`, `H=`, `PREDPROBS=`, autres mots-clés | sautés jeton par jeton | **ERROR** | J05-P4 |
| J02-P1 | LOGISTIC : `OUTPUT` sans `OUT=` | instruction abandonnée, aucun dataset | **ERROR** | non planifiée |
| J02-P1 | LOGISTIC ordinal : inversion de la matrice d'information en échec | SE, Wald et p imprimés NaN | **ERROR** explicite (matrice d'information singulière) | J05-P7 (Hessien exact) |
| J02-P1 | LOGISTIC / GENMOD : niveaux CLASS | calculés sur toutes les lignes lues (colonne nulle, ERROR de singularité trompeuse) | calculés sur les observations utilisées (doc SAS, « Missing Values ») | — |
| J02-P1 | LOGISTIC (binaire et ordinal) / GENMOD : statut de convergence | « Convergence criterion (GCONV=1E-8) satisfied. » alors que le critère testé est le changement relatif des paramètres | « Convergence criterion (XCONV=1E-8) satisfied. » | J05-P6 (GCONV=/XCONV= SAS) |
| J02-P1 | GENMOD sans `DIST=` | modèle de Poisson (lien log) | loi NORMAL, lien IDENTITY (défaut SAS) | — |
| J02-P1 | GENMOD : option PROC `DESCENDING` | sautée | honorée (ordre des niveaux de réponse) | — |
| J02-P1 | GENMOD : autre option PROC que `DATA=` / `DESCENDING` | sautée | **ERROR** « Unexpected option '…' on PROC GENMOD statement. » | non planifiée |
| J02-P1 | GENMOD `CLASS` : options `(REF= PARAM= ORDER= DESC MISSING …)` ou `/ …` | sautées (mots pris pour des variables) | **ERROR** | J07-P2 |
| J02-P1 | GENMOD : `SCALE=PEARSON\|P\|DEVIANCE\|D` ; `SCALE=<n>` sous POISSON/BINOMIAL | ignorés | **ERROR** (`SCALE=<n>` reste honoré sous NORMAL/GAMMA) | J07-P2 |
| J02-P1 | GENMOD `DIST=GAMMA` : réponse ≤ 0 | tronquée à 1e-300 dans la vraisemblance et la déviance | **ERROR** | — |
| J02-P1 | LOGISTIC : `CODE`, `EFFECT`, `EXACT`, `EXACTOPTIONS`, `LSMESTIMATE`, `NLOPTIONS`, `ODDSRATIO`, `ROC`, `ROCCONTRAST`, `SCORE`, `SLICE`, `STORE`, `STRATA`, `TEST`, `UNITS` | « 180-322 … not valid » | **ERROR** « not supported … cannot be ignored » ; `EFFECTPLOT` (graphique seul) → **WARNING** d'affichage | J05-P3 (`ODDSRATIO`), sinon non planifiée |
| J02-P1 | GENMOD : `ASSESS`, `BAYES`, `CODE`, `DEVIANCE`, `EFFECT`, `EXACT`, `EXACTOPTIONS`, `FWDLINK`, `INVLINK`, `LSMESTIMATE`, `REPEATED`, `SLICE`, `STORE`, `STRATA`, `VARIANCE`, `ZEROMODEL` | « 180-322 … not valid » | **ERROR** « not supported … cannot be ignored » ; `EFFECTPLOT` → **WARNING** d'affichage | J07-P5 (`REPEATED`), sinon non planifiée |
| J02-P2 | MIXED : `TYPE=xx(n)` autre que `AR(1)` (ex. `UN(1)` à bande, `CS(2)`, `AR(2)`) | parenthèse avalée, structure complète ajustée | **ERROR** « TYPE=UN(1) (parameterized covariance structure) is not supported … » | non planifiée |
| J02-P2 | MIXED : plusieurs instructions `RANDOM` | la dernière gagnait | **ERROR** | J06-P2 |
| J02-P2 | MIXED : `RANDOM` + `REPEATED` | `RANDOM` abandonné, seule la structure R ajustée | **ERROR** | J06-P2 |
| J02-P2 | MIXED : `SUBJECT=id(grp)` / `SUBJECT=a*b` (RANDOM, REPEATED) | premier identifiant seul (sujets fusionnés) | **ERROR** | J06-P2 |
| J02-P2 | MIXED `RANDOM` / `REPEATED` : `GROUP=`, `LOCAL` et toute option autre que `SUBJECT=`/`SUB=`/`TYPE=` | sautées | **ERROR** ; `G`, `GC`, `GCI`, `GCORR`, `GI`, `V`, `VC`, `VCI`, `VCORR`, `VI`, `SOLUTION`, `CL`, `ALPHA=` (RANDOM) et `R`, `RC`, `RCI`, `RCORR`, `RI` (REPEATED) → **WARNING** d'affichage (code 1) | J06-P2 (`GROUP=`, `LOCAL`) |
| J02-P2 | MIXED `REPEATED effet` : effet répété | sauté (R indexée par ordre d'apparition) | lu ; **ERROR** si ses niveaux ne sont pas strictement croissants dans un sujet, s'il manque des niveaux (TYPE=UN : niveaux 1..k ; AR(1) : niveaux consécutifs) ou s'il est manquant ; cas complet et trié inchangé ; effet composé → **ERROR** | J06-P2 |
| J02-P2 | MIXED : options PROC `CONVG=`, `CONVH=`, `MAXITER=`, `ORDER=`, `EMPIRICAL`, `SCORING=`, `NOPROFILE`, autres options valides ou inconnues | sautées | **ERROR** (« The CONVG= option is not supported … » ; option inconnue : « Unexpected option »), `CL` → **WARNING** d'affichage | non planifiée |
| J02-P2 | MIXED : `COVTEST`, `ASYCOV` | NOTE « parse-accepted but not implemented » puis ignorés | **ERROR** | J06-P3 |
| J02-P2 | MIXED `MODEL` : `OUTP=`, `OUTPM=`, `NOFIT` et autres options que `SOLUTION`/`NOINT`/`DDFM=CONTAIN` | sautées (aucun dataset créé ; `NOFIT` NOTE sur legacy, ignoré sur le chemin général) | **ERROR** | non planifiée |
| J02-P2 | MIXED (chemin général) : « Iteration History » | synthétique : 2 lignes, critère 0.00000000 inventé, compte Nelder-Mead présenté comme évaluations | retirée ; seul le statut de convergence réel reste imprimé | J06-P2 (historique réelle) |
| J02-P2 | MIXED (chemin legacy) `METHOD=ML` : en-tête de l'Iteration History | « -2 Res Log Like » | « -2 Log Like » (doc SAS 9.4, MIXED, Iteration History) | — |
| J02-P2 | MIXED : `NOINT` avec un effet CLASS | dernier niveau retiré malgré NOINT (colonne manquante) | **ERROR** | J06-P2 |
| J02-P2 | MIXED : `NOBOUND` | NOTE « not implemented » fausse sur le chemin legacy (honoré par la forme close) ; ignoré en silence sur le chemin général et sur données déséquilibrées (λ ≥ 0) | legacy équilibré : honoré sans NOTE ; chemin général et legacy déséquilibré : **ERROR** | J06-P2 |
| J02-P2 | MIXED : `CODE`, `LSMESTIMATE`, `PARMS`, `PRIOR`, `SLICE`, `STORE` | « 180-322 … not valid » | **ERROR** « not supported … cannot be ignored » | non planifiée |
| J02-P2 | MIXED : NOTE « parse-accepted » LSMEANS/ESTIMATE/CONTRAST/DDFM/REPEATED/NOFIT | code mort (instructions déjà rejetées au parsing) et en-tête du module obsolète | retirés | — |
| J02-P3 | GLIMMIX : plusieurs instructions `RANDOM` | la dernière gagnait | **ERROR** | J08-P4 |
| J02-P3 | GLIMMIX : `DIST=NORMAL` avec un `LINK=` autre que IDENTITY et un effet `RANDOM` G-side sous `METHOD=RSPL` | lien ignoré (modèle à lien identité ajusté par le solveur à composantes de variance) | **ERROR** ; sans `RANDOM` (mode GLM, IRLS avec le lien) inchangé | J08-P2 |
| J02-P3 | GLIMMIX `METHOD=LAPLACE` : paire DIST/LINK non canonique hors loi binaire (`POISSON`/`IDENTITY`, `NORMAL`/`LOG`…) | vraisemblance de Bernoulli appliquée (branche par défaut de la log-densité) | **ERROR** ; liens binaires (`PROBIT`, `CLOGLOG`, vraisemblance de Bernoulli exacte) inchangés | J08-P3 |
| J02-P3 | GLIMMIX : instruction `WEIGHT` | NOTE « parse-accepted but not implemented », poids ignorés | **ERROR** | J08-P4 |
| J02-P3 | GLIMMIX : `FREQ` avec `DIST=NORMAL`, `LINK=IDENTITY` et un `RANDOM` sous `METHOD=RSPL` | fréquences ignorées par les solveurs REML (composantes de variance, structure R) alors que « Number of Observations Used » affichait leur somme | **ERROR** ; `FREQ` reste honoré en mode GLM, par la boucle PQL et par LAPLACE | non planifiée |
| J02-P3 | GLIMMIX : options PROC autres que `DATA=` / `METHOD=` (`NOBOUND`, `EMPIRICAL`, `PCONV=`, `ORDER=`, `NOREML`, `MAXOPT=`, `CHOLESKY`, `SCORING=`, `OUTDESIGN=`, `NOFIT`, `IC=PQ\|Q`…), option inconnue, `METHOD=` sans valeur | sautées (`METHOD=` vide : RSPL) | **ERROR** (« The NOBOUND option is not supported … » ; inconnue : « Unexpected option ») ; `ASYCORR`, `ASYCOV`, `GRADIENT`, `HESSIAN`, `ITDETAILS`, `LIST`, `NAMELEN=`, `NOBSDETAIL`, `NOCLPRINT`, `ODDSRATIO`, `PLOTS=`, `IC=NONE` sous LAPLACE → **WARNING** d'affichage (code 1) ; honorées sans diagnostic : `INITGLM` (tous les ajustements partent du GLM sans effet aléatoire), `NOITPRINT` et `PLOTS=NONE` (aucune Iteration History ni aucun graphique produit), `IC=NONE` hors LAPLACE (aucun critère d'information imprimé, défaut SAS des méthodes PL) | non planifiée |
| J02-P3 | GLIMMIX : statut de convergence | « Convergence criterion (GCONV=1E-8) satisfied. » pour tout ajustement (aucun critère de gradient testé) ; `converged = true` codé en dur pour NORMAL + intercept aléatoire | critère réellement testé : `XCONV=1E-10` (IRLS du mode GLM), `PCONV=1E-6` (boucle de pseudo-vraisemblance), « Nelder-Mead simplex: FTOL=1E-12, XTOL=1E-10 » (LAPLACE, structure R NORMAL), « golden-section search: XTOL=1E-10 » (recherche de λ = σ²u/σ²e), « Closed-form REML solution (balanced data): no iteration required. » (forme close) ; convergence réelle de la recherche | J08-P2 / J08-P3 (critères et optimiseurs SAS) |
| J02-P3 | GLIMMIX : « Iteration History » | synthétique : 2 lignes répétant l'objectif final, « Change » 0.00000000 et nombres d'évaluations inventés | retirée ; seul le statut de convergence réel reste imprimé | J08-P2 / J08-P3 (historique réelle) |
| J02-P3 | GLIMMIX : σ²u négative (forme close) ou sur la frontière λ = 0 ; λ = σ²u/σ²e plafonné à 1000 | tronquée à 0 / plafonné en silence | NOTE « Estimated G matrix is not positive definite. » (texte SAS) ; NOTE « The variance component ratio search reached its boundary (lambda=1000) in PROC GLIMMIX; the estimate may be unreliable. » | — |
| J02-P3 | GLIMMIX : `RANDOM INTERCEPT / TYPE=AR(1)\|UN` | réinterprété en structure R sans effet aléatoire | **ERROR** (structure G-side) ; la structure R existante est exposée sous la syntaxe SAS `RANDOM _RESIDUAL_ / SUBJECT= TYPE=AR(1)\|UN` (alias `_RESID_`, `RESID`, `RESIDUAL` ; observations d'un sujet dans leur ordre d'apparition) ; `_RESIDUAL_` avec `TYPE=VC\|CS` ou combiné à d'autres effets, `TYPE=xx(n)` autre que `AR(1)`, `TYPE=` sans valeur → **ERROR** | J08-P4 (structures G-side) |
| J02-P3 | GLIMMIX : table « Type III Tests of Fixed Effects » | une ligne par paramètre (Intercept et chaque colonne de codage CLASS, F = t²) | une ligne par effet, sans ligne Intercept (défaut SAS) ; effet à un paramètre : F = t² (Wald, 1 ddl) ; effet à plusieurs paramètres : table retirée + NOTE ; modèle sans effet : pas de table | J08-P2 |
| J02-P3 | GLIMMIX `MODEL` : `OFFSET=`, `OBSWEIGHT=`, `DDF=`, `NOCENTER`, `LWEIGHT=`, `ZETA=`, `REFLINP=`, `HTYPE=` autre que 3, option inconnue ; `DIST=` / `LINK=` sans valeur | sautés (NORMAL / lien canonique par défaut) | **ERROR** ; `ALPHA=`, `CHISQ`, `CL`, `CORRB`, `COVB`, `COVBI`, `E`/`E1`/`E2`/`E3`, `INTERCEPT`, `ODDSRATIO`, `STDCOEF` → **WARNING** d'affichage ; `HTYPE=3` (défaut) accepté | non planifiée |
| J02-P3 | GLIMMIX : `NOINT` avec un effet CLASS | dernier niveau retiré malgré NOINT (colonne manquante) | **ERROR** | J08-P2 |
| J02-P3 | GLIMMIX sans `RANDOM` (mode GLM), `METHOD=LAPLACE` compris | « Estimation Technique : Residual PL » | « Maximum Likelihood » (« Restricted Maximum Likelihood » pour la loi normale) : en mode GLM, METHOD= est sans effet (doc SAS 9.4, « GLM Mode or GLMM Mode », « Default Estimation Techniques ») | — |
| J02-P3 | GLIMMIX `RANDOM` : `GROUP=`, `RESIDUAL`, `NOFULLZ`, `LDATA=`, `GCOORD=`, `KNOTMETHOD=`, `WEIGHT=`, option inconnue ; `SUBJECT=id(grp)` / `SUBJECT=a*b` | sautées / premier identifiant seul (sujets fusionnés) | **ERROR** ; `SOLUTION`, `G`, `GC`, `GCI`, `GCORR`, `GI`, `V`, `VC`, `VCI`, `VCORR`, `VI`, `CL`, `ALPHA=`, `KNOTINFO` → **WARNING** d'affichage | J08-P4 (`SUBJECT=` emboîté ou croisé) |
| J02-P3 | GLIMMIX `CLASS` : options `(REF= ORDER= DESC …)` ou `/ …`, liste `a1-a3` | sautées (mots pris pour des variables CLASS, `a2` perdu) | **ERROR** | non planifiée |
| J02-P3 | GLIMMIX : `CODE`, `COVTEST`, `EFFECT`, `LSMESTIMATE`, `NLOPTIONS`, `PARMS`, `SLICE`, `STORE` | « 180-322 … not valid » | **ERROR** « not supported … cannot be ignored » | non planifiée |
| J02-P3 | GLIMMIX : NOTE « parse-accepted » ESTIMATE/CONTRAST/LSMEANS/WEIGHT, en-tête du module | code mort (instructions déjà rejetées au parsing) et doc obsolète | retirés | — |
| J02-P4 | DISCRIM : options PROC inconnues ou non implémentées (`TESTDATA=`, `TESTOUT=`, `OUTCROSS=`, `OUTD=`, `CROSSLIST`, `CANONICAL`, `THRESHOLD=`, `SINGULAR=`…), options de table de `DATA=`/`OUT=`, `POOL=` sans valeur | sautées jeton par jeton (`POOL=` vide : YES) | boucle d'options commune : option inconnue → **ERROR** « Unexpected option 'X' on PROC DISCRIM statement. » ; option SAS valide qui peut changer un résultat ou créer une table → **ERROR** « The X option is not supported in PROC DISCRIM; it can affect results and cannot be ignored » ; options d'affichage (`NOPRINT`, `SIMPLE`, `ALL`, `LISTERR`, `POSTERR`…, statistiques F de `DISTANCE`) → **WARNING** d'affichage (code 1) ; options de table entre parenthèses → **ERROR** ; `POOL=` vide → **ERROR** ; `DATA=`, `OUT=`, `METHOD=NORMAL`, `POOL=YES`, `LIST` honorés, `PCOV`/`WCOV` aussi (matrices toujours imprimées) | J09-P2 (`CROSSLIST`, `TESTDATA=`, `TESTOUT=`), sinon non planifiée |
| J02-P4 | DISCRIM : `PRIORS` à probabilités explicites (`'A'=.3 'B'=.7`, `A=.3 B=.7`) et toute forme autre que `EQUAL`/`PROPORTIONAL`/`PROP` | remplacé par EQUAL | **ERROR** « PRIORS with explicit probabilities is not supported … » (forme invalide : ERROR de syntaxe) ; EQUAL et PROPORTIONAL inchangés | J09-P2 |
| J02-P4 | DISCRIM : `CROSSVALIDATE`, `OUTSTAT=` | NOTE « parse-accepted but not implemented » puis exécution sans validation croisée, aucune table OUTSTAT= | **ERROR** | J09-P2 |
| J02-P4 | DISCRIM : `NOCLASSIFY`, `SHORT` | NOTE « parse-accepted » puis ignorés | **WARNING** d'affichage (code 1), listing inchangé | non planifiée |
| J02-P4 | DISCRIM : `CLASS a b`, `ID a b` ; CLUSTER : `ID a b` ; DISCRIM : `VAR x1-x3` | première variable seule ; la boucle VAR de DISCRIM sautait les jetons non-noms (`x2` perdu) | **ERROR** « The CLASS statement of PROC DISCRIM takes a single variable; found A B. » (SAS 9.4 : `CLASS variable;`, `ID variable;`) ; liste VAR stricte (plage → ERROR de syntaxe) | — |
| J02-P4 | DISCRIM : table « Classification Results for Training Data » (une ligne par observation) | toujours imprimée | imprimée seulement avec `LIST` (doc SAS 9.4, PROC DISCRIM statement, LIST : « displays the resubstitution classification results for each observation ») ; « Error Count Estimates » inchangé | — |
| J02-P4 | PRINCOMP / FACTOR : table au format TYPE=CORR/COV (variables caractère `_TYPE_` et `_NAME_`, lignes `CORR`/`COV`/`UCORR`/`UCOV`/`SSCP`) | analysée comme des observations brutes | **ERROR** « A TYPE=CORR/COV input data set (… has _TYPE_ and _NAME_ variables) is not supported … » | J09-P5 (FACTOR : voir le constat ci-dessous) |
| J02-P4 | PRINCOMP / FACTOR (matrice de corrélation) : variable de variance nulle | corrélations forcées à 0, diagonale 1 (valeur propre inventée) | **ERROR** nommant la variable ; analyse `COV` inchangée | J09-P5 (comportement documenté) |
| J02-P4 | `stat::linalg` Jacobi (PRINCOMP, FACTOR, REG `RIDGE=`/`COLLIN`, `MTEST`, IML `EIGVAL`/`EIGVEC`) : entrée manquante/infinie, non-convergence | dernière itérée rendue en silence (valeurs propres NaN) | **ERROR** typée `SasError::Numerical` (« numerical error: matrix has missing or infinite entries (Jacobi) », « … did not converge after 100 sweeps … ») ; résidu hors diagonale accepté s'il est < 1e-15 ou ≤ 1e-12·‖A‖ (spectres quasi dégénérés à grande échelle) | — |
| J02-P4 | FACTOR : `ROTATE=VARIMAX\|PROMAX` avec un seul facteur retenu | rotation sautée en silence | **NOTE** « Only one factor is retained in PROC FACTOR; the ROTATE=VARIMAX rotation, which needs at least two factors, is not performed. » | — |
| J02-P4 | FACTOR : VARIMAX (et pré-rotation de PROMAX) arrêtée au plafond de 1000 balayages | motif présenté comme convergé | **WARNING** « The VARIMAX rotation did not converge after 1000 iterations in PROC FACTOR; the rotated factor pattern may be inaccurate. » | — (J09-P4 : critère relatif SAS) |
| J02-P4 | FACTOR : `OUT=` (libref non assigné, matrice de corrélation/covariance singulière), échec de PROMAX | ERROR après l'impression du listing | rotation et `OUT=` validés avant toute sortie : **ERROR** sans listing (« The correlation matrix is singular; PROC FACTOR cannot compute the OUT= factor scores. ») | — |
| J02-P4 | CLUSTER / FASTCLUS / DISTANCE : valeur manquante d'une variable VAR | propagée en NaN (distances, graines, centroïdes, OUTTREE=/OUT=) | **ERROR** « A missing value in a VAR variable (X, observation 3) is not supported … » avant toute sortie | J09-P6 (CLUSTER), J09-P7 (FASTCLUS, DISTANCE) |
| J02-P4 | FASTCLUS : `SEED=<nombre>` ; `SEED=<table>` | nombre accepté (NOTE) puis ignoré, graines farthest-first ; table : ERROR « expected a number » | **ERROR** « SEED= names a SAS data set of initial cluster seeds in PROC FASTCLUS, not a number. » (doc SAS 9.4) ; table → **ERROR** « not supported » | J09-P7 (`SEED=<table>`) |
| J02-P4 | FASTCLUS : semis farthest-first, `MAXITER=10` par défaut, `MAXCLUSTERS=` requis | divergence silencieuse avec SAS | **NOTE** à chaque exécution ; section « Approximations documentées » ci-dessous | J09-P7 |
| J02-P4 | DISTANCE : `METHOD=COSINE` / `METHOD=CORR` | dissimilarités 1 − cos / 1 − r (0 si indéfinies, diagonale 0) | similarités de la doc PROC DISTANCE (« Proximity Measures » : s20 cosinus, s8 corrélation, TYPE=SIMILAR) : diagonale s(x,x) = 1, `_TYPE_` = « SIMILAR », listing « Similarity Matrix » ; dénominateur nul → **ERROR** | — (dénominateur nul : J09-P7) |
| J02-P4 | DISCRIM `TESTCLASS`/`TESTFREQ`/`TESTID` ; PRINCOMP `PARTIAL` ; FACTOR `PARTIAL`/`PRIORS` ; CLUSTER `COPY`/`RMSSTD` ; DISTANCE `COPY` | « 180-322 … not valid » | **ERROR** « not supported … cannot be ignored » (FASTCLUS `ID`/`BY`/`FREQ`/`WEIGHT` : déjà le message partagé) ; FACTOR `PATHDIAGRAM` (graphique seul) → **WARNING** d'affichage | J09-P5 (`PARTIAL` de PRINCOMP), sinon non planifiée |
| J02-P4 | DISCRIM, CLUSTER, FASTCLUS, PRINCOMP : NOTE « parse-accepted » (OUTSTAT=, NOCLASSIFY, CROSSVALIDATE, SHORT), NOTE « SEED=… is accepted », en-têtes de module (OUTTREE= « parse-accepté », OUT= de PRINCOMP « scores not produced », `let _ = ast.id`) | doc et NOTEs contredisant le code | corrigés / retirés | — |
| J02-P5 | IML : options du statement PROC IML | sautées jeton par jeton (`proc iml foo=bar;` exécuté) | **ERROR** « Unexpected option 'FOO' on PROC IML statement. » (étape rejetée, l'étape suivante s'exécute) ; `SYMSIZE=n` / `WORKSIZE=n`, seules options SAS 9.4 (tailles mémoire, étendues automatiquement : doc « PROC IML Statement », « Memory and Workspace »), acceptées sans effet observable ; valeur non numérique → **ERROR** | — |
| J02-P5 | IML : options PRINT `[COLNAME= ROWNAME= FORMAT= LABEL=]` (abréviations `C=` `R=` `F=` `L=`) | lues puis abandonnées | **WARNING** d'affichage par option (code 1), listing inchangé ; tout autre contenu entre crochets → **ERROR** | non planifiée |
| J02-P5 | IML : erreur d'exécution | toute la sortie PRINT déjà produite perdue | sortie des instructions exécutées avant l'erreur rendue (SAS imprime au fil de l'exécution) ; une instruction PRINT en erreur n'imprime rien ; sans aucun PRINT, plus de page « The IML Procedure » vide ; l'étape s'arrête à la première erreur (**ERROR**, code 2 ; SAS reprendrait à l'instruction suivante) | — (reprise après erreur : non planifiée) |
| J02-P5 | IML : `CREATE`/`APPEND` sans `CLOSE` ; `CREATE x` puis `CLOSE work.x` | table jamais écrite (`X` et `WORK.X` traités comme deux tables) | tables encore ouvertes fermées et écrites à QUIT, dans l'ordre des CREATE (doc SAS 9.4, CLOSE statement : « automatically closes all open data sets when a QUIT statement is executed ») ; noms à un niveau normalisés en `WORK.` ; valeur manquante écrite comme manquant SAS (null), plus en NaN ; après une erreur d'exécution, tables ouvertes non écrites (aucune table partielle) avec **WARNING** « The data set WORK.W was not written because PROC IML stopped at an execution error. » | — |
| J02-P5 | IML : `SOLVE(A, b)` avec A non carrée | moindres carrés en silence, système incohérent compris | **ERROR** « IML: SOLVE requires a square matrix (got 3x2). » (doc SAS 9.4, SOLVE : « The matrix A must be square and nonsingular ») ; système carré inchangé | — (seuil de singularité relatif : J10-P4) |
| J02-P5 | IML : `MEAN`, `STD` ; `SUM`, `MIN`, `MAX` ; arguments surnuméraires | MEAN/STD réduits à un scalaire sur tous les éléments ; SUM/MIN/MAX ignorant les arguments après le premier ; autres fonctions (ABS(x, y)…) ignorant leurs arguments en trop ; manquants propagés | MEAN/STD par colonne, vecteur ligne 1×p, manquants exclus, STD manquant sous deux observations (doc SAS 9.4 MEAN, STD : exemples publiés reproduits) ; SUM/MIN/MAX sur tous leurs arguments (≤ 15), manquants exclus (tout manquant : SUM 0, MIN/MAX le plus grand / le plus négatif nombre représentable) ; nombre d'arguments faux → **ERROR** au parsing ; méthode de MEAN (`trimmed`, `winsorized`), pad-value et forme à un argument de SHAPE → **ERROR** « not supported » | — (méthode de MEAN, pad-value de SHAPE : non planifiées) |
| J02-P5 | IML : division par zéro, `LOG(x ≤ 0)`, `SQRT(x < 0)` ; manquants dans les opérations matricielles | inf / -inf / NaN silencieux, propagés dans INV, SOLVE, DET… | division : **WARNING** « Division by zero, result set to missing value. » (`operation : /`) et valeur manquante (doc SAS 9.4, Division Operator) ; LOG/SQRT hors domaine : **ERROR** « (execution) Invalid argument to function. » (`operation : LOG`) ; manquant en entrée de INV, SOLVE, DET, CHOL, CALL QR, CALL SVDCD ou du produit matriciel `*` : **ERROR** « (execution) Invalid argument or operand; contains missing values. » (doc SAS 9.4, Missing Values) ; opérations élément par élément : manquant propagé (doc) | — |
| J02-P5 | IML : EIGVAL / EIGVEC / CALL EIGEN d'une matrice non symétrique | « ERROR: ERROR: The argument to the EIGVAL function must be a symmetric matrix. » | un seul préfixe `ERROR:` | — |
| J02-P5 | IML : instruction inconnue ; `BY` ; erreurs de syntaxe | lue comme une affectation : « IML: expected '=' in an assignment, found Ident("x") » (jeton Rust) | repli partagé `common::unhandled_proc_statement` : inconnue → **ERROR** 180-322 « Statement 'X' is not valid or it is used out of proper order in PROC IML. » ; BY (IML n'en a pas), WHERE, WEIGHT, FREQ, CLASS, ID… → message partagé « not supported » ; FORMAT/LABEL → **WARNING** d'affichage ; une affectation `by = …` reste une affectation ; erreurs de syntaxe : texte source du jeton (« found ';' ») | — |
| J02-P5 | IML : valeur manquante imprimée | « NaN » | « . » (doc SAS 9.4, Missing Values : « a numeric missing value is specified as a single period ») | — |
| J02-P5 | IML : instructions SAS/IML valides non implémentées — START, FINISH, RUN, RETURN ; ABORT, CLOSEFILE, DELETE, DISPLAY, DO DATA, EDIT, FILE, FIND, FORCE, FREE, GOTO, INDEX, INFILE, INPUT, LINK, LIST, LOAD, PACKAGE, PAUSE, PURGE, PUT, REMOVE, REPLACE, RESET, RESUME, SAVE, SETIN, SETOUT, SORT, STOP, STORE, SUBMIT/ENDSUBMIT, SUMMARY, WINDOW ; affectation indicée `x[i, j] = …` ; READ NEXT, clause WHERE de READ, `VAR _NUM_` ; instructions globales dans le corps | affectation mal formée (« expected '=' in an assignment … ») ou échec à l'exécution seulement (STORE, LOAD, SHOW, FREE, REMOVE, EDIT, RESET, READ NEXT, WHERE) | **ERROR** au parsing, message du catalogue (« The START statement is not supported in PROC IML; it can affect results and cannot be ignored (planned: roadmap-avancee J10-P2). ») ; MATTRIB, SHOW, TITLE, FOOTNOTE → **WARNING** d'affichage ; OPTIONS, LIBNAME, FILENAME, ODS → **ERROR** (le corps IML, capturé brut, ne passe pas par l'exécuteur global) | J10-P2 (START, FINISH, RUN, RETURN), J10-P3 (affectation indicée, READ NEXT, WHERE, `VAR _NUM_`), sinon non planifiée |
| J02-P6 | SGPLOT : options de tracé non rendues — `GROUP=`, `RESPONSE=`, `STAT=` autre que FREQ (SUM, MEAN, MEDIAN, PERCENT, valeur inconnue), `SCALE=` autre que COUNT, `FILL`/`NOFILL`, `LEGENDLABEL=`, `MARKERATTRS=`, `LINEATTRS=`, `TYPE=` de DENSITY inconnu ou paramétré (`TYPE=KERNEL(C=…)`), toute autre option après `/` | lues dans l'AST ou sautées, sans diagnostic ; `STAT=` inconnu → FREQ en silence | **WARNING** d'affichage par option (code 1), l'étape s'exécute ; honorées sans diagnostic : `STAT=FREQ`, `SCALE=COUNT`, `BINWIDTH=`, `SMOOTH=`, `DEGREE=`, `TYPE=KERNEL\|NORMAL`, `CATEGORY=` ; avant le `/`, un jeton autre que `X=`/`Y=` (sauté) → **ERROR** de syntaxe | J14-P1 (`GROUP=`, `LEGENDLABEL=`, `RESPONSE=`/`STAT=`, `SCALE=`), sinon non planifiée |
| J02-P6 | SGPLOT : `XAXIS`/`YAXIS` — `VALUES=` réduit à ses deux premiers nombres, `TYPE=`, autres options d'axe (`GRID`, `MIN=`…) | silencieux : `VALUES=(10 20 30 40)` → axe 10..20, graduations perdues ; `VALUES=(-10 to 10)` → 10..10 (signe perdu) ; `TYPE=` lu, jamais rendu | **WARNING** « The VALUES= option of the XAXIS statement is only partly honored in PROC SGPLOT: its first two numbers (10 and 20) set the axis range; the other values and the tick marks are ignored. » ; `TYPE=` et autres options → **WARNING** ; signe conservé ; `LABEL=` honoré | J14-P2 |
| J02-P6 | SGPLOT : tracés que le moteur n'assemble pas — HBAR, VBOX, REG superposés, VBAR ou HISTOGRAM secondaire | abandonnés en silence par le moteur (build graphics) ; build par défaut : NOTE « renders only the first plot statement », fausse pour les superpositions LOESS/DENSITY/SERIES/SCATTER que le moteur dessine | **WARNING** de la couche execute, identique dans les deux builds : « The REG statement is ignored in PROC SGPLOT: it is not drawn over the SCATTER plot (planned: roadmap-avancee J13-P3). » ; HBAR/VBOX/REG en tracé principal : NOTE « REG plot deferred (not yet rendered in PROC SGPLOT). » du moteur, désormais aussi dans le build par défaut (qui annonçait une image différée) | J13-P3 (HBAR, VBOX, REG), sinon non planifiée |
| J02-P6 | SGPLOT : `BY` | NOTE « BY-group processing deferred », aucune image, code 0 | **ERROR** au parsing « The BY statement is not supported in PROC SGPLOT; it can affect results and cannot be ignored (planned: roadmap-avancee J13-P4). » ; même texte pour GPLOT, GCHART et PLOT (le message partagé ne nommait pas l'unité) | J13-P4 |
| J02-P6 | SGPLOT : options du statement PROC (`NOAUTOLEGEND`, `DESCRIPTION=`, `DATTRMAP=`, `SGANNO=`…, `TMPLOUT=`, option inconnue) ; instructions valides non implémentées | options sautées jeton par jeton ; instructions : « 180-322 … not valid » | options d'affichage → **WARNING** ; `TMPLOUT=` (fichier de sortie) → **ERROR** « not supported » ; inconnue → **ERROR** « Unexpected option » ; tracés (BAND, BLOCK, BUBBLE, DOT, ELLIPSE, FRINGE, HBOX, HEATMAP, HIGHLOW, HLINE, LINEPARM, NEEDLE, PBSPLINE, POLYGON, SPLINE, STEP, TEXT, VECTOR, VLINE, WATERFALL, …PARM, …BASIC, XAXISTABLE, YAXISTABLE) → **ERROR** « not supported … cannot be ignored » ; décorations (DROPLINE, GRADLEGEND, INSET, KEYLEGEND, LEGENDITEM, REFLINE, STYLEATTRS, SYMBOLCHAR, SYMBOLIMAGE, X2AXIS, Y2AXIS) → **WARNING** d'affichage | J13-P3 (HBOX), J14-P1 (KEYLEGEND), J14-P2 (REFLINE), sinon non planifiée |
| J02-P6 | GPLOT : options après le `/` de PLOT (`OVERLAY`, `HAXIS=`, `VAXIS=`, `LEGEND=`, `HREF=`, `VREF=`…) ; deuxième requête de tracé (`plot y1*x y2*x;`) | options sautées jusqu'au `;` ; requête : « 180-322: Statement 'Y2' is not valid » | **WARNING** par option (`HAXIS=`/`VAXIS=` : « the first AXIS statement of the step is applied to the horizontal axis and the second to the vertical axis ») ; requête multiple → **ERROR** « not supported … cannot be ignored (planned: roadmap-avancee J14-P3) » | J14-P3 |
| J02-P6 | GPLOT : sous-options SYMBOL (`HEIGHT=`, `WIDTH=`, `LINE=`, `REPEAT=`, `FONT=`…, `INTERPOL=` autre que JOIN/NONE, `VALUE=`, `COLOR=` hors BLACK/BLUE/GREEN/ORANGE/RED) et AXIS (`ORDER=` réduit à ses bornes, attributs de `LABEL=`, `LABEL=NONE`, `MAJOR=`, `MINOR=`, `VALUE=`…) | abandonnées sans diagnostic (symbole VALUE= : marqueur par défaut ; couleur : palette ; `LABEL=NONE` : texte « none » dessiné) | **WARNING** par sous-option ; `INTERPOL=JOIN\|NONE`, les cinq couleurs, le premier texte de `LABEL=` et les bornes de `ORDER=` restent rendus sans diagnostic | J14-P3 |
| J02-P6 | GPLOT : niveaux numériques de `y*x=z` | rangés par leur texte (10 avant 2), SYMBOLn et couleurs assignés dans cet ordre | **WARNING** de la couche execute quand l'ordre texte diffère de l'ordre des valeurs (« … are ordered as text, not by value, in PROC GPLOT (10 is drawn before 2) … (planned: roadmap-avancee J13-P2) ») ; le tri par valeur demandé exige de modifier le rendu, sous `cfg(feature = "graphics")` (constat ci-dessous) | J13-P2 |
| J02-P6 | GPLOT : BUBBLE, BUBBLE2 ; LEGENDn, PATTERNn, GOPTIONS, NOTE ; options PROC (`UNIFORM`, `ANNOTATE=`, `GOUT=`, `IMAGEMAP=`, inconnue) | « 180-322 … not valid » ; options sautées jeton par jeton | BUBBLE/BUBBLE2 → **ERROR** « not supported … cannot be ignored » ; décorations, `UNIFORM`, `ANNOTATE=` → **WARNING** ; `GOUT=` (catalogue), `IMAGEMAP=` (table) → **ERROR** ; inconnue → **ERROR** « Unexpected option » | J14-P3 (LEGEND, PATTERN, GOPTIONS), sinon non planifiée |
| J02-P6 | GCHART : `TYPE=PERCENT\|CFREQ\|CPERCENT`, `SUBGROUP=`, `GROUP=`, `MIDPOINTS=` et autres options de VBAR/HBAR/PIE | `TYPE=` → FREQ en silence ; options avalées | **WARNING** par option (diagramme FREQ dessiné) ; `SUMVAR=`, `TYPE=FREQ\|SUM\|MEAN` honorés sans diagnostic | J13-P5 (`TYPE=`), J14-P3 (`SUBGROUP=`/`GROUP=`/`MIDPOINTS=`) |
| J02-P6 | GCHART : HBAR ; VBAR3D, HBAR3D, PIE3D | barres verticales / dessin à plat, sans diagnostic | **WARNING** « The HBAR statement is drawn as a vertical bar chart in PROC GCHART; horizontal bars are not supported (planned: roadmap-avancee J13-P5). » ; variantes 3D : **WARNING** « drawn as a two-dimensional … chart » | J13-P5 (HBAR), sinon non planifiée |
| J02-P6 | GCHART : DONUT, STAR, BLOCK ; AXISn, LEGENDn, PATTERNn, GOPTIONS, NOTE ; options PROC (`ANNOTATE=`, `GOUT=`, `IMAGEMAP=`, inconnue) | « 180-322 … not valid » ; options sautées jeton par jeton | DONUT/STAR/BLOCK → **ERROR** « not supported … cannot be ignored » ; décorations, `ANNOTATE=` → **WARNING** ; `GOUT=`, `IMAGEMAP=` → **ERROR** ; inconnue → **ERROR** « Unexpected option » | J14-P3 (AXIS, LEGEND, PATTERN, GOPTIONS), sinon non planifiée |
| J02-P6 | PLOT : options nues après `/` (`BOX`, `OVERLAY`, `HZERO`…) ; symbole de `y*x='*'` ; options PROC (`NOLEGEND`, `HPERCENT=`, `FORMCHAR`…, inconnue) | sautées sans diagnostic (seules les options `NAME=` avaient un WARNING) ; symbole ignoré (listing : lettres A, B… des effectifs) | **WARNING** par option et pour le symbole ; options PROC d'affichage → **WARNING**, inconnue → **ERROR** « Unexpected option » | J13-P5 (`BOX`, symbole), sinon non planifiée |
| J02-P6 | ODS GRAPHICS : `RESET`, `RESET=ALL`, `RESET=WIDTH\|HEIGHT\|OUTPUTFMT` (alias `IMAGEFMT`) | parsés puis ignorés | honorés : valeurs par défaut de `WIDTH=`, `HEIGHT=`, `OUTPUTFMT=`, dans l'ordre du source (doc SAS 9.4 ODS User's Guide, ODS GRAPHICS statement, option RESET) ; option non modélisée (`ANTIALIAS`…) : toujours à son défaut, sans effet ; `RESET=IMAGENAME`, `RESET=INDEX` et la part préfixe/index de `RESET` → **WARNING** (constat ci-dessous) ; nom inconnu → **ERROR** | J14-P4 (préfixe et index, après ajout de `src/ast/global.rs` à son périmètre) |
| J02-P6 | ODS `<destination>` : `STYLE=`, `OPTIONS=` | ignorés sans diagnostic (`STYLE=` stocké, appliqué par aucune destination) | **WARNING** d'affichage, la destination s'ouvre | J14-P4 (`STYLE=`), sinon non planifiée |
| J02-P6 | SGPLOT, GPLOT, GCHART : `DATA=` dans le build par défaut | jamais ouvert : table ou variable absente → « image deferred » (ou NOTE de non-activation), code 0 | table et variables validées dans les deux builds, avant l'état ODS GRAPHICS : **ERROR** (« Variable NOPE not found. », une ligne par variable absente) | — |

GLIMMIX partage encore l'ancienne forme `common::parse_response_options`
(`DESC` y est honoré ; `EVENT=FIRST|LAST`, `ORDER=` et `REF=` y restent
ignorés). Son unité de contrat (J02-P3) n'a pas migré l'appel vers la forme
vérifiée : la forme historique de `src/procs/common/model.rs`, hors du
périmètre de fichiers de J02-P3, perdrait son dernier appelant et doit être
retirée dans la même modification (constat transmis au plan).

Les ERROR FACTOR « TYPE=CORR/COV » et « variance nulle » nomment J09-P5,
l'unité désignée par le manifeste pour PRINCOMP/FACTOR ; le périmètre de
fichiers de J09-P5 ne contient toutefois que `src/procs/princomp` : la levée
côté FACTOR doit être routée vers une unité FACTOR (constat transmis au plan
par J02-P4).

PROC IML (J02-P5) garde des écarts visibles ou hors de son périmètre de
fichiers (constats transmis au plan) : l'étape s'arrête à la première erreur
d'exécution là où SAS reprend à l'instruction suivante (ERROR et NOTE « stopped
processing » l'annoncent) ; les comparaisons avec une valeur manquante suivent
l'arithmétique IEEE alors que SAS/IML traite un manquant comme un très grand
négatif (doc « Missing Values ») ; le découpage en segments
(`src/macros/segmenter.rs`) coupe le corps IML à un `run;` intérieur ; les
instructions globales du corps IML ne peuvent être confiées à l'exécuteur
global sans une API de `StatementStream` (`src/parser`).

Les procédures graphiques (J02-P6) émettent leurs diagnostics dans les
couches communes aux deux builds (parsing, exécution,
`src/ods_graphics/contract.rs`) : le code de rendu, sous
`cfg(feature = "graphics")` et réservé à J13–J14, n'a pas été modifié. Deux
corrections demandées n'étaient donc pas réalisables dans ce périmètre
(constats transmis au plan) : le tri par valeur des niveaux numériques de
`=z` dans GPLOT vit dans `graphics_impl::build_series` (WARNING provisoire,
J13-P2) ; `ODS GRAPHICS RESET` ne peut remettre à zéro ni le préfixe
`IMAGENAME=` ni l'index d'image, faute de champ dans `OdsGraphicsStmt`
(`src/ast/global.rs`, hors des périmètres de J02-P6 et de J14-P4). Restent
par ailleurs, hors de la liste de J02-P6 et visibles seulement dans le build
graphics : `BINWIDTH=` d'HISTOGRAM approché (J13-P2), l'échelle par défaut
d'HISTOGRAM (COUNT ; SAS : PERCENT), les catégories numériques de VBAR et de
GCHART rangées comme du texte, la variable numérique de GCHART non regroupée
en points milieux et ses valeurs manquantes comptées comme une catégorie
(défauts SAS : `MIDPOINTS=` calculés, manquants exclus sans `MISSING`),
SYMBOLn/AXISn appliqués selon leur ordre d'apparition (et non leur numéro ni
`HAXIS=`/`VAXIS=`), la taille d'image par défaut 800×600 (SAS : 640×480,
J14-P4).

## Approximations documentées

Comportements livrés volontairement divergents de SAS 9.4 (état
« approximation documentée » de `CONTRIBUTING.md`, §6) : la divergence est
signalée à chaque exécution et décrite ici jusqu'à l'unité qui l'alignera.

| PROC | Élément | sasrs | SAS 9.4 (doc PROC FASTCLUS statement) | Diagnostic | Levée par |
| --- | --- | --- | --- | --- | --- |
| FASTCLUS | Graines initiales | farthest-first : première observation, puis l'observation la plus éloignée des graines déjà choisies | première observation complète, puis règles `RADIUS=` / `REPLACE=FULL` (remplacement des graines) | NOTE « PROC FASTCLUS approximates the SAS algorithm: … » à chaque exécution | J09-P7 |
| FASTCLUS | `MAXITER=` | 10 par défaut ; `MAXITER=0` effectue une itération | 1 par défaut sans `LEAST=` ; 0 : pas de recalcul des graines | même NOTE | J09-P7 |
| FASTCLUS | `MAXCLUSTERS=` | requis (ERROR s'il manque) ; `RADIUS=` non supporté | 100 par défaut ; `MAXCLUSTERS=` ou `RADIUS=` suffit | même NOTE | J09-P7 |
| FASTCLUS | Convergence | déplacement maximal des centroïdes < `CONVERGE=` × écart-type RMS global | changement relatif maximal des graines (distance ancienne/nouvelle graine divisée par la distance minimale entre graines initiales) ≤ `CONVERGE=` | même NOTE | J09-P7 |
| FASTCLUS | `OUT=` | entrée + `_CLUSTER_` | entrée + `CLUSTER` et `DISTANCE` | — | J09-P7 |

## Précisions J03 (statistiques pondérées, encodage, étape DATA)

Le jalon J03 n'ajoute pas de nouvelle règle de sévérité ; il précise le
comportement réel des zones touchées, documenté honnêtement ci-dessous :

- MEANS/SUMMARY sous `WEIGHT` : partition stricte SAS — un poids manquant ou
  ≤ 0 exclut l'observation de N **et** de NMiss (une valeur x manquante avec un
  poids valide compte dans NMiss). `VARDEF=` est honoré (DF → Σw−1, WEIGHT →
  Σw−Σw²/Σw). Toutes les statistiques, y compris `MEDIAN` et les percentiles
  (définition 5 pondérée, position par poids cumulés) et `OUTPUT OUT=
  stat(var)=name`, sont pondérées ; une statistique non calculable dans
  `OUTPUT` reste une ERROR (voir table J02).
- UNIVARIATE sous `WEIGHT` : par défaut un poids nul ou négatif compte 0 dans
  Σw et les moments mais l'observation **reste dans N** (sémantique SAS) ;
  `EXCLNPWGT` l'exclut de l'analyse, N compris, tout comme un poids manquant.
  `NORMAL`/`NORMALTEST` sous `WEIGHT` est indisponible (doc SAS) → NOTE dans le
  log, aucune section, aucun repli silencieux. `OUTPUT OUT=` restitue les
  mêmes statistiques pondérées que le listing.
- Encodage : le contrat D-001 ([docs/encoding.md](encoding.md)) prime — longueurs
  et troncatures en **caractères** (`LENGTH('é')` = 1), entrées UTF-8 strictes
  avec BOM ignoré, jamais de repli lossy silencieux.
- Étape DATA : `FIND`/`FINDC` commencent à la position de départ **incluse**
  (1-based, positions en caractères) ; seuls les modificateurs `i` sont
  honorés. `UPDATE` honore `UPDATEMODE=`/`UPDATE=` (`MISSINGCHECK` par défaut,
  `NOMISSINGCHECK`) aux deux formes documentées par SAS ; `NOMISSINGCHECK` laisse une valeur manquante de la transaction écraser celle du
maître (le défaut `MISSINGCHECK` l'empêche).

## Précisions J07 (levée des diagnostics provisoires J02-P4)

Les diagnostics provisoires posés en J02-P4 sur COMPARE, TRANSPOSE, PRINTTO et
les informats sont levés : les comportements correspondants sont réellement
implémentés et validés par le corpus (cas `compat/*`, cf.
[`conformance/STATUS.md`](../conformance/STATUS.md)) :

- **COMPARE** : `CRITERION=` (+ `METHOD=ABSOLUTE\|RELATIVE`), `OUTBASE=` /
  `OUTCOMP=` / `OUTDIF=` / `OUTNOEQUAL=` (`_TYPE_` BASE/COMP/DIF, `_OBS_`),
  `ID`, `VAR`/`WITH` et `BY` sont honorés. `BRIEF`/`BRIEFSUMMARY` (rapport
  condensé), `LISTALL` (la section valeurs liste toutes les variables
  comparées), `OUTPERCENT=` (lignes PERCENT de OUT=) et `MAXPRINT=n|(n,p)`
  le sont également depuis J07-P9 : `MAXPRINT=` plafonne la section
  « Value Comparison Results » à n différences imprimées par observation et
  p observations avec différences imprimées (défauts 50/50 ;
  `MAXPRINT=n` seul laisse p à 50) et émet une NOTE quand la limite tronque
  l'affichage. Seules les options inconnues restent des ERROR.
- **TRANSPOSE** : `IDLABEL` (+ `LABEL=`), `COPY` (une observation de sortie par
  observation d'entrée, complétée par des missings), `SUFFIX=`, `LET`
  (dernière occurrence des `ID` dupliquées) et `ID` multi-variables avec
  `DELIMITER=` sont honorés.
- **PRINTTO** : `LOG=` / `PRINT=` routent physiquement le journal / le listing
  vers le fichier externe, `NEW` remplace le contenu existant, et un
  `PROC PRINTTO;` nu retablit les destinations par défaut. L'ancien WARNING
  (« routing not supported », code de sortie 1) n'est plus émis.
- **Informats (étape DATA / PROC CONTENTS)** : les informats — explicites dans
  `INPUT` ou déclarées par l'instruction `INFORMAT` / `ATTRIB INFORMAT=` — sont
  persistés dans les métadonnées du dataset et restitués par
  `PROC CONTENTS OUT=` (`INFORMAT`, `INFORML`, `INFORMD`). L'ancienne
  limitation « informats are not persisted in dataset metadata » n'existe plus.
  Une variable encore inconnue référencée par l'instruction `INFORMAT` entre
  au PDV à sa position textuelle (J01-P3, doc SAS 9.4 INFORMAT statement) —
  caractère si l'informat commence par `$` (longueur déclarée = largeur de
  l'informat), numérique (8) sinon — comme le font déjà `LENGTH`/`FORMAT`/
  `ATTRIB`.
- **CONTENTS** : `NOPRINT` est honoré (J01-P3) — supprime tout le listing
  (en-tête et table des variables) ; `OUT=` reste toujours écrit. L'ancien
  ERROR « Unexpected option 'NOPRINT' on PROC CONTENTS statement » n'est plus
  émis. La colonne `NAME` de `OUT=` restitue le nom de variable avec la casse
  déclarée (J01-P7 ; exemple du CONTENTS statement, SAS 9.4 Procedures Guide :
  `length aa 7 bb 6 ...` → `NAME` = `aa`, `bb`, …) ; les observations de
  `OUT=` suivent l'ordre `VARNUM` (position des variables).
- **MEANS/SUMMARY** : `CLASS / MISSING`, `NWAY`, `ORDER=FREQ`, `FREQ` (pondère
  N et STD), `ID` copiée dans `OUT=` et `OUTPUT OUT=` avec `AUTONAME` /
  `MAXDEC=` sont honorés et validés. Ordre des colonnes de `OUT=` (J01-P2/
  J01-P3) : `BY` → `CLASS` → `_TYPE_` → `_FREQ_` → `ID` → statistiques.

## Catalogue et exemples

- `unsupported_statement(proc, stmt)` : ERROR « The BY statement is not supported
  in PROC GLM; it can affect results and cannot be ignored. » Même règle pour
  WEIGHT dans LOGISTIC et BY/WEIGHT/FREQ/OUTPUT/ID/WHERE/CLASS dans les procédures
  qui ne les exécutent pas. REWEIGHT/REFIT dans REG, ESTIMATE/CONTRAST/LSMEANS dans
  MIXED et GLIMMIX, ID dans FASTCLUS, DELETE/COPY dans CATALOG et GUESSINGROWS dans
  IMPORT sont également rejetés : enregistrer une demande sans la réaliser
  n'est pas un support effectif.
- `ignored_display_statement(proc, stmt)` : WARNING « The FORMAT statement is
  ignored in PROC GLM; display customization is not supported. » Les demandes
  locales FORMAT, LABEL et ATTRIB limitées à l'affichage suivent cette règle
  quand elles ne sont pas implémentées. ATTRIB LENGTH ou INFORMAT est une ERROR,
  car ces attributs peuvent changer les valeurs stockées. PAINT dans REG est
  également une personnalisation d'affichage ignorée avec WARNING.
- Une instruction inventée, par exemple `invented x;`, produit une ERROR
  « 180-322: Statement 'INVENTED' is not valid or it is used out of proper order
  in PROC GLM. » Le nom de la procédure, de l'instruction et le span du token
  fautif sont conservés. Aucun saut silencieux vers le prochain point-virgule.

```sas
proc glm data=work.t;
  model y=x;
  by group;           /* ERROR : l'analyse par groupe n'est pas implémentée. */
run;
proc logistic data=work.t;
  model y=x;
  weight w;           /* ERROR : les poids ne doivent pas être perdus. */
run;
proc print data=work.t;
  format x 8.2;       /* WARNING : la PROC s'exécute avec l'affichage courant. */
run;
```

Les anciens tests qui exigeaient le saut silencieux d'une instruction inconnue
sont remplacés par des assertions ERROR. La fixture `m36/mtest.sas` conserve son
intention de tester MTEST et ADD : sa demande REWEIGHT non exécutée est retirée.
Le rejet de REWEIGHT est couvert séparément par les tests `contract_*`.

## Instructions globales dans une PROC

TITLE, FOOTNOTE (y compris les niveaux déjà supportés), OPTIONS, LIBNAME, ODS et
FILENAME passent par le parseur global et l'exécuteur global existants. Elles
prennent effet, dans leur ordre source, avant l'exécution de la procédure. Elles
ne sont ni ignorées ni considérées comme des instructions inconnues. Les effets
déjà parsés et les avertissements sont drainés une seule fois, même si une
instruction ultérieure invalide la PROC ; ils ne fuient pas à l'étape suivante.
Une erreur d'exécution d'une instruction globale empêche l'exécution de la PROC
avec un état incomplet. Les commentaires `* ... ;` et les points-virgules vides
restent inertes ; DATA/PROC ouvre une nouvelle étape implicite.

```sas
proc print data=work.t;
  title 'Résultats';
  footnote 'Source interne';
  options ls=100;
  ods select all;
  var x;
run;
```

Le transport ne modifie pas le support des différentes options globales. Les
options d'en-tête de PROC, sous-options d'une instruction implémentée et le
langage SQL font l'objet d'unités distinctes ; cette unité couvre les
instructions de corps de PROC et leurs anciens chemins d'ignorance.

## Références indépendantes et vérification

- Le [guide SAS des étapes PROC](https://support.sas.com/documentation/cdl/en/grstatproc/65235/HTML/default/p15w7pav2htadsn1pvwlbm2t7ca7.htm)
  précise que les instructions globales sont autorisées dans une étape PROC.
- La [documentation SAS sur les instructions globales](https://blogs.sas.com/content/sgf/2021/03/15/how-to-conditionally-execute-sas-global-statements/)
  décrit leur effet et leur persistance pendant la session.
- La [référence SAS du diagnostic 180-322](https://blogs.sas.com/content/sasdummy/2016/08/25/error-180-322-missing-semicolon/)
  donne la catégorie et le texte d'erreur des instructions non valides.

Ces références et la règle de sévérité préexistante de CONTRIBUTING constituent
l'oracle indépendant. Les snapshots `j02/contract_*.sas` verrouillent les logs,
listings et codes de sortie ; ils ne sont pas, à eux seuls, une preuve de
conformité numérique à SAS.
