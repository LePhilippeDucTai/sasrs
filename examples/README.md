# Exemples sasrs (J06-P3)

Exemples autonomes testés (`tests/examples.rs` les exécute) :

- `data/patients.csv` — petit jeu de données d'entrée (5 patients).
- `cli/analysis.sas` — analyse CLI : `PROC IMPORT` du CSV, agrégation
  `PROC SQL` par sexe, `PROC EXPORT` du résumé vers `examples/out/summary.csv`,
  puis `PROC PRINT`. Les chemins relatifs résolvent sous le répertoire du
  fichier `.sas`. Sortie attendue documentée dans l'en-tête du fichier et
  vérifiée par `tests/examples.rs`.
- `quickstart.rs` — façade `sasrs::api` : créer une session, soumettre un
  programme (import + agrégation), relire la table produite
  (`Session::dataset`), afficher un diagnostic structuré du log, fermer la
  session. La dernière ligne imprimée est `OK`.

Commandes :

```sh
cargo run --locked --bin sasrs -- examples/cli/analysis.sas
cargo run --locked --example quickstart
```
