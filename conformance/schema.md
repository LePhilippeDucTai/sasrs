# Schéma du corpus de conformité `conformance/cases/`

Ce document fait autorité sur la structure d'un cas. Le guide d'usage
(goupes, politique de provenance, promotion des divergences) est dans
[`README.md`](README.md) ; l'exécuteur vit dans `tests/conformance.rs`.

## Arborescence d'un cas

```
conformance/cases/<groupe>/<id>/
├── case.json            métadonnées et attentes (obligatoire)
├── program.sas          programme SAS exécuté (obligatoire)
├── data/                entrées CSV (optionnel)
│   └── <table>.csv      converties en parquet par l'exécuteur
└── expected/            datasets attendus (au moins un)
    └── <dataset>.csv    comparé à WORK.<dataset>
```

Groupes actuels : `example` (cas de démonstration du corpus),
`base`/`stat`/`compat` (réservés aux unités suivantes). L'exécuteur parcourt
TOUS les groupes présents.

## `case.json`

| Champ | Type | Obligatoire | Description |
|---|---|---|---|
| `id` | string | oui | Identifiant du cas, = nom du répertoire. |
| `title` | string | oui | Courte description lisible. |
| `provenance` | objet | oui | D'où viennent les valeurs attendues (voir ci-dessous). |
| `validates` | string | oui | `math` (numérique/flottants) ou `sas-behaviour` (sémantique documentée du langage). |
| `tolerance` | objet | oui | Tolérances numériques (voir ci-dessous). |
| `log` | objet | non | Attentes sur la log : `required` (sous-chaînes qui DOIVENT apparaître), `forbidden` (regex [fancy-regex] qui ne doivent PAS apparaître). |
| `exit_code` | entier | non | Code retour attendu du CLI (défaut `0`). |
| `status` | string | oui | `validated` ou `known-divergence`. |
| `issue` | string | si `known-divergence` | Référence du problème documentant la divergence. |
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

## Exécution par l'exécuteur

Pour chaque cas : tempdir frais, `data/*.csv` → `<tmp>/data/*.parquet`
(types de `data_types`), puis

```
sasrs --work <tmp>/work --log <tmp>/run.log --print <tmp>/run.lst \
      --deterministic <tmp>/program.sas      # cwd = <tmp>
```

Sont vérifiés : code retour, lignes de log requises, regex interdites,
et chaque `expected/<ds>.csv` contre `WORK.<ds>`. Un cas `validated` doit
tout passer ; un cas `known-divergence` doit échouer au moins une
vérification — s'il passe, l'exécuteur signale « à promouvoir » et échoue.
