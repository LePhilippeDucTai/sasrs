<!-- conformance_report:begin -->
<!-- Généré par scripts/conformance_report.py à partir des
     conformance/cases/*/*/case.json — NE PAS ÉDITER À LA MAIN. -->
<!-- Régénérer : python3 scripts/conformance_report.py -->

# Statut de conformité sasrs ↔ SAS 9.4

Corpus : **26 cas** — 19 validés, 7 divergences connues.

Un cas `validated` **doit passer** (un échec est une régression) ; un cas `known-divergence` **doit échouer** (divergence documentée entre `sasrs` et SAS, cf. la colonne Issue). La définition des statuts et la provenance des attendus sont détaillées dans [`conformance/README.md`](README.md) ; c'est ce rapport que la marque « *validated against a reference* » du [`README.md`](../README.md) désigne.

## Vue d'ensemble par zone

| Zone | Cas | Validés | Divergences connues |
|---|---|---|---|
| DATA step — FIRST./LAST. | 1 | 1 | 0 |
| DATA step — MERGE | 1 | 1 | 0 |
| DATA step — RETAIN | 1 | 1 | 0 |
| DATA step — UPDATE | 1 | 1 | 0 |
| DATA step — arithmétique | 1 | 1 | 0 |
| DATA step — fonctions caractère | 1 | 1 | 0 |
| DATA step — missing spéciaux | 1 | 1 | 0 |
| DATA step — tableaux | 1 | 1 | 0 |
| FORMAT / PUT | 1 | 1 | 0 |
| PROC CORR | 2 | 0 | 2 |
| PROC FREQ | 3 | 2 | 1 |
| PROC GLM | 1 | 0 | 1 |
| PROC LOGISTIC | 1 | 1 | 0 |
| PROC MEANS | 1 | 1 | 0 |
| PROC NPAR1WAY | 1 | 0 | 1 |
| PROC REG | 1 | 0 | 1 |
| PROC SORT | 1 | 1 | 0 |
| PROC SQL | 2 | 2 | 0 |
| PROC TRANSPOSE | 1 | 1 | 0 |
| PROC TTEST | 1 | 1 | 0 |
| PROC UNIVARIATE | 2 | 1 | 1 |

## Détail par zone

### DATA step — FIRST./LAST.

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `base` | `first-last-groups` | validé | documentation SAS publiée |

### DATA step — MERGE

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `base` | `merge-by-match` | validé | documentation SAS publiée |

### DATA step — RETAIN

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `base` | `retain-cumulative` | validé | documentation SAS publiée |

### DATA step — UPDATE

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `base` | `update-master` | validé | documentation SAS publiée |

### DATA step — arithmétique

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `example` | `bmi-calculation` | validé | oracle indépendant |

### DATA step — fonctions caractère

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `base` | `char-date-functions` | validé | documentation SAS publiée |

### DATA step — missing spéciaux

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `example` | `special-missing-flag` | validé | documentation SAS publiée |

### DATA step — tableaux

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `base` | `data-array-impute` | validé | documentation SAS publiée |

### FORMAT / PUT

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `base` | `format-value-put` | validé | documentation SAS publiée |

### PROC CORR

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `stat` | `corr-pearson-outp` | divergence connue | documentation SAS publiée |
| `stat` | `corr-spearman-outs` | divergence connue | documentation SAS publiée |

Divergences connues :
- `corr-pearson-outp` — null — divergence à ouvrir par le coordinateur : OUTP= ne contient que 5 observations (_TYPE_=MEAN, STD, N puis CORR×2) là où la doc SAS décrit une observation par statistique descriptive (N, MEAN, STD, SUM, MIN, MAX — soit 6 lignes, N en premier) plus les 2 lignes CORR ; l'ordre des lignes et leur nombre divergent (les valeurs de corrélation elles-mêmes, 0.9600051599448531, sont conformes)
- `corr-spearman-outs` — null — divergence à ouvrir par le coordinateur : mêmes écarts de layout qu'en Pearson — OUTS= ne contient que 5 observations (_TYPE_=MEAN, STD, N puis CORR×2) au lieu des statistiques descriptives N/MEAN/STD/SUM/MIN/MAX + 2 lignes CORR de la doc ; rho de Spearman lui-même conforme (0.9428571428571428)

### PROC FREQ

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `base` | `freq-chisq-output` | validé | documentation SAS publiée |
| `stat` | `freq-fisher-2x2` | divergence connue | documentation SAS publiée |
| `base` | `freq-tables-out` | validé | documentation SAS publiée |

Divergences connues :
- `freq-fisher-2x2` — null — divergence à ouvrir par le coordinateur : l'instruction OUTPUT n'est pas supportée dans PROC FREQ (ERROR: The OUTPUT statement is not supported in PROC FREQ; it can affect results and cannot be ignored. — code retour 2, dataset WORK.FS non produit) ; la p-value exacte de Fisher reste calculable côté listing mais pas en dataset

### PROC GLM

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `stat` | `glm-oneway-predicted` | divergence connue | documentation SAS publiée |

Divergences connues :
- `glm-oneway-predicted` — null — divergence à ouvrir par le coordinateur : l'instruction OUTPUT n'est pas supportée dans PROC GLM (ERROR: The OUTPUT statement is not supported in PROC GLM; it can affect results and cannot be ignored. — code retour 2, dataset WORK.PRED non produit) ; l'ANOVA elle-même est correcte (Model MS=72.33333, Error MS=1.0)

### PROC LOGISTIC

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `stat` | `logistic-binary-output` | validé | documentation SAS publiée |

### PROC MEANS

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `base` | `means-class-output` | validé | documentation SAS publiée |

### PROC NPAR1WAY

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `stat` | `npar1way-wilcoxon-out` | divergence connue | documentation SAS publiée |

Divergences connues :
- `npar1way-wilcoxon-out` — null — divergence à ouvrir par le coordinateur : la Z de Wilcoxon applique une correction de continuité (0.5) absente de la formule de la doc SAS — _WIL_=89 conforme, mais Z_WIL=2.8353240556 produit vs 2.893187811789223 attendu (sans correction) et P2_WIL=0.0045779224 produit vs 0.00381353188258207 attendu ; P1_WIL supplémentaire géré

### PROC REG

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `stat` | `reg-simple-lineart` | divergence connue | documentation SAS publiée |

Divergences connues :
- `reg-simple-lineart` — null — divergence à ouvrir par le coordinateur : colonne de la variable dépendante dans OUTEST= — sasrs produit -1 là où la doc SAS donne la SSE de l'observation _TYPE_=PARMS (0.07276190476190475 attendu) ; _RMSE_, Intercept et fert sont conformes (0.13487207342691884, 0.053333333333334565, 1.9942857142857142), de même que OUTPUT p=/r=

### PROC SORT

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `base` | `sort-nodupkey` | validé | documentation SAS publiée |

### PROC SQL

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `base` | `sql-join-remerge` | validé | documentation SAS publiée |
| `base` | `sql-select-computed` | validé | documentation SAS publiée |

### PROC TRANSPOSE

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `base` | `transpose-var` | validé | documentation SAS publiée |

### PROC TTEST

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `stat` | `ttest-twosample-pooled` | validé | documentation SAS publiée |

### PROC UNIVARIATE

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `stat` | `univariate-moments-output` | divergence connue | documentation SAS publiée |
| `stat` | `univariate-weighted-output` | validé | documentation SAS publiée |

Divergences connues :
- `univariate-moments-output` — null — divergence à ouvrir par le coordinateur : statistiques SKEWNESS et KURTOSIS non supportées dans l'instruction OUTPUT de PROC UNIVARIATE (ERROR: Unsupported statistic 'SKEWNESS' in OUTPUT statement. — code retour 2, dataset WORK.MOM non produit) ; n/mean/std/min/max sont acceptés

## Provenance des attendus

Chaque `case.json` cite sa source sous `provenance` (documentation SAS publiée, oracle indépendant ou exécution SAS réelle) ; l'attendu n'est jamais la sortie courante de `sasrs`. Les valeurs d'un cas `known-divergence` ne sont jamais modifiées par l'implémenteur de la correction — seul le statut change, avec justification.

<!-- conformance_report:end -->
