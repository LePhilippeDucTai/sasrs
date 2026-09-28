# ADR 0004 — Adaptateurs sas7bdat / XPT : import/export aux frontières

Statut : proposé (J08-P4). Portée : futurs adaptateurs `PROC IMPORT`/`PROC
EXPORT` et `LIBNAME … XPORT`, feuille de route J08-P5. Unité d'analyse
uniquement — aucun code dans cette unité.

## Contexte

sasrs stocke nativement ses tables en Parquet + sidecar JSON (ADR 0001) ;
aucun chemin de code ne lit ni n'écrit aujourd'hui les formats binaires de
SAS. Or l'interopérabilité avec l'écosystème existant est une exigence
récurrente des utilisateurs :

- **`sas7bdat`** : format propriétaire binaire des bibliothèques SAS 9+
  (fichier par table, métadonnées embarquées : formats, informat, libellés,
  longueurs). Version 7/8/9 ; `sashdat` (Hadoop) hors périmètre.
- **`xport` (XPORT)** : format d'échange documenté publiquement par SAS
  (`SAS Technical Support document TS-140` pour la v5, extension v8 dans la
  version 8+). C'est le format de facto des dépôts de données (FDA/CDISC,
  données publiques). `LIBNAME … XPORT` côté SAS ne lit/écrit que la v5
  pour les bibliothèques déclarées `V5` ; la v8 est accessible en v9.

Sans ces adaptateurs, un utilisateur sasrs doit convertir ses données via
pandas/haven avant tout traitement — exactement le frottement que le projet
prétend supprimer. La question posée : **comment** lire/écrire ces formats,
et à quel point de l'architecture ?

## Options considérées

1. **Crates Rust pures (`sas7bdat`, parseur XPT maison)** —
   - `sas7bdat` (crate, Apache-2.0/MIT) lit le format 7/8/9 : décompression
     CHAR/BINARY, sous-ensembles de colonnes, mais couverture incomplète des
     cas réels (dates/heures exotiques, compressions rares, fichiers v9.4+
     avec encodages non déclarés).
   - XPT : format simple et documenté (TS-140), un parseur/écrivain maison
     v5 est de l'ordre de quelques centaines de lignes ; la v8 ajoute des
     noms longs et des libellés étendus.
   - Avantages : aucune dépendance C, intégration au workspace, licences
     permissives, pas de friction de cross-compilation (cf. roues J08-P5 /
     ADR 0003).
   - Inconvénients : risque de fidélité sur un corpus croissant de fichiers
     réels ; c'est le travail le plus long à fiabiliser.
2. **ReadStat via FFI (C)** — ReadStat (bibliothèque de Wizard/haven) lit et
   écrit sas7bdat, XPT v5/v8, SPSS, Stata, avec un excellent taux de réussite
   sur fichiers réels. MAIS : ReadStat est distribué sous **GPL-2.0+**. Une
   liaison dynamique ou statique rend l'ensemble dérivé — la distribution
   d'un binaire sasrs (CLI) ou de roues Python (ADR 0003) sous licence
   permissive deviendrait contrainte (obligation de publier les sources
   complets sous GPL au moment de la distribution). C'est jugé incompatible
   avec le modèle de distribution du projet (binaires et roues précompilés
   publics). Rejetée, mais documentée : si la politique de licence change,
   l'interface d'adaptateur ci-dessous permet de brancher ReadStat comme
   back-end optionnel (feature flag, non activé par défaut).
3. **Déléguer aux outils SAS (`PROC COPY`, SAS/CONNECT)** — rejetée :
   suppose une licence SAS installée, contredit la raison d'être de sasrs.
4. **Couche de conversion externe (pandas + pyreadstat côté Python)** —
   rejetée comme voie principale : déplace la complexité vers l'utilisateur,
   deux parcours d'import (natif vs Python) divergents ; reste néanmoins
   utilisable par les utilisateurs de la future liaison Python.

## Décision

Les adaptateurs sas7bdat/XPT sont placés **aux frontières** de sasrs —
exclusivement derrière `PROC IMPORT`, `PROC EXPORT` et le moteur
`LIBNAME … XPORT` — et **ne remplacent pas** le stockage interne
Parquet + sidecar (ADR 0001), qui reste le format canonique de
`DirLibrary::write`/`read`.

1. **Direction** : implémentation Rust pure (option 1) — crates existantes
   pour la lecture `sas7bdat` si leur fidélité est démontrée sur le corpus
   de tests, sinon parseur maison ; parseur/écrivain XPT **maison** (le
   format v5 est petit et documenté, la v8 suit). Aucune dépendance GPL
   (ReadStat) dans les binaires et roues distribués.
2. **Sémantique d'import** : `PROC IMPORT` lit sas7bdat/XPT → produit un
   `SasDataset` en mémoire, puis la persistance passe par le chemin Parquet +
   sidecar habituel. Les métadonnées portées par les fichiers SAS (format,
   libellé, longueur) sont mappées sur les `VarMeta` du sidecar — c'est
   exactement ce que le sidecar a été conçu pour porter ; rien n'est perdu à
   l'aller-retour sas7bdat → Parquet → sas7bdat pour les attributs couverts.
3. **Sémantique d'export** : `PROC EXPORT`/`LIBNAME … XPORT` sérialisent un
   `SasDataset` vers XPT v5 (défaut, cible d'interopérabilité maximale :
   FDA/CDISC, SAS 9, pandas) ; la v8 est une option explicite pour les noms
   longs et libellés > 40 octets, avec un WARNING loggé sur les attributs non
   représentables en v5.
4. **Encodages** : les fichiers sas7bdat déclarent leur encodage dans
   l'en-tête ; l'adaptateur lit cette déclaration, retombe sur UTF-8 avec
   WARNING en cas d'en-tête absent/invalide, et transcode vers UTF-8 en
   interne (stockage Parquet = UTF-8). XPT v5 est latin-1-ish par spécification
   (ASCII strict en pratique) : l'export encode en ASCII avec WARNING sur les
   caractères non représentables ; la v8 autorise UTF-8.
5. **Licences** : toute crate retenue doit être Apache-2.0/MIT/BSD ;
   l'inventaire des dépendances et leurs licences est vérifié en CI
   (`cargo deny` ou équivalent). ReadStat reste exclu (GPL-2.0+) sauf
   décision explicite de politique de licence.
6. **Plan de tests (prérequis de confiance, J08-P5)** :
   - corpus de fichiers sas7bdat/XPT réels, versionnés dans un dépôt de
     fixtures (ou téléchargés avec empreinte SHA-256 vérifiée) : v7/8/9,
     compressions CHAR/BINARY/NONE, encodages variés, libellés/formats
     exotiques, dates/heures, valeurs manquantes typées SAS (`._ABC`),
     noms courts et longs ;
   - tests d'aller-retour : sas7bdat → dataset → Parquet → dataset → XPT →
     dataset, avec comparaison exacte des données ET des `VarMeta` (formats,
     libellés, longueurs) ;
   - tests différentiels contre un oracle existant (pandas/pyreadstat ou
     haven) exécutés hors CI sur le corpus, pour chiffrer la fidélité avant
     de qualifier un chemin « fiable » ;
   - tests d'échec propre : fichier tronqué, encodage inconnu, format non
     supporté (v6/v9 exotic) → erreur typée, jamais de panic.

## Conséquences

- **Positives** : interopérabilité native avec l'écosystème SAS et les dépôts
  de données publics, sans dépendance GPL ni outil SAS ; les métadonnées
  sas7bdat trouvent un hôte naturel dans le sidecar (ADR 0001) ; la surface
  `PROC IMPORT`/`EXPORT` reste le seul point d'entrée, le cœur n'est pas
  pollué.
- **Négatives / coûts** : le sas7bdat est propriétaire et mal spécifié — la
  fidélité se conquiert fichier par fichier, c'est un effort long et
  incrémental ; maintenir un écrivain XPT v5/v8 est un périmètre de
  conformité supplémentaire ; l'ASCII-strict de la v5 impose des pertes
  documentées (WARNING) sur les données non latines.
- **Non-goals** : écrire du sas7bdat (lecture seule ; l'écriture passe par
  XPT et Parquet), lire `sashdat`, supporter SPSS/Stata (ReadStat-only, non
  retenu).

## Items feuille de route (J08-P5)

- [ ] Constituer le corpus de fixtures sas7bdat/XPT réels (empreintes
      SHA-256, provenances documentées) et l'oracle différentiel
      pandas/pyreadstat hors CI.
- [ ] Évaluer les crates de lecture sas7bdat existantes (`sas7bdat`,
      `parquet-`voisines, forks) sur le corpus ; décider lecture maison vs
      crate, avec tableau de fidélité mesuré.
- [ ] Implémenter le parseur/écrivain XPT v5 (TS-140) : en-têtes, membres,
      NAMESTR, observation records, valeurs manquantes typées.
- [ ] Étendre à XPT v8 (noms longs, libellés étendus, encodage UTF-8) et
      WARNING de non-représentabilité en v5.
- [ ] Brancher `PROC IMPORT` / `PROC EXPORT` / `LIBNAME … XPORT` sur ces
      adaptateurs, mapping sas7bdat → `VarMeta` du sidecar, tests
      d'aller-retour en CI.
- [ ] `cargo deny` (ou équivalent) : contrôle de licences des dépendances,
      garde-fou contre l'introduction d'une dépendance GPL dans les binaires
      et roues distribués.
