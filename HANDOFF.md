# Passage de relais — jalon M41 (2026-08-19)

Note de reprise pour l'agent suivant, écrite après une session interrompue.
Le curseur reste `PROGRESS.md` ; ce fichier ne dit que ce qui n'est PAS
déductible du dépôt. **À supprimer une fois M41 terminé.**

## Où on en est

Jalon courant **M41 — Macro : quoting complet + %SYSCALL/%SYSMACDELETE**
(modèle **Fable**, cf. tableau Phase G de `PLAN.md`, qui fait foi).
Aucun commit n'a été produit : `main` est exactement dans l'état vérifié vert
du 2026-08-11 (2 681 tests, 0 `.snap.new`, clippy et fmt verts).

## À savoir avant de commencer (ce qui a été établi et qui n'est pas dans le plan)

1. **M41.1 et M41.2 sont déjà implémentés depuis M12.2.** `%BQUOTE`,
   `%NRBQUOTE` et `%SUPERQ` existent (`src/macros/expand/quoting.rs`, câblés
   dans la table `DISPATCH` de `src/macros/expand.rs`, testés dans
   `src/macros/tests/sysfunc.rs`). Ces deux cases ne sont donc PAS une
   implémentation mais un **audit de fidélité SAS + comblement des trous**.
2. **L'oracle de M41.1 dans `PROGRESS.md` est faux** — il dit que
   `%bquote(a&b)` « masque `&` sans résoudre ». En SAS 9.4, `%BQUOTE` **résout
   d'abord**, puis masque tout SAUF `&`/`%` ; c'est `%NRBQUOTE` qui masque en
   plus les déclencheurs. Le code actuel est conforme : ne pas le « corriger »
   vers cet oracle. Corriger la ligne de `PROGRESS.md` au passage.
3. **Le vrai trou de M41.1** est ailleurs : `%QUOTE`/`%NRQUOTE`, la paire de
   quoting d'exécution « historique » de SAS, est absente de `DISPATCH`. Même
   contrat que `%BQUOTE`/`%NRBQUOTE`, mais les quotes et parenthèses non
   appariées doivent être échappées par un `%` (`%'`, `%"`, `%(`, `%)`, `%%`).
4. **`README.md` ligne « Quoting » est périmé** : il affirme encore
   « no `%SUPERQ`/`%BQUOTE`/`%NRBQUOTE` » alors que les trois existent. À
   corriger dans le commit qui clôt M41.1.
5. **M41.3 est la seule case réellement non implémentée** : `%SYSCALL` et
   `%SYSMACDELETE` sont aujourd'hui consommés avec une NOTE « not supported in
   this build » (table `KW` de `src/macros/control.rs`). Piste : `%SYSCALL`
   suit le modèle de `%SYSFUNC` (`eval_sysfunc` dans
   `src/macros/expand/sysfunc.rs`) — `EvalCtx::default()` +
   `datastep::functions::call`, mais avec des arguments qui sont des **noms de
   variables macro** (sans `&`), lus puis réécrits dans la table des symboles.

## Une tentative de M41.1 existe sur `wip/m41.1` — NON VALIDÉE

La branche `wip/m41.1` (commit `2598e3f`, 6 fichiers, ~305 lignes) porte une
tentative interrompue de la case M41.1 : `%QUOTE`/`%NRQUOTE` complets avec
échappements `%'` `%"` `%(` `%)` `%%`, `^`/`¬`/`#` ajoutés à `STR_MASKED`,
expansion complète des arguments d'invocation portant un `%`, `unmask` de
l'écho MLOGIC, et ses tests unitaires.

**Clippy était vert ; `cargo test -p sasrs` n'a jamais tourné jusqu'au bout.**
L'invariant « zéro `.snap.new` » n'est donc PAS vérifié, et deux des
modifications sont précisément de celles qui font bouger des snapshots (voir
« Pièges » ci-dessous). À traiter comme une ébauche à valider, pas comme un
acquis : la reprendre coûte une suite de tests complète et une revue serrée.
La jeter et refaire M41.1 à neuf en s'appuyant sur les points 2 à 4 ci-dessus
est une option légitime.

## Prochaine étape concrète

Reprendre **M41.1**, soit depuis `wip/m41.1` (valider puis revoir), soit à neuf
depuis `main`, en tenant compte des points 2 à 4 ci-dessus ; puis enchaîner
M41.2, M41.3 et la DoD (fixtures `tests/fixtures/m41/`) selon la boucle de la
skill `sasrs-impl`.

## Pièges de validation propres à ce jalon

- Jalon de **complétion** : le comportement neuf ne doit s'activer que sur des
  constructions jusque-là non supportées → **zéro `.snap.new`** après
  `cargo test -p sasrs`. Si un snapshot bouge, corriger la cause, pas le
  snapshot.
- Deux endroits font bouger des snapshots existants si on y touche sans
  précaution, et méritent une revue serrée : la lecture des arguments
  d'invocation de macro (`src/macros/define/invoke.rs` — tout élargissement de
  l'expansion des arguments touche potentiellement toutes les fixtures macro)
  et l'écho `MLOGIC` des paramètres (une valeur quotée porte des sentinelles
  internes qui ne doivent jamais fuir dans le log).
- Élargir `STR_MASKED` (`src/macros/quoting.rs`) change `%str`/`%nrstr` pour
  TOUT le dépôt : à ne faire qu'avec la suite snapshot complète en vert.

## Environnement

Sur le Raspberry Pi d'origine, `cargo` n'est pas dans le `PATH` par défaut :
`export PATH="$HOME/.cargo/bin:$PATH"`. Une compilation complète y coûte
~15 min, et `test` + `clippy --all-targets` sont deux profils distincts, donc
deux compilations. Prévoir des timeouts généreux.
