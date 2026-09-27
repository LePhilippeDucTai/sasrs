# ADR 0002 — Façade publique `sasrs::api`

- Statut : accepté (J06-P1)
- Date : 2026-02-14
- Décideurs : consolidation J06
- Liens : ADR 0001 (stockage parquet + sidecar), `docs/support-contract.md`

## Contexte

Le crate `sasrs` expose historiquement une seule entrée : la fonction libre
`run(source_text, RunOptions) -> RunOutcome` (`src/lib.rs`). Cette fonction
couvre le cas « un programme, un processus » du binaire, mais pas celui d'un
consommateur embarqué (harnais de test, service, bibliothèque) qui veut :

- soumettre **plusieurs programmes successifs** sur une **même session** (WORK
  et table de symboles macro partagées entre soumissions) ;
- **injecter** des données hostes (un `DataFrame` Polars) dans une bibliothèque
  SAS, puis **relire** le résultat avec ses métadonnées (label, format,
  longueur) ;
- **fermer** explicitement la session (finalisation des destinations ODS,
  suppression de WORK) et obtenir un rapport final.

Par ailleurs, les types internes (`session::Session`, `LibraryManager`,
`LogWriter`, …) sont aujourd'hui `pub` par nécessité de collaboration entre
modules du crate, sans contrat de stabilité : tout consommateur externe qui
les toucherait dépendrait de détails d'implémentation qui bougent à chaque
jalon.

## Décision

1. **Surface stable `sasrs::api`.** Le module `sasrs::api` est la seule
   surface publique supportée du crate. Il fournit une façade complète et
   autonome :

   - `api::Session` — session SAS vivante, réutilisable :
     - `Session::new(Options) -> Result<Session, ApiError>` ;
     - `submit(&mut self, code: &str) -> Submission` — soumet un programme
       SAS complet (log, compteurs d'erreurs/warnings, code retour), sans
       consommer la session : WORK, librefs, catalogue de formats et moteur
       macro persistent d'une soumission à l'autre ;
     - `register_dataset(libref, name, DataFrame, métadonnées optionnelles)`
       — injecte un `DataFrame` Polars coercé vers le modèle de types SAS
       (numérique f64 / caractère string) et écrit la table
       `<name>.parquet` + sidecar de métadonnées (ADR 0001) dans le libref ;
     - `dataset(libref, name) -> (DataFrame, Vec<VarMeta>)` — relit une table
       avec ses métadonnées ;
     - `close(self) -> CloseReport` — finalise les destinations ODS ouvertes,
       draine log/listing, calcule le code retour global, puis détruit la
       session (le répertoire WORK temporaire est supprimé au drop).
   - `api::Options` — options de création (WORK, base de résolution des
     chemins relatifs, déterminisme, fast-path vectorisé).
   - `api::Submission`, `api::CloseReport` — résultats typés (log, listing,
     compteurs, code retour).
   - `api::ApiError` — erreurs typées de la façade (initialisation, libref
     inconnu, table inconnue, échec d'enregistrement/lecture, métadonnées
     incohérentes avec les colonnes).
   - Ré-exports de commodité : `api::VarMeta`, `api::VarType`.

2. **Compatibilité : semver sur `sasrs::api` uniquement.** Toute casse
   (signature, sémantique, valeur de code retour) sur la façade exige un
   bump majeur de la version du crate. Le reste du crate (`sasrs::session`,
   `sasrs::executor`, `sasrs::procs`, …) est **interne** : il peut changer à
   tout moment sans préavis.

3. **Internes : `#[doc(hidden)]` plutôt que cassés.** Les modules internes
   restent `pub` (nécessaire à la compilation des modules frères et aux tests
   d'intégration du crate) mais ne font PAS partie du contrat. Lorsqu'un
   interne doit rester accessible pour des raisons techniques, il est marqué
   `#[doc(hidden)]` afin de l'exclure de la documentation publique et de
   signaler son statut — on ne le supprime/casse pas au seul motif qu'il est
   interne ; la casse éventuelle suit le cycle de version normale.

4. **`run()` conservé, réimplémenté sur la façade.** La fonction libre
   `sasrs::run(source_text, RunOptions) -> RunOutcome` est conservée à
   l'identique (mêmes champs, mêmes codes retour, même log/listing) : elle
   crée une `api::Session`, soumet une fois, ferme, et projette le
   `CloseReport` sur `RunOutcome`. Un seul chemin d'exécution sert ainsi le
   binaire et la bibliothèque.

## Types exposés

| Type | Rôle |
|---|---|
| `Options` | Création de session : `work_dir`, `base_dir`, `deterministic`, `vectorize` |
| `Session` | Façade vivante : `new`, `submit`, `register_dataset`, `dataset`, `close` |
| `Submission` | Résultat d'une soumission : `log`, `listing`, `exit_code`, `errors`, `warnings` |
| `CloseReport` | Résultat de fermeture : `log`, `listing`, `exit_code`, `errors`, `warnings` |
| `ApiError` | Erreurs typées (`Init`, `UnknownLibrary`, `UnknownDataset`, `Register`, `Read`, `MetadataMismatch`) |
| `VarMeta` / `VarType` | Métadonnées de variables (nom, type, longueur, format, label) |

## Conséquences

- `RunOutcome`/`run()` restent la compatibilité minimale pour le binaire et
  les tests existants ; aucune sortie ne change.
- Le code retour suit la convention existante : sans demande explicite
  (`%ABORT RETURN`, etc.), 0 = propre, 1 = warnings, 2 = erreurs ; une
  demande explicite prime, avec plancher 1 si des erreurs ont été comptées.
- Chaque `submit` rend le log produit par cette soumission (numérotation du
  source repartant à 1) et le listing finalisé à ce stade ; `close` rend le
  reliquat (ex. NOTEs de finalisation ODS) et le code retour global.
- Les consommateurs externes ne référencent que `sasrs::api` (+ `polars`) ;
  toute dépendance à un autre module est non supportée.
