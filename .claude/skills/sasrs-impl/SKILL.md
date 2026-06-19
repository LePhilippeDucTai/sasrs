---
name: sasrs-impl
description: Avance le projet sas_interpreter (interpréteur SAS en Rust/Polars) jusqu'à la fin du jalon courant — itère sur TOUTES les cases non cochées du jalon, une par une (ou un groupe ⫽ à la fois), committe ET pousse après chaque validation avant de passer à la suivante. S'arrête en fin de jalon ou si les limites de contexte approchent.
---

# sasrs-impl — exécuter le prochain incrément du jalon courant

Tu es l'ORCHESTRATEUR du projet `sas_interpreter` (crate du workspace, branche de
développement : la branche courante du repo, normalement
`claude/sas-rust-interpreter-hlzxlc` — ne JAMAIS pousser ailleurs).

## Sources de vérité (à lire dans cet ordre, toujours)

1. `sas_interpreter/PROGRESS.md` — le curseur : jalon courant + cases cochées.
2. `sas_interpreter/PLAN.md` — architecture, décisions actées, table modèle/effort,
   checklist des pièges (§Checklist — relire avant toute revue).
3. L'en-tête de chaque fichier squelette à implémenter — il contient SON plan détaillé
   (sémantique SAS, algorithmes, pièges, tests à écrire). C'est le cahier des charges.

## Procédure d'une invocation

1. **État des lieux** : `git status` (l'arbre doit être propre — sinon examiner, terminer
   ou committer l'en-cours avant tout), `git pull origin <branche>`, puis
   `cargo test -p sas_interpreter` pour vérifier que la base est verte. Base rouge =
   la réparer D'ABORD (c'est l'incrément du jour).
2. **Sélection** : identifier la PROCHAINE case non cochée du jalon courant dans
   PROGRESS.md, dans l'ordre du fichier (il encode les dépendances). Si plusieurs cases
   consécutives sont marquées ⫽ (fichiers indépendants), les regrouper en un seul lot
   parallèle ; sinon prendre un seul fichier à la fois. L'objectif de l'invocation est
   de couvrir TOUTES les cases du jalon courant — ne pas s'arrêter après un seul fichier
   tant que des cases restent et que le contexte le permet (voir garde-fous).
3. **Implémentation** : pour chaque fichier du lot, déléguer à un sous-agent via le tool
   Agent avec le paramètre `model` suggéré par PROGRESS.md/PLAN.md (`sonnet`, `opus`,
   `fable` — les fichiers marqués Fable peuvent aussi être faits directement par
   l'orchestrateur). Donner au sous-agent : le chemin du fichier, l'instruction de lire
   son en-tête + PLAN.md §Checklist, d'implémenter TOUT le fichier (zéro `todo!()`
   restant) ET ses tests unitaires, et de faire passer
   `cargo test -p sas_interpreter`. Les fichiers indépendants (⫽) peuvent être délégués
   en parallèle.
4. **Validation orchestrateur** (obligatoire avant commit — c'est le contrat) :
   - relire le diff de chaque fichier livré : conformité au plan d'en-tête, respect de
     la checklist des pièges (sas_cmp partout, nullify_specials, pas de get_row,
     troncature char, NOTEs au pluriel invariable...) ;
   - `cargo test -p sas_interpreter` complet, zéro warning nouveau ;
   - rejeter/faire corriger ce qui ne passe pas la revue.
5. **Commit + push IMMÉDIATEMENT après validation** (protection contre la perte de
   session et mise à jour du PR GitHub) : cocher les cases dans PROGRESS.md (+ passer
   les fichiers à ✅ dans la table de PLAN.md quand un fichier est terminé), inclure
   PROGRESS.md/PLAN.md dans le MÊME commit que le code, message clair (`sasrs M1:
   implement parser/expr (Pratt SAS precedence)`), puis `git push -u origin <branche>`
   (échec réseau : réessayer 4 fois, backoff 2/4/8/16 s). Un commit par fichier validé
   ou par groupe ⫽ cohérent — jamais de gros commit fourre-tout, jamais de code non
   validé. **Ne jamais commencer le fichier ou groupe suivant sans avoir committé ET
   poussé le précédent.**
5b. **Boucle interne — cases restantes du jalon** : après chaque commit+push réussi,
    retourner à l'étape 2 et sélectionner la prochaine case non cochée du MÊME jalon.
    Répéter les étapes 2→3→4→5→5b jusqu'à l'une des conditions d'arrêt suivantes :
    - toutes les cases du jalon courant sont cochées → passer à l'étape 6 ;
    - les limites de contexte ou d'utilisation approchent → terminer proprement (étape 5
      pour l'en-cours) et rapporter à l'étape 7 ;
    - un blocage nécessite une décision utilisateur → rapporter à l'étape 7.
    Ne jamais rompre la boucle silencieusement : toute sortie anticipée DOIT apparaître
    dans le rapport de fin d'invocation (étape 7).
6. **Fin de jalon** : quand toutes les cases du jalon sont cochées, dérouler sa ligne
   "DoD"/fixtures (snapshots insta : générer, VÉRIFIER À LA MAIN la plausibilité SAS de
   chaque snapshot avant `cargo insta accept`, committer les .snap), mettre à jour
   "Jalon courant : **Mn+1**" en tête de PROGRESS.md, committer, pousser.
7. **Rapport de fin d'invocation** : 2–5 lignes — ce qui a été livré/committé (hashes),
   où en est le jalon, ce que la PROCHAINE invocation prendra. Si un blocage nécessite
   une décision utilisateur, le dire explicitement.

## Garde-fous

- Périmètre : uniquement `sas_interpreter/` (+ `Cargo.lock`). Ne pas toucher aux autres
  crates du workspace.
- Ne jamais cocher une case pour du code contenant encore `todo!()`/`unimplemented!()`.
- Ne pas rediscuter les décisions actées de PLAN.md (types SAS stricts, parser SQL
  dédié, etc.).
- Snapshots insta : un snapshot n'est PAS un oracle — le relire et le confronter au
  comportement SAS documenté avant de l'accepter.
- Si les limites d'utilisation approchent ou que le contexte devient long : finir le
  fichier en cours, valider, committer, pousser, rapporter. Le travail committé est le
  seul qui compte.
