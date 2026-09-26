# Contrat d'encodage (D-001)

Ce document fixe le comportement d'encodage de `sasrs`. Il complète le
support-contract : en cas de divergence, le présent contrat prime pour tout ce
qui touche aux encodages.

## Modèle : session SAS LATIN1/WLATIN1

`sasrs` se comporte comme une session SAS 9.4 classique configurée en
`ENCODING=LATIN1` (ou `WLATIN1` sous Windows) **mais avec Unicode comme
représentation interne** :

- les **longueurs** de variables caractère (`LENGTH`, `VarMeta::length`,
  longueurs dérivées des littéraux) comptent en **caractères**, pas en octets ;
- les **troncatures** (PDV, PROC APPEND, affectations avec `LENGTH $n`) coupent
  en **caractères** : `LENGTH('é')` vaut `1`, `SUBSTR` travaille sur des
  frontières de caractères ;
- une valeur n'est **jamais** une séquence UTF-8 invalide : la représentation
  interne est un `String` Rust (UTF-8 valide), la sortie est toujours de
  l'UTF-8 valide.

### Écart avec une session SAS UTF-8

Dans une session SAS configurée avec `ENCODING=UTF-8`, `LENGTH` compte en
**octets** UTF-8 (`LENGTH('é')` vaut `2`) et la troncature coupe en octets
(sans garantie de frontière de caractère). `sasrs` diverge volontairement de ce
comportement et suit la sémantique « caractères » des sessions LATIN1/WLATIN1.
L'écart est documenté ici et assumé : il correspond au cas d'usage dominant
(données européennes occidentales, programmes portés de sessions Windows
WLATIN1).

## Entrées

- Fichiers sources `.sas`, fichiers `%INCLUDE` et fichiers `INFILE` :
  **UTF-8 strict**. Un fichier qui n'est pas valide UTF-8 produit une `ERROR`
  qui **nomme le fichier** — aucun repli lossy, aucun commentaire silencieux.
- Un **BOM UTF-8** (`EF BB BF`) en tête de ces fichiers est **ignoré**
  (retiré avant lexing / expansion / découpage en lignes), comme le fait SAS.
- Dans le source, les **littéraux chaîne** (`'…'` / `"…"`) sont décodés en
  **UTF-8** : `'café'` produit la chaîne `café` (4 caractères), pas une
  séquence d'octets réinterprétée.
- Les **identifiants** (noms de variables, macros, datasets) restent
  **ASCII** (`[A-Za-z_][A-Za-z0-9_]*`). Un caractère non ASCII hors littéral
  ou commentaire est une erreur de parsing.

## Sorties

- Log, listing et destinations HTML/Excel : **UTF-8**.
- **RTF** : les caractères non ASCII sont échappés en séquences RTF
  `\uN?` (codepoint Unicode décimal, repli `?`).
- **PDF** : la sortie texte embarquée est ASCII ; les caractères non ASCII
  sont remplacés par `?`. Cette limite est **documentée et assumée** (les
  polices de base Helvetica/WinAnsi ne couvrent pas l'Unicode étendu sans
  embedding, hors périmètre).

## Règles de test

- Tout correctif d'encodage s'accompagne d'un reproducer et d'un test de
  non-régression nommé `utf8*` (voir CONTRIBUTING.md §3) : l'implémenteur
  n'écrit pas le seul oracle.
- Les snapshots ne doivent contenir **aucun mojibake** (séquence
  `C3 A2 C2 80 …` caractéristique d'un décodage octet-par-octet d'un tiret
  cadratin ou d'un accent) — vérifié par le check `j03-no-mojibake`.
- Un commit qui modifie un `.snap` porte une ligne `Snapshot: <fixture> —
  <raison>` par snapshot modifié (CONTRIBUTING.md §4).
