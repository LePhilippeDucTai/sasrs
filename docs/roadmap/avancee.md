# Feuille de route avancée — sasrs après consolidation

Statut : document de planification (J08-P5, issue #10). Ce document ne promet
rien que le code ne fait pas : chaque item porte l'**état actuel** constaté dans
le code à la révision `3a776b83e19d100122723fa6eff1763055d2cbcb` —
*implémenté* (le code le fait, sans oracle externe dédié), *validé* (cas de
conformité `conformance/STATUS.md` avec référence externe), *approximation*
(comportement documenté qui diverge volontairement de SAS 9.4) ou *ERROR*
(diagnostic explicite, jamais de repli silencieux — contrat
`docs/support-contract.md`).

Chaque item est **borné par comportement** : un oracle indépendant prévu et un
critère d'acceptation exécutable — jamais « terminer PROC X ». Pour les procs
statistiques, chaque item précise le traitement prévu de : non-convergence,
matrice singulière, paramètre sur frontière, identifiabilité, tolérances.

## LOGISTIC

État actuel (README, `src/procs/logistic/`) : binaire + logit/cloglog/probit et
proportional-odds ordinal implémentés (Newton-Raphson), `CLASS` PARAM=REF
(ref=FIRST|LAST), `MODEL y(DESCENDING EVENT=)/LINK=`, `FREQ`,
`OUTPUT OUT= PREDICTED=/P=/XBETA=`, fit/estimates/odds-ratios ; un cas de
conformité **validé** (`stat/logistic-binary-output`). J02-P5/J02-P6 : options
inconnues → ERROR, convergence véridique (WARNING SAS si non-convergence, un
pas singulier ordinal ne `break` plus en silence).

| Item (borné par comportement) | État actuel | Dépendances | Oracle indépendant prévu |
|---|---|---|---|
| `PARAM=EFFECT|GLM` codage de CLASS (estimates + odds ratios identiques à SAS sur un plan 2×2 déséquilibré) | ERROR (PARAM=REF only) | aucun | exemples publiés SAS/STAT « CLASS Variable Parameterization », valeurs recopiées |
| multinomial généralisé (generalized logit, `LINK=GLOGIT` : 3 niveaux nominaux, estimates par logit vs catégorie de référence) | ERROR | aucun | doc SAS/STAT « Example: Logistic Regression with a Nominal Response » |
| Score test for proportional odds (p-value du test de score sur un modèle ordinal à pentes inégales simulées) | non émis (différé) | proportional-odds existant | statsmodels `OrderedModel` ou table publiée SAS |
| `ESTIMATE`/`CONTRAST`/`LSMEANS` (une contrast 1-df : estimate, SE, χ², p) | ERROR | `lincom` (`src/procs/lincom`, ex-M37) | valeurs recopiées de la doc SAS/STAT ESTIMATE |
| `BY` (invariant : un seul groupe BY = sortie sans BY ; en-têtes de groupe SAS) | ERROR | `common::by` | invariant mono-groupe + doc SAS BY |
| `UNITS`/`ROC`/`SCORE` (aire ROC du modèle binaire + stat de Hosmer-Lemeshow optionnelle) | ERROR | aucun | package `pROC`/`ResourceSelection` ou table SAS publiée |
| Diagnostics numériques : non-convergence (WARNING SAS « Convergence was not attained… »), séparation quasi complète (« The maximum likelihood estimate may not exist… »), plan singulier → ERROR explicite, pas singulier sans break silencieux | implémenté (J02-P6, tests `convergence_*`) | aucun | déjà testé unitairement ; ajouter un cas corpus à séparateur montrerait le WARNING |
| Tolérances : β à 1e-8 relatif vs oracle ; p-values 1e-10 ; Hessian régularisé à 1e-12 avant inversion, flag « not positive definite » | approximation implicite (aucune politique écrite) | aucun | oracle R `glm` sur 3 datasets canoniques |

## GENMOD

État actuel : DIST=POISSON/BINOMIAL/NORMAL/GAMMA, LINK associés, SCALE=/NOSCALE,
CLASS ref, FREQ, fit/estimates avec CI Wald ; NR/IRLS avec step-halving
(GCONV=1e-8) ; J02-P5 : DIST/LINK inconnus et MODEL inconnus → ERROR ; J02-P6 :
ligne « Convergence criterion … satisfied » véridique, épuisement du
step-halving = non-convergence. Rien de **validé** au corpus.

| Item | État actuel | Dépendances | Oracle indépendant prévu |
|---|---|---|---|
| échelle Gamma exacte ML (digamma) vs approximation Pearson actuelle : β et scale à 1e-6 relatif sur 2 datasets gamma publiés | approximation (Pearson-dispersion documentée) | aucun | R `glm(family=Gamma)` recopiée |
| `OFFSET=` (modèle Poisson avec offset : identique à SAS à 1e-8) | ERROR (J02-P5) | aucun | doc SAS/STAT GENMOD « offset » example |
| `ESTIMATE`/`CONTRAST` (1 contrast linéaire : χ² Wald) | ERROR | `lincom` | valeurs SAS publiées |
| `OUTPUT OUT=` (résidus deviance/pearson, prédiction μ̂, CL) | ERROR | aucun | R `residuals(type="deviance")` |
| GEE `REPEATED` (échangeable, GEE de niveau 1 : β + SE sandwich sur données longitudinales publiées) | ERROR | aucun | `geepack` R ou table SAS |
| Non-convergence/singularité : WARNING SAS à l'épuisement du step-halving ; colinéarité parfaite → ERROR ; identifiabilité (colonne CLASS redondante) signalée « DF 0 » sans crash ; tolérance GCONV configurable | implémenté pour la convergence ; identifiabilité partielle (DF 0) ; GCONV non configurable | aucun | tests unitaires + oracle R |

## GLM / ANOVA

État actuel : GLM ✅ (TYPE I/III, LSMEANS/SE, ESTIMATE/CONTRAST main effects,
SOLUTION) ; ANOVA ✅ (TYPE I/III) ; plan de rang incomplet → ERROR explicite
(J02-P6). Cas corpus : `glm-oneway-predicted` en **divergence connue**
(`OUTPUT` non supporté).

| Item | État actuel | Dépendances | Oracle indépendant prévu |
|---|---|---|---|
| `OUTPUT OUT=` (prédictions + résidus studentisés ; lève la divergence corpus) | ERROR | aucun | cas `stat/glm-oneway-predicted` (attendus déjà dans le corpus) |
| covariables continues dans le MODEL (TYPE III identique à SAS sur plan déséquilibré + covariable) | non couvert | aucun | doc SAS/STAT GLM Example A |
| `LSMEANS / ADJUST=TUKEY` (lettres de regroupement, p ajustées) | ERROR | aucun | tables publiées Tukey |
| Type II SS (égale TYPE III sans interaction ; diffère avec interaction sur plan déséquilibré — 3 nombres exacts) | non émis | aucun | doc SAS « The Four Types of Estimable Functions » |
| `MEANS` comparison tests ANOVA (Tukey/Duncan/Scheffé) | ERROR | aucun | table SAS publiée |
| BY (invariant mono-groupe) | ERROR | `common::by` | invariant + doc SAS |
| Singularité : rang incomplet → ERROR explicite (déjà posé J02-P6) ; généralisée inverse + estimabilité (règle « Non-estimable » de SAS pour LSMEANS sous rang incomplet) | ERROR global (pas d'inverse généralisée) | aucun | exemple SAS « General Inverse » publié |
| Tolérances : SS à 1e-10 relatif ; F/p 1e-10 ; codage sum-to-zero exact | implémenté (TYPE III validé informellement par tests unitaires) | aucun | cas corpus GLM additionnels |

## MIXED

État actuel : REML/ML, RANDOM INTERCEPT/SUBJECT TYPE=VC|CS, REPEATED VC|CS|AR(1)|UN,
optimisation (RE)ML générale (Nelder-Mead + polish), β̂/SE avec ddl Contain ;
J02-P5 : TYPE/METHOD inconnus → ERROR, DDFM≠CONTAIN → ERROR ; J02-P6 :
non-convergence véridique, « Estimated G matrix is not positive definite. » si
composante tronquée à 0.

| Item | État actuel | Dépendances | Oracle indépendant prévu |
|---|---|---|---|
| RANDOM slopes (modèle aléatoire pente+ordonnée TYPE=UN sur données publiées : covariance estimates à 1e-6) | ERROR | structure existante | dataset `sleepstudy` + `lme4` R, valeurs publiées |
| `LSMEANS`/`ESTIMATE`/`CONTRAST` (1 estimate avec ddl Contain, t, p) | ERROR | `lincom`, Contain existant | examples SAS MIXED publiés |
| Satterthwaite / Kenward-Roger ddl (ddl KR sur le modèle random-intercept du manuel SAS : mêmes ddl à ±0.1) | ERROR (DDFM=CONTAIN only) | — | table SAS MIXED « Example 41.4 » ou package `pbkrtest` |
| `COVTEST` (Wald/Z sur composantes de variance) | ERROR | aucun | — |
| Multiple random statements / TYPE=ARH(1), TOEP, SP(POW) | ERROR | aucun | — |
| BY (invariant mono-groupe) | ERROR | `common::by` | invariant |
| Non-convergence (WARNING + pas de « Convergence criteria met » si échec), G non définie positive (NOTE SAS), paramètre de variance sur frontière 0 (NOTE, pas d'échec), tolérances GCONV=1e-8 configurables | implémenté (J02-P6) sauf GCONV configurable | aucun | tests unitaires existants + oracle `lme4` |
| Identifiabilité : plan fixe singulier dans un MIXED → ERROR explicite (comme GLM) | implémenté via le chemin commun | aucun | — |

## GLIMMIX

État actuel : RSPL/PQL (Breslow-Clayton), LAPLACE (random intercept, ML vrai),
DIST=NORMAL|POISSON|BINARY, LINK associés, REPEATED R-side, FREQ, SOLUTION,
Type III ; cross-validations documentées (≡ MIXED ML, ≡ GENMOD/LOGISTIC sans
random) ; J02-P5 : QUAD → ERROR, DIST/LINK inconnus → ERROR ; J02-P6 : drapeau
Nelder-Mead respecté.

| Item | État actuel | Dépendances | Oracle indépendant prévu |
|---|---|---|---|
| `METHOD=QUAD` (quadrature adaptative, 1 random intercept : β et SE à 1e-6 vs SAS/GLMMadapt sur données publiées binomiales à grappes) | ERROR | aucun | dataset `cbpp` + `lme4::glmer` nAGQ, valeurs publiées |
| RANDOM slopes (PQL : β du modèle pente aléatoire à 1e-6 vs `glmmPQL`) | ERROR | MIXED slopes | `MASS::glmmPQL` |
| `DIST=GAMMA`+LOG | ERROR | GENMOD gamma exact | R `glmmTMB` |
| `WEIGHT` | ERROR | aucun | — |
| LSMEANS/ESTIMATE/CONTRAST | ERROR | `lincom` | examples SAS GLIMMIX |
| LAPLACE avec AR(1)/UN/random multiples : NOTE actuelle → comportement complet | approximation (NOTE) | — | oracle SAS |
| Non-convergence PQL/Laplace (WARNING SAS, drapeau Nelder-Mead), G non définie positive (NOTE), λ sur frontière (NOTE), tolérances | implémenté (J02-P6) | aucun | tests unitaires + oracle `glmer` |

## Multivarié — PRINCOMP / FACTOR / DISCRIM / CLUSTER / FASTCLUS

État actuel : PRINCOMP 🟡 (scores OUT=, N=, COV) ; FACTOR 🟡 (PRINCIPAL,
VARIMAX/PROMAX, OUT= scores ; QUARTIMAX/OBLIMIN → ERROR) ; DISCRIM 🟡 (LDA
POOL=YES, PRIORS, OUT= posteriors ; POOL=NO → ERROR) ; CLUSTER 🟡 (WARD/
AVERAGE/SINGLE/COMPLETE, OUTTREE=) ; FASTCLUS 🟡 (k-means farthest-first,
OUT=). Aucun cas corpus validé sur ces procs.

| Item | État actuel | Dépendances | Oracle indépendant prévu |
|---|---|---|---|
| PRINCOMP `OUTSTAT=`/`TYPE=CORR` en entrée (eigen round-trip : λ et vecteurs à 1e-10 sur matrice publiée) | ERROR/non couvert | aucun | `prcomp` R / table SAS |
| PRINCOMP `PARTIAL` (résidus de régression avant ACP : mêmes scores que SAS sur exemple doc) | non couvert | aucun | doc SAS PRINCOMP Example |
| FACTOR `METHOD=ML` (extraction ML avec convergence véridique — WARNING si non-convergence ; loadings à 1e-4 vs `factanal` R) | ERROR | aucun | `psych::fa` / `factanal` |
| FACTOR `HEYWOOD` (communalité > 1 bornée + WARNING SAS « A Heywood case has been detected ») | ERROR | METHOD=ML | doc SAS FACTOR |
| DISCRIM `POOL=NO` QDA (fonctions quadratiques + taux d'erreur sur données publiées iris : identique à `qda` R) | ERROR (J02-P5) | aucun | `MASS::qda` sur iris |
| DISCRIM `CROSSVALIDATE` (leave-one-out : matrice de confusion identique à `cv.glm`-style LOO) | ERROR | aucun | `MASS::lda` CV |
| CLUSTER `PSEUDO=` (F/t² pseudo, valeurs du manuel SAS sur exemple US cities) | non couvert | aucun | table SAS CLUSTER Example |
| CLUSTER `CCC` (coefficient cubique de cluster : reproductible sur l'exemple du manuel à 1e-6) | non couvert | PSEUDO | Sarle/SAS published table |
| FASTCLUS `SEED=`/`RADIUS=`/`DISTANCE` (le même partitionnement que SAS sur l'exemple doc, seed fixé) | ERROR/non couvert | aucun | doc SAS FASTCLUS Example |
| Singularité : matrice de corrélation singulière (variables colinéaires) → ERROR nommant les variables, pas de NaN silencieux ; PRINCOMP/FACTOR : variables à variance nulle exclues avec NOTE SAS | approximé (QR gère, pas de diagnostic dédié) | aucun | tests unitaires + prop tests |
| Tolérances : vecteurs propres à 1e-10 (signe déterministe documenté) ; convergence rotation VARIMAX à 1e-8 | implémenté (convention de signe déterministe documentée) | aucun | `eigen` R |

## IML

État actuel (`src/procs/iml/`) : sous-langage dédié — littéraux, indexation,
opérateurs (dont `'`, `#`, `@`), stats élémentaires, IF/DO, PRINT, INV/SOLVE/
EIGVAL/EIGVEC/CHOL/QR/SVDC/DET, CREATE/APPEND/READ/CLOSE, SHAPE, sous-script
range. Manquent : modules, READ NEXT/WHERE, LOAD/STORE/SHOW.

| Item | État actuel | Dépendances | Oracle indépendant prévu |
|---|---|---|---|
| modules `START`/`FINISH` avec arguments (récursion fibonacci(10) = 55 via module ; portée locale) | ERROR/non couvert | aucun | programme de référence IML doc SAS |
| `CALL` supplémentaires numériques : `ROOT` (racine d'un polynôme), `LPSOLVE` simple (solution d'un LP 2 variables publiée) | non couvert | aucun | doc SAS/IML User's Guide |
| `READ … WHERE`/`READ NEXT` (lecture filtrée d'une table : matrice identique à l'étape DATA équivalente) | ERROR/non couvert | aucun | programme DATA équivalent (auto-oracle) |
| Graphiques IML (`CALL GDRAW`…) : hors périmètre proposé — renvoyé à la section Graphiques | non couvert | graphics | — |
| Matrice singulière : `INV` d'une matrice singulière → ERROR typée (jamais de NaN silencieux) ; `SOLVE` incohérent → ERROR | à vérifier/implémenter | aucun | tests unitaires matrice singulière |
| Tolérances : INV/SOLVE vérifiés par X·A − I < 1e-10 en norme Frobenius, ERROR si dépassé | non couvert | aucun | propriété (test auto) |

## S3

État actuel (`src/library/s3.rs`, feature `s3`, OFF par défaut) : `LIBNAME
libref s3://bucket/prefix` → scan cloud polars **lecture seule**.
`read`/`scan` implémentés ; `exists` → `false` (stub, pas de HEAD objet) ;
`list`/`write`/`delete`/`rename` → ERROR explicite « read-only cloud scan
stub » ; sidecar non lu sur S3 (métadonnées formats/labels non restituées).

| Item | État actuel | Dépendances | Oracle indépendant prévu |
|---|---|---|---|
| `exists`/`list` via HEAD/LIST d'objets (PROC DATASETS LISTING d'une lib S3 de test MinIO : noms exacts) | approximation (exists=false ; list=ERROR) | cible de test locale (MinIO) | fixture MinIO + attendus figés |
| `write` Parquet + sidecar (aller-retour : table écrite depuis sasrs puis relue, données ET VarMeta identiques) | ERROR | ADR 0001 (protocole atomique — à adapter multi-objet) | test d'aller-retour auto-oracle |
| `delete`/`rename` (sémantique d'anciens commits : pas d'orphelins parquet/sidecar) | ERROR | write | propriété pas d'orphelin (fault-injection étendue au cloud en mock) |
| Credentials/endpoint explicites (`ENDPOINT=`, `REGION=`, profile) au lieu des seules variables d'environnement | non couvert | aucun | fixture MinIO |
| CI : job `test-s3-lib` étendu à un MinIO éphémère (actuellement tests lib hors réseau) | partiel | MinIO | — |

## Graphiques

État actuel : feature `graphics` (plotters), PNG/SVG via ODS GRAPHICS.
SGPLOT 🟡 (SCATTER/SERIES/VBAR/HISTOGRAM/DENSITY/VBOX/REG/LOESS, axes ;
**parse-only sous graphics pour HBAR/VBOX/REG → NOTE** ; `GROUP=`,
`RESPONSE=`/`STAT=`, `SCALE=` parsés mais non rendus ; pas de légendes ;
pas d'images par groupe `BY`). GPLOT/GCHART/PLOT/SGPLOT : `BY` → ERROR.
UNIVARIATE/REG graphiques câblés. `ODS GRAPHICS RESET=` parsé puis ignoré.

| Item | État actuel | Dépendances | Oracle indépendant prévu |
|---|---|---|---|
| rendre HBAR/VBOX/REG de SGPLOT (mêmes données que VBAR : mêmes comptes de barres/valeurs ajustées) | parse-only (NOTE) | aucun | données de l'exemple SGPLOT doc + valeurs numériques des bins (pas de l'image) |
| `GROUP=` SGPLOT (une série par niveau de GROUP, légende des niveaux) ; `RESPONSE=`/`STAT=` (barres = SUM/MEAN de la réponse, pas FREQ brut) ; `SCALE=` (axe densité) | parsé non rendu | aucun | exemples doc SGPLOT (valeurs d'échelle vérifiées, pas le pixel) |
| légendes (KEYLEGEND/LABEL : liste de niveaux ordonnée) | non couvert | GROUP= | — |
| styles ODS (`STYLE=` complet : jeu de couleurs/polices déterministe par style) | approximation (partial) | aucun | snapshot déterministe par style |
| overlays multi-plot (plusieurs SERIES/SCATTER superposés au-delà de primary+LOESS/DENSITY) | non couvert | aucun | — |
| images par groupe `BY` (une image `nom_1..n` par niveau, ordre des groupes = ordre de tri) | ERROR | `common::by` | fixture BY à 2 groupes → 2 fichiers, noms contractuels |
| PLOT/GCHART/GPLOT `BY` (idem) | ERROR | `common::by` | — |
| GCHART HBAR horizontal (barres horizontales réelles, plus de dessin en vertical sans diagnostic) | approximation silencieuse → à diagnostiquer ou corriger | aucun | fixture + nom de fichier |
| `ODS GRAPHICS RESET=` (retour aux défauts vérifiable : WIDTH/HEIGHT/IMAGENAME retombent) | parsé-ignoré | aucun | test d'état |
| PLOT `HREF=`/`VREF=`/axes formatés dans le rendu ASCII/image (lignes de référence aux valeurs demandées) | WARNING (reconnu non rendu) | aucun | — |

## Résidus Base — TABULATE, REPORT, DATASETS, CATALOG, OPTIONS

### TABULATE

État : CLASS/VAR/TABLE 1-3 dims, stats N NMISS SUM MEAN MIN MAX STD PCTN PCTSUM,
ALL, `*` crossings, OUT=, formats, labels ; `BY` implémenté (J08-P2).
Dénominateurs de groupe `PCTN<...>` : atome parenthésé → ERROR explicite.

| Item | État actuel | Dépendances | Oracle indépendant prévu |
|---|---|---|---|
| `PCTN<dim>`/`PCTSUM<dim>` dénominateurs de groupe (une table 2×2 avec PCTN<row> : les 4 pourcentages exacts de la doc SAS) | ERROR | aucun | exemple TABULATE doc SAS (pourcentages publiés) |
| `CONCAT`/`UNION` d'instructions TABLE multiples | non couvert | aucun | — |
| `BOX=`/`RTS=`/`KEYLABEL` | ERROR (reconnus) | aucun | — |

### REPORT

État : COLUMN/DEFINE (DISPLAY/ORDER/GROUP/ANALYSIS), WHERE, BREAK/RBREAK
SUMMARIZE, COMPUTE + COMPUTE AFTER/LINE. `DEFINE FLOW` et COMPUTE riche
(écriture dans les colonnes calculées avec bibliothèque de fonctions complète)
manquent.

| Item | État actuel | Dépendances | Oracle indépendant prévu |
|---|---|---|---|
| `DEFINE … FLOW` (cellule longue repliée sur N lignes dans la largeur de colonne : lignes produites identiques à SAS) | ERROR | aucun | exemple REPORT doc SAS (texte replié publié) |
| COMPUTE avec rétro-écriture dans une colonne calculée + fonctions complètes (recodage conditionnel d'une colonne : dataset OUT= identique à l'étape DATA équivalente) | approximation (affectations simples) | aucun | étape DATA équivalente (auto-oracle) |
| ordre d'évaluation COMPUTE vs BREAK (variables automatiques `_BREAK_`) | non couvert | COMPUTE riche | doc SAS REPORT |

### DATASETS

État : DELETE/CHANGE/COPY/EXCHANGE/SAVE/MODIFY RENAME/LABEL, run-group.
`APPEND`/`REPAIR`/`CONTENTS` internes → ERROR (APPEND existe en PROC séparée,
validé).

| Item | État actuel | Dépendances | Oracle indépendant prévu |
|---|---|---|---|
| `APPEND` dans DATASETS (déléguer à la PROC APPEND existante : mêmes WARNING FORCE que l'appel direct) | ERROR | proc APPEND (implémentée) | tests APPEND existants |
| `REPAIR` (sidecar orphelin reconstruit depuis le parquet : table relisible avec métadonnées par défaut) | ERROR | ADR 0001/J04 | tests fault-injection (corruption volontaire) |
| `CONTENTS` dans DATASETS (déléguer à la PROC CONTENTS) | ERROR | proc CONTENTS | snapshots CONTENTS existants |

### CATALOG

État : CATALOG=libref.cat, CONTENTS (formats en mémoire), DELETE/COPY no-op +
NOTE. Vrais `.sas7bcat` non lus (liés à ADR 0004 : lecture sas7bdat seulement,
catalogue sas7bcat = extension naturelle du même parseur).

| Item | État actuel | Dépendances | Oracle indépendant prévu |
|---|---|---|---|
| persistance du catalogue formats au format sas7bcat réel (lecture ; écriture hors périmètre comme ADR 0004) | approximation (JSON sidecar `formats.sascat.json`) | ADR 0004, corpus sas7bcat de fixtures | fichier .sas7bcat réel + valeurs FMTLIB attendues |
| sélection par type d'entrée (`ENTRYTYPE=`) | non couvert | aucun | — |

### OPTIONS

État : listing des options système ; options appliquées : LINESIZE, FIRSTOBS,
OBS, NODATE/NONUMBER/NOCENTER, MISSING=, YEARCUTOFF=, MPRINT/MLOGIC/SYMBOLGEN,
FMTSEARCH= ; PAGESIZE stockée seulement ; les autres → WARNING « not yet
supported ».

| Item | État actuel | Dépendances | Oracle indépendant prévu |
|---|---|---|---|
| `PROC OPTIONS OPTION=<name>` (détail par option : valeur + description de la doc SAS) | non couvert | aucun | doc SAS OPTIONS par option |
| pagination PAGESIZE/PS réelle (sauts de page dans le listing) | stockée seulement | aucun | attendus de pagination figés |
| honourer `NONUMBER`/`DATE` combos restants et `CENTER/NOCENTER` dans tous les flux de sortie | partiel | aucun | snapshots |

## Procs dont BY reste en ERROR

Établi par J08-P2 (CORR et TABULATE désormais OK) : BY est supporté par SORT,
MEANS/SUMMARY, PRINT, RANK, CORR, FREQ, UNIVARIATE, TTEST, NPAR1WAY, REG,
TRANSPOSE, COMPARE, TABULATE, SGPLOT. Partout ailleurs l'instruction `BY` tombe
sur le fallback commun (`unhandled_proc_statement`) → ERROR « The BY statement
is not supported in PROC … ». Liste à date :

- Statistiques : `ANOVA`, `GLM`, `LOGISTIC`, `GENMOD`, `MIXED`, `GLIMMIX`,
  `PRINCOMP`, `FACTOR`, `DISCRIM`, `CLUSTER`, `FASTCLUS`, `DISTANCE`.
- Graphiques : `GPLOT`, `GCHART`, `PLOT` (SGPLOT a BY sans images par groupe —
  cf. section Graphiques).
- Base/divers : `IML`, `REPORT`, `CATALOG`, `DATASETS`, `FORMAT`, `IMPORT`,
  `EXPORT`, `CONTENTS`, `APPEND`, `SQL` (ces huit dernières rejettent BY
  correctement : BY n'y est pas non plus une instruction valide en SAS 9.4 —
  pas des candidates à la généralisation).

Item borné commun, à dérouler proc par proc avec le même invariant et le même
oracle — priorité aux procs où l'analyse par groupe est un cas d'usage réel
(GLM/LOGISTIC puis les multivariés) : **invariant « un seul groupe BY = sortie
sans BY » + en-têtes de groupe SAS + `OUT=` avec variables BY ; oracle :
doc SAS par proc + invariant mono-groupe en test automatique** (le motif
`common::by` de MEANS/COMPARE/CORR/TABULATE, aucune modification du noyau
attendue).

## Items ADR 0003 — API Python native (pyo3/maturin)

Reprise intégrale des items de `docs/adr/0003-api-python.md` :

1. **Surface minimale `sasrs::api`** (types, erreurs → exceptions Python
   `SasrsError` hiérarchiques) préalable à la crate `sasrs-py`. État actuel :
   façade `sasrs::api` implémentée et testée (J06-P1/J06-P2) ; état item :
   partiel (typage d'erreurs à exposer).
2. **Squelette `sasrs-py`** : manifeste maturin, paquet `python/src/sasrs_py`,
   pytest de fumée (session + aller-retour RecordBatch). État : non couvert
   (seul le wrapper CLI existe).
3. **Conversion `SasDataset` ↔ Arrow RecordBatch** avec métadonnées sidecar en
   `schema.metadata` + tests de fidélité (formats, libellés). État : non
   couvert.
4. **CI roues matricielle** (manylinux x86_64, macOS arm64, Windows x86_64),
   publication PyPI draft, versions alignées avec les tags CLI. État : non
   couvert (seul le binaire CLI est publié, J06-P5).
5. **Décision repli CLI** dans `sasrs_py` (import conditionnel) + matrice
   supportée dans `python/README.md`. État : non tranché.
6. **Benchmark wrapper CLI vs liaison native** (aller-retour Parquet vs Arrow
   FFI) pour arbitrer la migration par défaut. État : non couvert.

Dépendances entre items : 1 → (2, 3) → 4 → (5, 6). Oracle indépendant : pytest
end-to-end (données connues), `abi3-wheel` testée sur les 3 OS en CI.

## Items ADR 0004 — Adaptateurs sas7bdat / XPT

Reprise intégrale des items de `docs/adr/0004-adaptateurs-sas7bdat-xpt.md`
(aucun code à date — `PROC IMPORT` lit CSV/TAB/DLM/XLSX ; XPT/sas7bdat → ERROR) :

1. **Corpus de fixtures sas7bdat/XPT réels** (empreintes SHA-256, provenances)
   + oracle différentiel pandas/pyreadstat hors CI. État : non couvert.
2. **Évaluation des crates de lecture sas7bdat** sur le corpus ; décision
   maison vs crate avec tableau de fidélité mesuré. État : non tranché.
3. **Parseur/écrivain XPT v5 (TS-140)** : en-têtes, membres, NAMESTR,
   observation records, missings typés. État : non couvert.
4. **XPT v8** (noms longs, libellés étendus, UTF-8) + WARNING de
   non-représentabilité v5. État : non couvert. Dépend : XPT v5.
5. **Branchement `PROC IMPORT`/`EXPORT`/`LIBNAME … XPORT`** avec mapping
   sas7bdat → `VarMeta` du sidecar, tests d'aller-retour en CI. État : non
   couvert. Dépend : 3 (et 1-2 pour sas7bdat).
6. **`cargo deny`** : contrôle de licences (garde-fou GPL). État : non couvert.

Oracle indépendant : le corpus de fichiers réels + différentiel
pandas/pyreadstat (hors CI), et la spec TS-140 comme référence écrite.

## Ordre recommandé (dépendances × bénéfice)

1. **Divergences déjà au corpus** (coût minime, validité immédiate) :
   GLM `OUTPUT OUT=` (lève `glm-oneway-predicted`), MEANS `OUTPUT` liste de
   stats, NPAR1WAY correction de continuité, REG `OUTEST=` SSE dep-var, CORR
   layout OUTP=/OUTS=, UNIVARIATE SKEWNESS/KURTOSIS en OUTPUT, FREQ
   `OUTPUT` CHISQ. Chacun borné par le cas `known-divergence` existant
   (attendus déjà écrits, jamais modifiés par l'implémenteur).
2. **BY généralisé** (motif `common::by` éprouvé) : GLM/ANOVA, LOGISTIC, puis
   les multivariés ; invariant mono-groupe comme garde.
3. **XPT v5 puis branchement IMPORT/EXPORT** (ADR 0004 items 3 puis 5) :
   interopérabilité FDA/CDISC à haut bénéfice, indépendante du reste.
4. **Résidus Base** : TABULATE `PCTN<>`, DATASETS APPEND (délégation triviale),
   REPORT FLOW, puis CATALOG sas7bdat lecture (après XPT).
5. **Statistiques dirigées par oracle** : GENMOD OFFSET/OUTPUT + gamma exact,
   MIXED slopes + Satterthwaite, GLIMMIX QUAD, LOGISTIC EFFECT/GLOGIT — chaque
   item adossé à un oracle externe publié (R/lme4/doc SAS).
6. **S3 écriture/listing** (MinIO en CI) puis **graphiques** (rendre
   parse-only, GROUP/légendes, images BY), puis **IML modules**, puis
   **sasrs-py** (ADR 0003), le plus gros morceau, en dernier.

## Correspondance ex-M46–M66 → items de cette feuille de route

| Ex-jalon (plan V2 figé) | Items correspondants (bornés par comportement) |
|---|---|
| **M46** — TABULATE `PCTN<dim>` | § Résidus Base / TABULATE — item `PCTN<dim>`/`PCTSUM<dim>` |
| **M47** — REPORT DEFINE FLOW + COMPUTE riche | § Résidus Base / REPORT — items FLOW et COMPUTE rétro-écriture (+ ordre d'évaluation) |
| **M48** — DATASETS APPEND/CONTENTS/MODIFY/REPAIR | § Résidus Base / DATASETS — items APPEND, CONTENTS (délégations), REPAIR (MODIFY RENAME/LABEL déjà implémentés) |
| **M49** — CATALOG catalogues réels | § Résidus Base / CATALOG — items sas7bcat lecture et ENTRYTYPE= |
| **M50** — PRINTTO routage réel | **Livré par J07-P5** (validé `compat/printto/*`) — reste hors feuille de route |
| **M51** — OPTIONS détail par option | § Résidus Base / OPTIONS — items OPTION=<name>, PAGESIZE, combos |
| **M52** — LOGISTIC complétion | § LOGISTIC — items EFFECT/GLM, GLOGIT, score PO, ESTIMATE, UNITS/ROC, BY |
| **M53** — GENMOD complétion (GEE) | § GENMOD — items gamma exact, OFFSET, ESTIMATE, OUTPUT, GEE, BY |
| **M54** — MIXED complétion (Kenward-Roger) | § MIXED — items slopes, LSMEANS/ESTIMATE, Satterthwaite/KR, COVTEST, TYPE étendus, BY |
| **M55** — GLIMMIX complétion (QUAD) | § GLIMMIX — items QUAD, slopes, GAMMA, WEIGHT, LSMEANS, LAPLACE étendu |
| **M56** — PRINCOMP complétion | § Multivarié — items OUTSTAT/TYPE=CORR, PARTIAL (+ BY commun) |
| **M57** — FACTOR complétion (ML) | § Multivarié — items METHOD=ML, HEYWOOD (+ BY commun) |
| **M58** — DISCRIM complétion | § Multivarié — items QDA POOL=NO, CROSSVALIDATE (+ BY commun) |
| **M59** — DISTANCE complétion | § BY commun (le reste — SHAPE/FREQ/normalisation — reste optionnel, bénéfice faible) |
| **M60** — CLUSTER complétion (CCC) | § Multivarié — items PSEUDO=, CCC |
| **M61** — FASTCLUS complétion | § Multivarié — item SEED=/RADIUS=/DISTANCE |
| **M62** — IML complétion | § IML — modules, CALL ROOT/LPSOLVE, READ WHERE, singularité INV |
| **M63** — GPLOT complétion | § Graphiques — items BY, SYMBOL/AXIS étendus |
| **M64** — GCHART complétion | § Graphiques — items HBAR horizontal, SUBGROUP=, BY |
| **M65** — PLOT complétion | § Graphiques — items HREF=/VREF=/axes, symboles par groupe, BY |
| **M66** — SGPLOT complétion | § Graphiques — items rendre HBAR/VBOX/REG, GROUP=/RESPONSE=/STAT=/SCALE=, légendes, overlays, images BY |

(M37–M45, antérieurs à M46, sont couverts par la consolidation J02–J08 ;
M46–M66 hors M50 sont intégralement remappés ci-dessus.)

## Préparation du `/milestone-plan` suivant

Cette feuille de route est prête à être reprise par `/milestone-plan` :

- chaque ligne du tableau est un **comportement vérifiable** (oracle + critère
  d'acceptation), directement traduisible en unité avec check exécutable ;
- l'ordre recommandé ci-dessus fournit le graphe de dépendances (divergences
  corpus → BY → XPT → résidus Base → stats par oracle → S3 → graphiques →
  IML → sasrs-py) ;
- les états actuels sont audités (ce document, README.md,
  conformance/STATUS.md) et le contrat « jamais de promesse au-delà du code »
  (CONTRIBUTING.md) s'applique à chaque unité ;
- les divergences `known-divergence` du corpus fournissent des unités à
  acceptation immédiate (le cas passe `validated`), modèle déjà éprouvé en
  J07/J08.
