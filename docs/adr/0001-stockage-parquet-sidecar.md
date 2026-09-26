# ADR 0001 — Stockage Parquet + sidecar : protocole d'écriture atomique

Statut : accepté (J04-P1). Portée : `SasDataset::write_parquet`/`write_sidecar`,
`DirLibrary::write`/`list`/`delete`.

## Contexte

Une table SAS persiste en deux fichiers : `<table>.parquet` (données, types) et
`<table>.parquet.sasmeta.json` (sidecar : format, libellé, longueur déclarée —
des métadonnées que le Parquet ne porte pas). Jusqu'ici l'écriture était
directe : `File::create` sur le parquet puis `fs::write` sur le sidecar. Une
interruption (panne processus, kill, pleine absence de courant) au milieu de
cette séquence laissait le stockage dans un état incohérent :

- parquet tronqué/à moitié écrit (table illisible), ou
- **nouveau parquet publié avec l'ANCIEN sidecar** : des formats et libellés
  d'une version précédente des colonnes s'appliquaient silencieusement aux
  nouvelles données — la pire des corruptions, car indolore et invisible.

SAS lui-même garantit qu'une étape DATA interrompue ne remplace jamais la
version précédente d'une table ; le stockage doit offrir la même garantie.

## Options considérées

1. **Écriture directe (statu quo)** — rejetée : les deux états corrompus
   ci-dessus, aucun outil de récupération.
2. **Temporaires dans un répertoire système (`TMPDIR`) + copie** — rejetée : le
   `rename(2)` n'est atomique que au sein d'un même système de fichiers ;
   `TMPDIR` est souvent sur un autre montage que la bibliothèque (tmpfs),
   il faudrait retomber sur une copie non atomique.
3. **Journal (write-ahead log) + rejeu** — rejetée : machinery lourde
   (format de log, points de contrôle, rejeu au démarrage) pour un gain nul
   sur la garantie qui compte ici ; le parquet fait déjà autorité pour les
   données.
4. **Temporaires dans le MÊME dossier que la cible + fsync + rename, sidecar
   horodaté par une empreinte du parquet** — retenue : coût minimal (un
   temporaire de plus, deux fsync), tout `rename` est atomique, et l'empreinte
   rend un sidecar périmé détectable et inoffensif.

## Décision

Protocole en quatre temps, implémenté dans `SasDataset::write_parquet` et
`SasDataset::write_sidecar` (et donc dans `DirLibrary::write`, qui délègue) :

1. **Parquet → temporaire** `<cible>.sasrs-tmp.<pid>` dans le même dossier,
   puis `fsync` du fichier. Le fichier final n'est pas touché.
2. **Publication des données** : `rename` atomique du temporaire vers
   `<table>.parquet`, puis `fsync` du dossier.
3. **Sidecar → temporaire** `<table>.parquet.sasmeta.json.sasrs-tmp.<pid>`,
   `fsync`, puis `rename` + `fsync` du dossier. Si le dataset n'a AUCUNE
   métadonnée à persister, l'ancien sidecar est simplement supprimé.
4. **Empreinte** : le sidecar porte l'empreinte du parquet publié —
   `{ taille en octets, nombre de lignes, nombre de colonnes }`. À la
   lecture, un sidecar dont l'empreinte ne correspond pas au parquet qui
   l'accompagne est **périmé : ignoré, avec un WARNING dans le log** ; les
   VarMeta sont alors redéduits du Parquet seul.

**Ordre garanti** : le parquet est TOUJOURS publié avant le sidecar. Une
interruption ne peut donc produire que (a) données anciennes + métadonnées
anciennes cohérentes, (b) données nouvelles + sidecar absent ou périmé
(ignoré avec diagnostic) — jamais de nouvelles données affublées de
métadonnées fausses.

Le format Parquet et la décision « sidecar JSON » sont inchangés ; le sidecar
gagne simplement deux champs (`fingerprint`, `vars`).

**Injection de pannes** (tests) : la feature cargo `fault-injection` (hors
défaut, AUCUNE dépendance) offre des points d'arrêt nommés —
`after_parquet_tmp`, `after_parquet_rename`, `after_sidecar_tmp`,
`after_sidecar_rename` — pilotés par la variable d'environnement
`SASRS_FAULT_INJECT` (liste de noms séparés par des virgules) ; un point
atteint termine le processus sur-le-champ (simulation d'interruption brute,
sans unwind ni destructeur). Hors feature, le coût est nul (fonction vide
inlinée).

## Conséquences

- Un fsync fichier supplémentaire par écriture de table, plus deux fsync de
  dossier : négligeable devant l'écriture Parquet elle-même, et c'est le prix
  de la durabilité.
- Les sidecars écrits avant ce protocole (sans empreinte) sont traités comme
  périmés : ignorés avec WARNING, métadonnées redéduites du Parquet. La perte
  est cosmétique (format/libellé) et s'élimine en réécrivant la table.
- Les temporaires d'une écriture interrompue restent sur disque jusqu'à la
  prochaine écriture de la même cible (purge) — ils sont toujours ignorés par
  `DirLibrary::list`.
- `DirLibrary::delete` supprime désormais le sidecar avec le parquet, pour
  qu'un sidecar orphelin ne puisse pas correspondre par coïncidence à une
  future réécriture de même empreinte.

## Stratégie de récupération

La récupération est paresseuse (au coup par coup, pas de phase de démarrage) :

- **Temporaires orphelins** (`*.sasrs-tmp.*`) : ignorés par `list` (jamais des
  tables), purgés au début de la prochaine écriture de la même cible.
- **Sidecar périmé** (empreinte ≠ parquet présent, y compris sidecar
  illisible ou d'ancien format) : ignoré, WARNING dans le log, VarMeta
  redéduits du Parquet. Le parquet fait toujours autorité pour les données.
- **Interruption entre publication du parquet et publication du sidecar** :
  les nouvelles données sont lisibles, sans métadonnées étendues — une perte
  d'affichage, jamais une fausse métadonnée.
- **Interruption avant publication du parquet** : la version précédente de la
  table est intacte et inchangée.
