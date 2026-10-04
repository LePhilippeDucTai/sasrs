# PROC REG OUTEST= : provenance et correction de l'oracle (#22)

## Convention SAS

La documentation **SAS/STAT 13.2, The REG Procedure, Output Data Sets,
OUTEST= Data Set** indique que la colonne de la variable dépendante vaut
**−1** dans l'observation `_TYPE_=PARMS` :

https://support.sas.com/documentation/cdl/en/statug/67523/HTML/default/statug_reg_details04.htm

La même convention est illustrée par **SAS/STAT 14.3, exemple 100.3,
Output 100.3.6, OUTEST Data Set** (colonne `weight` à −1) :

https://support.sas.com/documentation/onlinedoc/stat/143/reg.pdf

Ces références décrivent le format SAS ; les calculs MCO ci-dessous justifient
les statistiques numériques. Il ne s'agit pas d'une nouvelle exécution SAS.

## Calcul indépendant sur `data/sample.csv`

Avec six observations, une constante et un régresseur :

- moyenne de `fert` : 7/2 ; moyenne de `yield` : 211/30 ;
- somme des carrés centrés de `fert` : 35/2 ;
- somme des produits centrés : 349/10 ;
- pente : 349/175 ; constante : 4/75 ;
- résidus : 11/210, −149/1050, 86/525, −137/1050, 79/1050, −2/105 ;
- SSE : 191/2625, soit environ 0.07276190476190476 ;
- degrés de liberté résiduels : 6 − 2 = 4 ;
- `_RMSE_` : racine de (191/10500), soit 0.13487207342691884.

L'écart d'arrondi avec la SSE initiale 0.07276190476190475 est sans effet
à la tolérance du cas. L'erreur était son emplacement dans `est.yield`.

## Diagnostic et portée du correctif

L'attendu erroné est présent dès l'introduction du cas dans le commit
`a5abae1`. L'issue [#22](https://github.com/LePhilippeDucTai/sasrs/issues/22)
a repris son interprétation incorrecte de la documentation.

`src/procs/reg/output/outest.rs::write_outest()` écrit explicitement
`Some(-1.0)` dans la colonne dépendante de la ligne PARMS. Ce comportement
est conforme. `build_outest_entry()` utilise séparément SSE / ddl pour
calculer `_RMSE_` ; `OUTPUT p=/r=` conserve les prédictions et résidus.

Cette correction de corpus ne modifie aucune sémantique du moteur et ne
constitue pas un ajustement de l'oracle pour valider une nouvelle implémentation
(CONTRIBUTING.md §2). Une seule cellule attendue change, sur la base de la
documentation SAS indépendante : `expected/est.csv`, `yield`, SSE → −1.
Les autres coefficients, `_RMSE_`, `expected/pred.csv` et les tolérances restent
inchangés. Le statut passe à `validated` pour rendre tout futur écart bloquant.

Le test `outest_parms_dependent_marker_is_independent_of_sse` vérifie les
coefficients, `_RMSE_`, prédictions et résidus contre ce calcul rationnel,
puis multiplie la réponse par 10 : la SSE est multipliée par 100, tandis
que le marqueur dépendant reste exactement −1.
