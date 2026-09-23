## Résumé

<!-- Ce que change cette PR et pourquoi. Référence de la part/jalon
     (ex. J02-P3) et de l'issue le cas échéant. -->

…

## Reproducer

<!-- Programme SAS minimal (ou test) qui expose le comportement : avant cette PR
     il faisait/produisait quoi, après il fait/produit quoi. Obligatoire pour tout
     changement de sémantique (voir CONTRIBUTING.md §3). -->

…

## Tests

- [ ] Nouveaux tests unitaires / intégration / snapshot (lister les noms)
- [ ] `scripts/check.sh all` vert en local (mêmes commandes que la CI)

## Oracle et provenance

<!-- D'où viennent les valeurs attendues ? Citer précisément :
     - doc SAS 9.4 publiée (URL),
     - sortie SAS réelle (log/listing/dataset, provenance),
     - oracle indépendant (cas `conformance/cases/<id>/`, script Python…).
     L'agent qui implémente n'écrit pas le seul oracle qui le valide : si l'oracle
     est nouveau dans cette PR, dire qui l'a écrit vs qui a implémenté.
     Statut `known-divergence` : valeurs attendues inchangées, seul le statut
     peut changer (avec justification). -->

…

## Snapshots modifiés

- [ ] Aucun snapshot modifié
- [ ] Pour chaque `.snap` modifié, une ligne dans le commit :

      Snapshot: <fixture> — <raison>

## CI

- [ ] Check `ci-ok` vert sur le dernier commit de cette PR (la CI arbitre, pas
      l'affirmation d'un agent que « les tests sont verts »)
