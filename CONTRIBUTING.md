# Contribuer à `sasrs`

Ce dépôt est développé par des agents IA (Claude, GPT, GLM) sous supervision humaine.
Les règles ci-dessous transcrivent les « Exigences spécifiques pour des développements
100 % agents IA » de l'[issue #11](https://github.com/LePhilippeDucTai/sasrs/issues/11) ;
elles s'appliquent à toute contribution, agent ou humain.

La feuille de route active est le plan de consolidation dans `docs/plans/consolidation/`
(jalons J01–J08, contrats par part dans `docs/plans/consolidation/jalons/`). L'ancienne
roadmap M1–M66 de `PLAN.md`/`PROGRESS.md` racine est gelée ; son remapping est décrit
dans `PLAN.md` § « Correspondance M46–M66 → consolidation ».

## 1. La CI arbitre, jamais la parole d'un agent

- **Aucune PR ne repose sur la simple affirmation d'un agent que « les tests sont
  verts ».** L'arbitre est la CI, reproductible : le check agrégé `ci-ok`
  (`.github/workflows/ci.yml`) doit être vert sur le dernier commit de la PR.
- La CI tourne avec `CI=true` et `INSTA_UPDATE=no` : un écart de snapshot échoue le
  build, aucun `.snap.new` n'est écrit ni accepté (`git status --porcelain` doit rester
  vide après les jobs de test).
- En local, `scripts/check.sh` exécute exactement les mêmes commandes (voir §7) ; un
  rapport d'agent (« tests verts », « conforme à SAS ») est une hypothèse à vérifier,
  jamais une preuve.

## 2. L'implémenteur n'écrit pas le seul oracle

- **L'agent qui écrit une implémentation ne crée pas lui-même le seul oracle qui la
  valide.** Les valeurs attendues proviennent d'une source indépendante de la part qui
  implémente :
  - documentation SAS 9.4 publiée, URL citée dans la PR/le cas de conformité ;
  - sortie SAS 9.4 réelle (log/listing/dataset) ;
  - oracle écrit par une autre part (corpus de conformité `conformance/cases/`,
    script Python indépendant).
- Un cas du corpus marqué **`known-divergence`** (divergence connue et documentée entre
  `sasrs` et SAS) ne voit **jamais ses valeurs attendues modifiées par l'implémenteur** :
  seul son statut peut changer, avec justification. Le statut `known-divergence` se
  décide contre une référence externe, pas contre la sortie courante du code.
- Les tâches numériques difficiles sont découpées **par comportement précis** (un
  comportement = un reproducer, un oracle, un critère d'acceptation), jamais en items
  vagues du type « terminer PROC MIXED ».

## 3. Reproducer et test de non-régression

Tout **changement de sémantique** (sortie, listing, log, diagnostic, code retour,
dataset produit, métadonnées) est livré avec :

1. un **reproducer** minimal — programme SAS ou test qui expose l'ancien comportement
   (bug, silence, divergence) avant le changement ;
2. un **test de non-régression** qui verrouille le nouveau comportement attendu.

Un correctif sans reproducer, ou un comportement nouveau sans test qui le fige, n'est
pas terminé.

## 4. Snapshots : stabilité ≠ conformité

- **Un snapshot n'est jamais une preuve de conformité à SAS à lui seul** : il verrouille
  la stabilité (auto-cohérence d'une exécution à l'autre), pas la fidélité. La fidélité
  vient d'un oracle externe (§2).
- Les changements de snapshot sont autorisés lorsqu'ils corrigent une erreur, mais
  **chaque snapshot modifié doit être justifié dans le message de commit** par une
  ligne :

  ```
  Snapshot: <fixture> — <raison>
  ```

  Une ligne par fixture touchée (chemin du `.snap` ou nom de la fixture). Un commit qui
  modifie un `.snap` sans cette justification est refusé en revue/CI.

## 5. Contrat « reconnu mais ignoré »

- Toute fonctionnalité (instruction, statement, option) **reconnue** par le parseur mais
  **non honorée** doit produire un diagnostic explicite — jamais un repli silencieux :
  - **ERROR** si l'ignorer peut modifier le résultat (valeur numérique, dataset
    produit, contrôle de flux) ;
  - **WARNING** si l'effet est limité à l'affichage.
- Une instruction inconnue produit une ERROR façon SAS (`180-322`), pas un crash ni un
  ignore. Le détail du contrat (règles par cas, catalogue des diagnostics) vit dans
  `docs/support-contract.md`.

## 6. Les quatre états de couverture

La couverture publique (`README.md`) ne doit jamais promettre plus que ce qui est
réellement implémenté et validé. Quatre états, mutuellement exclusifs — c'est le
vocabulaire commun du README, du corpus de conformité (`conformance/STATUS.md`) et des
revues :

| État | Signification | Marque README |
|---|---|---|
| **implémenté** | Comportement codé et couvert par les tests du dépôt (unitaires, intégration, snapshots). La conformité SAS est plausible (vérification manuelle éventuelle) mais pas attestée par un oracle externe formalisé. | ✅ |
| **validé contre référence** | Implémenté **et** validé contre une référence externe citée (doc SAS 9.4 publiée, sortie SAS réelle, oracle indépendant) : cas du corpus de conformité ou équivalent, avec tolérances et provenance. Seul cet état autorise l'affirmation « conforme à SAS ». | ✅ (référence citée) |
| **approximation documentée** | Un comportement proche mais volontairement divergent est livré ; la nature de l'écart (conditions, effet, direction) est documentée publiquement. Cas `known-divergence` du corpus : écart figé, valeurs attendues intouchables par l'implémenteur. | 🟡 |
| **non supporté** | Fonctionnalité reconnue mais non implémentée : l'utiliser produit un diagnostic explicite (ERROR/WARNING, cf. §5), jamais un silence. | 🔴 |

Le passage d'un état à l'autre se fait dans la part qui livre le changement, avec la
preuve correspondante (test pour *implémenté*, référence citée pour *validé contre
référence*, documentation de l'écart pour *approximation documentée*, diagnostic pour
*non supporté*).

## 7. Vérifier avant de pousser : `scripts/check.sh`

`scripts/check.sh` exécute en local les mêmes commandes que la CI, dans le même
environnement (dont `CI=true`, `INSTA_UPDATE=no`), et s'arrête au premier échec
(code retour non nul) :

```sh
scripts/check.sh lint   # cargo fmt --check && cargo clippy --all-targets -- -D warnings
scripts/check.sh test   # cargo test -p sasrs (+ jobs de test features graphics / s3 de la CI)
scripts/check.sh build  # cargo build --features graphics && cargo build --features s3
scripts/check.sh all    # lint + test + build
```

`scripts/ci-status.sh [branche]` (défaut `consolidation`) vérifie que le dernier run CI
poussé sur la branche est vert et porte bien le HEAD attendu — c'est lui qui fait foi
pour l'état de la CI, pas le souvenir d'un agent.

## 8. Workflow Git

- Une part validée = un commit de merge `<id>: <titre>` sur la branche de consolidation ;
  les workers commitent dans leur worktree.
- Jamais de `git push --force`, jamais de `git commit --amend` sur ce qui est poussé ;
  un commit de snapshot modifié porte ses lignes `Snapshot:` (§4).
- La PR décrit reproducer, tests, oracle et provenance, snapshots modifiés et état de la
  CI : voir `.github/pull_request_template.md`.

## 9. Main protégé

La branche `main` est protégée : toute fusion `consolidation` → `main` passe par une
PR avec le check `ci-ok` vert (§1). Les vérifications locales se font via
`scripts/check.sh` (§7) avant d'ouvrir la PR.
