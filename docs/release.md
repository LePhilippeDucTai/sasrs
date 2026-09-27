# Procédure de release — sasrs

Ce document décrit la publication d'une version versionnée de `sasrs`
(J06-P5) : build multi-plateforme, `SHA256SUMS`, `manifest.json`, immutabilité
des assets, et l'arrêt préalable du watchdog Hermes (R-001).

## Vue d'ensemble

La release est produite par le workflow [`.github/workflows/release.yml`](../.github/workflows/release.yml),
déclenché **uniquement** par le push d'un tag `v*` (par ex. `v0.1.0`). Poser
le tag est un **geste utilisateur** : aucun workflow ne publie ni ne déplace un
tag de lui-même.

Pour chaque tag `v<version>`, le workflow publie :

| Asset | Contenu | Runner / cible |
|---|---|---|
| `sasrs-linux-x86_64.tar.gz` | binaire Linux | `x86_64-unknown-linux-gnu` |
| `sasrs-windows-x86_64.exe` | binaire Windows (nu : c'est le nom que le wrapper Python télécharge) | `x86_64-pc-windows-msvc` |
| `sasrs-macos-arm64.tar.gz` | binaire macOS Apple Silicon | `aarch64-apple-darwin` |
| `SHA256SUMS` | empreintes SHA-256 des trois binaires |
| `manifest.json` | version, commit, features, rustc, cibles |

Le workflow vérifie avant publication que le binaire rend bien
`sasrs <version> (commit <sha|unknown>, features: …)` via `--version` — le
commit et les features sont embarqués à la compilation par `build.rs`.

## Étapes (geste utilisateur)

1. **Vérifier l'état voulu** : `cargo test --locked` vert sur le commit à
   publier ; `Cargo.toml` porte la version `<version>` visée.
2. **Arrêter le watchdog Hermes (R-001)** — voir section dédiée ci-dessous.
3. **Poser le tag** sur le commit à publier :
   ```sh
   git tag v<version> <commit>
   git push origin v<version>
   ```
4. **Attendre le workflow `Release`** : le job `guard` refuse de publier si une
   release existe déjà pour ce tag ; les jobs `build` compilent les trois
   cibles, vérifient `--version`, puis `publish` génère `SHA256SUMS` et
   `manifest.json` et crée la release.
5. **Recopier les empreintes** : les SHA des binaires issus de l'asset
   `SHA256SUMS` de la release sont reportés dans les constantes du wrapper
   Python (`python/src/sasrs_py/cli.py`, une constante par ligne — conflit
   R-001), et le tag y est épinglé à `v<version>`.

## Immutabilité des assets

Les assets publiés ne sont **jamais remplacés** :

- le job `guard` échoue dès qu'une release existe déjà pour le tag ;
- la création finale utilise `gh release create`, qui échoue lui aussi si la
  release existe.

Une release défectueuse se corrige en publiant un **nouveau tag**
(`v<version>.1` ou version suivante), jamais en ré-uploasant par-dessus.
Supprimer une release existante est une décision humaine exceptionnelle (et
doit être tracée dans `DECISIONS.md`).

## Wrapper Python

Le wrapper (`python/src/sasrs_py/cli.py`) télécharge le binaire depuis la
release **épinglée** `v<version>` (constante `RELEASE_TAG`), vérifie son
empreinte SHA-256 contre les constantes embarquées — issues de l'asset
`SHA256SUMS` de cette même release — puis met le binaire en cache. Il n'y a
**plus aucune réécriture externe** des empreintes : les SHAs vivent uniquement
dans ce fichier, une constante nommée par ligne.

## Arrêt du watchdog Hermes (R-001)

Le watchdog Hermes réécrivait historiquement les lignes de SHAs du wrapper sur
`main` (conflit R-001), rendant les empreintes inauditables. Avant toute
publication :

1. arrêter le service Hermes (`hermes stop`, ou l'équivalent documenté pour
   l'installation locale) ;
2. vérifier que `python/src/sasrs_py/cli.py` est propre
   (`git diff -- python/src/sasrs_py/cli.py` vide) ;
3. publier selon les étapes ci-dessus ;
4. ne PAS relancer Hermes ensuite : la procédure de release ci-dessus rend le
   watchdog obsolète (les SHAs sont figés dans le dépôt, épinglés sur `v<version>`).

## Non-régression

- `tests/cli.rs::version_reports_commit_and_features` échoue si `--version`
  ne rend pas exactement une ligne
  `sasrs <version> (commit <sha|unknown>, features: …)`.
- Le workflow échoue si la release existe déjà, si `--version` est mal formé,
  ou si un artefact manque (`if-no-files-found: error`).
