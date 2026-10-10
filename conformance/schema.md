# Schéma du corpus de conformité `conformance/cases/`

Ce document fait autorité sur la structure d'un cas. Le guide d'usage
(goupes, politique de provenance, promotion des divergences) est dans
[`README.md`](README.md) ; l'exécuteur vit dans `tests/conformance.rs`.

## Arborescence d'un cas

```
conformance/cases/<groupe>/[<sous-groupe>/…]<id>/
├── case.json            métadonnées et attentes (obligatoire)
├── program.sas          programme SAS exécuté (obligatoire)
├── data/                entrées CSV (optionnel)
│   └── <table>.csv      converties en parquet par l'exécuteur
├── expected/            datasets attendus (au moins un)
│   └── <dataset>.csv    comparé à WORK.<dataset>
└── oracle/              oracle rejouable (optionnel, `independent-oracle`)
    └── oracle.py         rejoué par scripts/replay_oracles.py
```

Groupes actuels : `example` (cas de démonstration du corpus), `base`
(langage), `stat` (statistiques) et `compat` (compatibilité, rangé par PROC :
`compat/<proc>/<id>/`).

**Découverte récursive** : l'exécuteur (`tests/conformance.rs`) et le
rapport (`scripts/conformance_report.py`) parcourent `conformance/cases/`
à toute profondeur ; tout répertoire qui contient un `case.json` est un cas
(on ne descend pas en dessous : `data/` et `expected/` lui appartiennent).
L'`id` du cas est le nom de ce répertoire et doit être unique dans tout le
corpus ; le chemin relatif (`conformance/cases/compat/means/<id>`) est celui
affiché par l'exécuteur et, sans le préfixe, par `STATUS.md`.

## `case.json`

Schéma strict (J01-P8) : toute clé absente des tableaux ci-dessous, à
n'importe quel niveau (`case.json`, `provenance`, `tolerance`,
`tolerance.columns.<COL>`, `log`, `listing`), est une erreur de lecture —
une clé mal orthographiée (`"listing": {"require": […]}`) ne peut plus être
ignorée en silence.

| Champ | Type | Obligatoire | Description |
|---|---|---|---|
| `id` | string | oui | Identifiant du cas, = nom du répertoire. |
| `title` | string | oui | Courte description lisible. |
| `zone` | string | non | Zone (PROC / zone du langage) du cas dans `STATUS.md`, p. ex. `PROC MEANS`. À défaut, `scripts/conformance_report.py` la lit dans sa table `ZONE_BY_CASE_ID` ; un cas sans l'un ni l'autre fait échouer le rapport (exit 2). Une chaîne vide est refusée. |
| `provenance` | objet | oui | D'où viennent les valeurs attendues (voir ci-dessous). |
| `validates` | string | oui | `math` (numérique/flottants) ou `sas-behaviour` (sémantique documentée du langage). |
| `tolerance` | objet | oui | Tolérances numériques (voir ci-dessous). |
| `log` | objet | non | Attentes sur la log : `required` (sous-chaînes qui DOIVENT apparaître), `forbidden` (regex [fancy-regex] qui ne doivent PAS apparaître). |
| `listing` | objet | non | Attentes sur le listing (`run.lst`, sortie `--print`) : même forme et mêmes règles que `log`. Un listing absent est traité comme vide. |
| `exit_code` | entier | non | Code retour attendu du CLI (défaut `0`). |
| `status` | string | oui | `validated` ou `known-divergence`. |
| `issue` | string | si `known-divergence` | Référence du problème documentant la divergence (non vide). |
| `data_types` | objet | si `data/` existe | Types des colonnes d'entrée : `{ "<fichier>.csv": { "<colonne>": "num"\|"char" } }`. |

### `provenance`

L'implémenteur d'une fonctionnalité n'écrit jamais le seul oracle qui la
valide (CONTRIBUTING.md §2) : la provenance est obligatoire et auditée.

| Champ | Type | Description |
|---|---|---|
| `kind` | `sas-doc` \| `sas-run` \| `independent-oracle` | Documentation SAS 9.4 publiée / exécution SAS réelle / oracle indépendant (script Python, calcul indépendant…). |
| `source` | string | URL ou référence précise (page, version, script). |
| `sas_version` | string | Version SAS de référence (p. ex. `9.4M5`). |
| `options` | array | Options CLI requises (p. ex. `["--deterministic"]`). |

### `tolerance`

```json
{
  "abs": 1e-9,
  "rel": 1e-9,
  "columns": { "POIDS": { "abs": 0.5 } }
}
```

- `abs`/`rel` (défaut `1e-9`) : tolérances par défaut de toutes les colonnes
  numériques ; la comparaison passe si `|got − want| ≤ abs` **ou**
  `|got − want| ≤ rel · max(|got|, |want|)`.
- `columns` : surcharge par colonne (clé = nom de colonne en MAJUSCULES).
- Les colonnes caractère et les **missings** sont comparés exactement :
  `.`/vide = missing ordinaire, `._`, `.A`…`.Z` = missings spéciaux — un
  missing n'est jamais « proche » d'une valeur ni d'un autre missing.

## Formats CSV

- `data/<table>.csv` : première ligne = noms de colonnes ; une colonne
  déclarée `num` accepte les nombres et les missings (`.` vide, `._`, `.A`…
  `.Z`), encodés en parquet comme le fait le crate (null = `.`, NaN à
  payload = spécial).
- `expected/<dataset>.csv` : première ligne = noms de colonnes en MAJUSCULES,
  **dans l'ordre des variables du dataset produit** (ordre de compilation
  SAS) ; chaque ligne = une observation, mêmes conventions de missing.
  Le dataset produit est relu depuis le répertoire `--work` via l'API
  publique `sasrs::dataset::SasDataset::read_parquet`.

## `oracle/oracle.py` (oracle rejouable)

Un cas dont `provenance.kind` vaut `independent-oracle` peut porter
`oracle/oracle.py`, rejoué par `scripts/replay_oracles.py` (cf.
`README.md` §« Oracle rejouable ») :

- bibliothèque standard Python uniquement (aucun import hors
  `sys.stdlib_module_names`), sans module de réseau, de sous-processus, de
  code natif ou d'import dynamique (`subprocess`, `socket`, `ssl`,
  `urllib`, `http`, `ftplib`, `smtplib`, `importlib`, `ctypes`,
  `multiprocessing`…), sans attribut de lancement de processus sur aucun
  objet (`system`, `popen`, `spawn*`, `exec*`, `fork`…, donc aussi via un
  alias de `os`), sans `__import__`/`eval`/`exec`/`compile`, `getattr` à
  nom non littéral ni attribut dunder hors liste blanche, sans chaîne
  littérale citant `expected/` — `replay_oracles.py` refuse statiquement
  (`ast`, sans exécuter) tout script qui y manque ;
- déterministe, sans réseau, sans sous-processus, et n'exécute jamais
  `sasrs` (il ne prouve rien s'il relit la sortie qu'il est censé
  valider) ;
- invoqué comme `python3 -I -B oracle/oracle.py <dir>`, avec pour
  répertoire courant un bac à sable temporaire qui ne contient que des
  copies de `data/` et `oracle/` du cas (jamais `expected/`), `<dir>` un
  autre répertoire temporaire fourni par `replay_oracles.py` ;
- doit écrire `<dir>/<dataset>.csv` pour chaque `expected/<dataset>.csv`
  du cas, même format que `expected/` (en-tête en MAJUSCULES dans l'ordre
  des variables, mêmes conventions de missing) ;
- comparé à `expected/<dataset>.csv` avec les tolérances de `tolerance`
  (mêmes règles que l'exécuteur : missings exacts, colonnes dans l'ordre,
  `abs`/`rel` par défaut et par colonne).

## Exécution par l'exécuteur

Pour chaque cas, les métadonnées sont d'abord validées, quel que soit le
statut : `id` = nom du répertoire, `status`/`validates`/`provenance.kind`
dans leur vocabulaire, `issue` non vide si `known-divergence`,
`provenance.source`/`sas_version` non vides, `provenance.options` gérées,
`zone` non vide si présente, regex `log.forbidden`/`listing.forbidden`
valides, types `data_types` dans `num|char`. Un échec → `BAD-STATUS`
(corpus en échec), le cas n'est pas exécuté — un `known-divergence` mal
décrit n'est jamais classé `DIVERGENT`.

Puis : tempdir frais, `data/*.csv` → `<tmp>/data/*.parquet`
(types de `data_types`), puis

```
sasrs --work <tmp>/work --log <tmp>/run.log --print <tmp>/run.lst \
      --deterministic <tmp>/program.sas      # cwd = <tmp>
```

Sont vérifiés : code retour, lignes de log requises, regex interdites
de la log, mêmes attentes sur le listing `run.lst` (champ `listing`), et
chaque `expected/<ds>.csv` contre `WORK.<ds>`. Un cas `validated` doit
tout passer ; un cas `known-divergence` doit échouer au moins une
vérification — s'il passe, l'exécuteur signale « à promouvoir » et échoue.
