# ADR 0003 — API Python native : pyo3/maturin au-dessus de `sasrs`

Statut : proposé (J06-P6). Portée : future crate `sasrs-py` (liaisons Python
 natives), feuille de route J08-P5. Unité d'analyse uniquement — aucun code
 dans cette unité.

## Contexte

Aujourd'hui la voie Python de sasrs est un **wrapper CLI**
(`python/src/sasrs_py/cli.py`, durci en J06-P4) : il télécharge le binaire
précompilé depuis une GitHub Release (SHA-256 vérifié, cache local, verrou
concurrent), puis l'invoque en sous-processus. Les données circulent en
fichiers Parquet + sidecar (ADR 0001) ; aucun objet Python ne manipule
directement la mémoire Rust.

Le cœur Rust expose déjà des structures publiques et stables
(`SasDataset`, le parseur/lexer, l'exécuteur de `PROC`s, la gestion
`DirLibrary` — cf. `src/dataset.rs`, `src/session.rs`, l'arborescence
`src/procs/`, `src/executor/`). Un module `sasrs::api` de façade est le point
d'entrée naturel prévu pour les consommateurs embarqués (J06-P1). La question
posée ici : faut-il exposer ce cœur à Python **nativement** via
[pyo3](https://github.com/PyO3/pyo3) + [maturin](https://github.com/PyO3/maturin),
et si oui, à quelles conditions et à quel coût ?

## Options considérées

1. **Statu quo — wrapper CLI sous-processus** : zéro compilation Python-
   spécifique, binaire unique, isolation par processus (timeout, kill).
   Limites : coût de sérialisation Parquet aller-retour à chaque appel, pas
   d'objets DataFrame en mémoire, latence de démarrage, pas d'accès
   incrémentiel (une session = un process).
2. **pyo3 + maturin, liaison native mince** : une crate `sasrs-py` qui
   enveloppe `sasrs::api` et expose quelques classes (`SasSession`,
   `SasDataset`) via PyO3. Les DataFrames passent par **Arrow C Data
   Interface** (via `arrow`/`pyarrow` ou `arrow::ffi_stream`), ce qui donne
   zéro-copie vers **Polars** et conversion O(1) vers **pandas 2.x**
   (`pa.Table.to_pandas`). C'est l'option retenue ci-dessous.
3. **pyo3 + Polars comme dépendance compilée** : rejeté pour l'instant.
   Compiler Polars dans la roue multiplie le temps de build (de l'ordre de
   plusieurs minutes à ~20 min en CI selon le parallélisme) et la taille des
   roues (des dizaines de Mo par plateforme) ; Polars doit rester une
   dépendance *optionnelle* du consommateur, pas embarquée. On échange
   Arrow via FFI, Polars le consomme sans qu'on le compile.
4. **Embedding Python côté Rust (PyO3 comme interpréteur hôte)** — rejeté :
   inverse la dépendance, complique la distribution du binaire CLI, aucun
   besoin utilisateur identifié.

## Échange de DataFrames

Convention retenue (option 2) : toute frontière Python↔Rust passe par
**Arrow** (record batches / C Data Interface ou IPC).

- `SasDataset` ↔ `arrow::record_batch::RecordBatch` : mappage direct des
  colonnes (le stockage est déjà Parquet/Arrow-ish, ADR 0001) ; les
  métadonnées du sidecar (formats, libellés, longueurs) voyagent en
  métadonnées de schéma Arrow (`schema.metadata`), pas en colonnes.
- vers Python : `pyarrow.Table` (zéro copie par FFI), puis
  `df = pl.from_arrow(tbl)` / `tbl.to_pandas()` au choix de l'appelant.
  **Polars et pandas ne sont jamais des dépendances obligatoires de
  `sasrs-py`** ; `pyarrow` est la seule dépendance Python recommandée
  (optionnelle si l'utilisateur ne consomme que des fichiers).

## Roues par plateforme

maturin construit des roues par triplet (OS × architecture × ABI). Coûts et
contraintes :

- cibles prioritaires : `manylinux_2_28 x86_64` (et aarch64 ensuite),
  `macosx_arm64`, `win_amd64` (MSVC). Le projet ne publie aujourd'hui qu'un
  asset Windows (cf. `cli.py`, `_PLATFORM_ASSETS`) ; la liaison native
  élargit la matrice, il faut l'assumer avec cibuildwheel/CI matricielle.
- taille : une liaison mince sans Polars reste de l'ordre de quelques Mo par
  roue (le cœur sasrs + arrow), compatible avec les réflexes déjà en place
  (le README du wrapper refuse d'embarquer un binaire de ~55 Mo dans les
  roues).
- ABI : viser `abi3` (pyo3 `abi3-py39`) pour une roue par OS/arch au lieu
  d'une par version de Python — cohérent avec la matrice CI Python 3.9/3.x
  du job J06-P4.
- découverte (discovery) par maturin : paquet `sasrs_py` dans
  `python/src/`, manifest séparé de l'espace de travail Cargo pour ne pas
  alourdir les builds du CLI.

## Coût de build Polars

Rappel chiffré pour la décision : ajouter `polars` comme **dépendance Cargo
de la roue** coûte typiquement 10–20 min de compilation propre en CI et
60+ Mo de roue décompressée, pour une valeur nulle (le consommateur qui
veut Polars l'a déjà). La voie Arrow-FFI apporte le même interop pour un
surcoût quasi nul. Conclusion : Polars jamais embarqué ; si un jour des
opérations Polars natives sont voulues dans la roue, ce sera un extra
`pip install sasrs[polars]` avec roues séparées.

## Coexistence avec la voie CLI

Les deux voies restent, avec des rôles distincts :

- **CLI/wrapper** : zéro installation Rust, reproductible, scripts one-shot,
  environnements sans compilateur ; le wrapper J06-P4 (téléchargement
  vérifié, cache, verrou, repli Windows) reste la voie par défaut jusqu'à
  publication des roues.
- **Liaison native `sasrs-py`** : sessions longues, pipelines
  DataFrame-idiomatiques, tests unitaires du cœur depuis `pytest`, usage
  notebook. Elle ne remplace pas le wrapper : elle l'utilise même comme
  repli possible (si l'extension native n'est pas installée, `sasrs_py`
  peut retomber sur le sous-processus — même surface d'API).
- point de vigilance : deux canaux de version (tag de release du binaire vs
  versions PyPI des roues) — aligner les versions et documenter la matrice
  supportée.

## Décision

Adopter **pyo3/maturin** comme voie d'exposition Python native, au-dessus de
`sasrs::api`, selon les conditions suivantes :

1. crate séparée `sasrs-py` (workspace séparé du CLI), liaison **mince** :
   `SasSession`, `SasDataset`, exécution d'étapes ; pas de logique métier
   dans la couche pyo3 ;
2. échange de données exclusivement via **Arrow** (C Data Interface / IPC) ;
   `pyarrow` seule dépendance Python conseillée ; Polars et pandas
   consommés côté utilisateur, jamais compilés ni embarqués dans la roue ;
3. roues **abi3** (py3.9+) par plateforme, prioritaires manylinux x86_64,
   macOS arm64, Windows x86_64 ; le wrapper CLI reste la voie par défaut
   tant que la matrice de roues n'est pas complète ;
4. coût Polars assumé comme non-objectif : aucun build Polars dans la roue
   (voir section ci-dessus) ;
5. coexistence garantie : API Python unifiée, repli CLI possible depuis la
   liaison, versions binaire/roue alignées.

## Items feuille de route (J08-P5)

- [ ] Définir la surface minimale `sasrs::api` (types, erreurs → exceptions
      Python typées `SasrsError` hiérarchique) préalable à la crate `sasrs-py`.
- [ ] Squelette `sasrs-py` : Cargo manifest maturin, paquet `python/src/sasrs_py`,
      test `pytest` de fumée (session + aller-retour RecordBatch).
- [ ] Conversion `SasDataset` ↔ Arrow RecordBatch avec métadonnées sidecar en
      `schema.metadata`, + tests de fidélité (formats, libellés).
- [ ] CI : job de build de roues matriciel (manylinux x86_64, macOS arm64,
      Windows x86_64), publication PyPI en draft, alignement des versions avec
      les tags de release du binaire CLI.
- [ ] Décider du repli CLI dans `sasrs_py` (import conditionnel de
      `sasrs_py.cli`) et documenter la matrice supportée dans `python/README.md`.
- [ ] Benchmark comparatif wrapper CLI vs liaison native (aller-retour Parquet
      vs Arrow FFI) pour arbitrer la migration par défaut.
