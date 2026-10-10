# Corpus de conformité `sasrs`

Ce corpus vérifie que `sasrs` se comporte comme SAS 9.4 sur des programmes
réels, avec des valeurs attendues dont la **provenance est indépendante**
de l'implémentation (documentation SAS publiée, exécution SAS réelle, ou
oracle indépendant — jamais la sortie courante de `sasrs` elle-même).

- Structure exacte d'un cas : [`schema.md`](schema.md).
- Exécuteur : `tests/conformance.rs`, invoqué par la CI
  (`cargo test --locked -p sasrs --test conformance`). Avec `-- --nocapture`,
  il affiche une ligne par cas : `PASS`, `FAIL`, `DIVERGENT`, `PROMOTE` ou
  `BAD-STATUS`, suivie du chemin `conformance/cases/<…>/<id>`.
- Rapport : [`STATUS.md`](STATUS.md), généré par
  `python3 scripts/conformance_report.py` (`--check` en CI).
- Outil d'acceptation : `python3 scripts/conformance_require.py --status
  validated|known-divergence|any CAS…` exige que chaque cas (chemin sous
  `conformance/cases`, p. ex. `compat/means/means-class-missing-nway`)
  existe, porte le statut demandé, une provenance (`kind`, `source`) non
  vide, une `issue` s'il est `known-divergence`, et au moins un
  `expected/*.csv` (exit 0 = OK, 1 = exigence violée, 2 = usage).
  `--self-test` rejoue ces règles sur des copies temporaires.
- Oracles rejouables (J01-P5) : un cas `independent-oracle` peut porter
  `oracle/oracle.py`, un script reproduisant indépendamment ses
  `expected/*.csv` — voir « Oracle rejouable » ci-dessous et
  `schema.md`. `python3 scripts/replay_oracles.py` les découvre et les
  rejoue tous ; `--self-test` vérifie l'outil lui-même (cas conforme,
  valeur divergente, import interdit).

## Ajouter un cas

1. Créer `conformance/cases/<groupe>/<id>/` (ou un niveau plus profond,
   p. ex. `compat/<proc>/<id>/` — la découverte est récursive) avec
   `case.json`, `program.sas`, `data/*.csv` (entrées) et
   `expected/<dataset>.csv` (attendus). Le groupe `example` rassemble les
   cas de démonstration ; les cas de conformité série vont dans `base`
   (langage), `stat` (statistiques), `compat` (compatibilité)… L'`id` est le
   nom du répertoire et reste unique dans tout le corpus.
2. Déclarer la zone du rapport dans le champ `zone` du `case.json`
   (p. ex. `"zone": "PROC MEANS"`) ; sans ce champ ni entrée dans la table
   de `scripts/conformance_report.py`, le rapport échoue (exit 2).
3. Obtenir les valeurs attendues d'une source indépendante et la citer dans
   `provenance` (URL de la doc SAS, description de l'oracle, …). Un cas
   sans provenance exploitable est refusé par l'exécuteur.
4. `cargo test --locked -p sasrs --test conformance` doit passer ;
   régénérer `STATUS.md` (`python3 scripts/conformance_report.py`).

## Oracle rejouable

Un cas `independent-oracle` peut porter `oracle/oracle.py`, en plus de
`program.sas` : un script Python **bibliothèque standard seulement**,
déterministe, sans réseau ni sous-processus, qui n'exécute jamais `sasrs`.
Lancé comme `python3 -I -B oracle/oracle.py <dir>` (le répertoire du cas
comme répertoire courant), il doit écrire `<dir>/<dataset>.csv` pour
chaque `expected/<dataset>.csv` du cas — la preuve que l'attendu est
reproductible par un calcul indépendant, pas relevé une fois puis figé.

`python3 scripts/replay_oracles.py` découvre tous les
`conformance/cases/**/oracle/oracle.py` du corpus, refuse statiquement
(sans l'exécuter) tout script qui importe un module hors
`sys.stdlib_module_names`, ou `subprocess`/`socket`/`ctypes`/
`multiprocessing`, ou qui utilise `os.system`/`os.popen`, puis exécute
chaque script accepté dans un répertoire temporaire et compare sa sortie
à `expected/*.csv` avec les mêmes règles que l'exécuteur (missings SAS
exacts, colonnes dans l'ordre, tolérance `abs`/`rel` par défaut et par
colonne de `case.json`). Sans aucun oracle dans le corpus, l'outil
rapporte « 0 oracle(s) rejoué(s) » et sort en 0. `--self-test` vérifie
l'outil lui-même (cas conforme, valeur divergente, import interdit) sur
des cas fabriqués, sans toucher au corpus.

Exécuté par `scripts/check.sh test` et par le job CI `conformance`, à la
suite de `cargo test --test conformance` et `conformance_report.py --check`.

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
