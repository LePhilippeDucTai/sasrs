# Corpus de conformité `sasrs`

Ce corpus vérifie que `sasrs` se comporte comme SAS 9.4 sur des programmes
réels, avec des valeurs attendues dont la **provenance est indépendante**
de l'implémentation (documentation SAS publiée, exécution SAS réelle, ou
oracle indépendant — jamais la sortie courante de `sasrs` elle-même).

- Structure exacte d'un cas : [`schema.md`](schema.md).
- Exécuteur : `tests/conformance.rs`, invoqué par la CI
  (`cargo test --locked -p sasrs --test conformance`).

## Ajouter un cas

1. Créer `conformance/cases/<groupe>/<id>/` avec `case.json`, `program.sas`,
   `data/*.csv` (entrées) et `expected/<dataset>.csv` (attendus). Le groupe
   `example` rassemble les cas de démonstration ; les cas de conformité
   série vont dans `base` (langage), `stat` (statistiques), `compat`
   (compatibilité)…
2. Obtenir les valeurs attendues d'une source indépendante et la citer dans
   `provenance` (URL de la doc SAS, description de l'oracle, …). Un cas
   sans provenance exploitable est refusé par l'exécuteur.
3. `cargo test --locked -p sasrs --test conformance` doit passer.

## Statuts

- `validated` : le cas DOIT passer. Un échec est une régression (ou un bug
  du corpus — dans ce cas la correction vient avec sa propre justification,
  jamais en « alignant » l'attendu sur la sortie de `sasrs`).
- `known-divergence` (+ `issue`) : divergence connue et documentée entre
  `sasrs` et SAS. Le cas DOIT échouer. S'il passe, l'exécuteur signale
  « à promouvoir » et échoue : la promotion vers `validated` est une
  décision explicite (issue référencée), pas un glissement silencieux.

Les valeurs attendues d'un cas `known-divergence` ne sont jamais modifiées
par l'implémenteur de la correction — seul le statut change, avec
justification (CONTRIBUTING.md §2).

## Relations avec les autres suites

- Indépendant des snapshots insta (`tests/snapshots/`) : aucun `.snap` n'est
  lu ni écrit ici. Un snapshot verrouille une sortie ; un cas de conformité
  la confronte à SAS.
- `tests/storage_integrity.rs` verrouille le protocole de stockage ; le
  corpus ne réimplémente pas le lecteur parquet : les datasets produits
  sont relus via les API publiques du crate (`sasrs::dataset`,
  `sasrs::missing`, `sasrs::value`).
