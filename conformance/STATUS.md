<!-- conformance_report:begin -->
<!-- Généré par scripts/conformance_report.py à partir des
     conformance/cases/**/case.json — NE PAS ÉDITER À LA MAIN. -->
<!-- Régénérer : python3 scripts/conformance_report.py -->

# Statut de conformité sasrs ↔ SAS 9.4

Corpus : **36 cas** — 36 validés, 0 divergences connues.

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
| PROC COMPARE | 2 | 2 | 0 |
| PROC CONTENTS — informats persistés | 2 | 2 | 0 |
| PROC CORR | 2 | 2 | 0 |
| PROC FREQ | 3 | 3 | 0 |
| PROC GLM | 1 | 1 | 0 |
| PROC LOGISTIC | 1 | 1 | 0 |
| PROC MEANS | 3 | 3 | 0 |
| PROC NPAR1WAY | 1 | 1 | 0 |
| PROC PRINTTO | 2 | 2 | 0 |
| PROC REG | 1 | 1 | 0 |
| PROC SORT | 1 | 1 | 0 |
| PROC SQL | 2 | 2 | 0 |
| PROC TRANSPOSE | 3 | 3 | 0 |
| PROC TTEST | 1 | 1 | 0 |
| PROC UNIVARIATE | 2 | 2 | 0 |

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

### PROC COMPARE

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `compat/compare` | `compare-outbase-outcomp-id` | validé | documentation SAS publiée |
| `compat/compare` | `compare-outdif-by-criterion` | validé | documentation SAS publiée |

### PROC CONTENTS — informats persistés

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `compat/informat` | `informat-contents-date` | validé | documentation SAS publiée |
| `compat/informat` | `informat-contents-declared` | validé | documentation SAS publiée |

### PROC CORR

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `stat` | `corr-pearson-outp` | validé | documentation SAS publiée |
| `stat` | `corr-spearman-outs` | validé | documentation SAS publiée |

### PROC FREQ

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `base` | `freq-chisq-output` | validé | documentation SAS publiée |
| `stat` | `freq-fisher-2x2` | validé | documentation SAS publiée |
| `base` | `freq-tables-out` | validé | documentation SAS publiée |

### PROC GLM

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `stat` | `glm-oneway-predicted` | validé | documentation SAS publiée |

### PROC LOGISTIC

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `stat` | `logistic-binary-output` | validé | documentation SAS publiée |

### PROC MEANS

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `compat/means` | `means-class-missing-nway` | validé | documentation SAS publiée |
| `base` | `means-class-output` | validé | documentation SAS publiée |
| `compat/means` | `means-order-freq-autoname` | validé | documentation SAS publiée |

### PROC NPAR1WAY

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `stat` | `npar1way-wilcoxon-out` | validé | documentation SAS publiée |

### PROC PRINTTO

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `compat/printto` | `printto-log-new` | validé | documentation SAS publiée |
| `compat/printto` | `printto-print-routing` | validé | documentation SAS publiée |

### PROC REG

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `stat` | `reg-simple-lineart` | validé | documentation SAS publiée |

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
| `compat/transpose` | `transpose-copy-suffix` | validé | documentation SAS publiée |
| `compat/transpose` | `transpose-id-let-delimiter` | validé | documentation SAS publiée |
| `base` | `transpose-var` | validé | documentation SAS publiée |

### PROC TTEST

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `stat` | `ttest-twosample-pooled` | validé | documentation SAS publiée |

### PROC UNIVARIATE

| Groupe | Cas | Statut | Provenance |
|---|---|---|---|
| `stat` | `univariate-moments-output` | validé | documentation SAS publiée |
| `stat` | `univariate-weighted-output` | validé | documentation SAS publiée |

## Provenance des attendus

Chaque `case.json` cite sa source sous `provenance` (documentation SAS publiée, oracle indépendant ou exécution SAS réelle) ; l'attendu n'est jamais la sortie courante de `sasrs`. Les valeurs d'un cas `known-divergence` ne sont jamais modifiées par l'implémenteur de la correction — seul le statut change, avec justification.

<!-- conformance_report:end -->
