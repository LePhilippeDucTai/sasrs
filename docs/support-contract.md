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

GLIMMIX partage encore l'ancienne forme `common::parse_response_options`
(`DESC` y est désormais honoré) ; la migration de son appel vers la forme
vérifiée relève de son unité de contrat (hors périmètre de J02-P1).

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
