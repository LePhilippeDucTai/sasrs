# DECISIONS — consolidation

Generated from Mission Control; do not edit.

## 77ddb1e9-0f86-4599-a78c-c7d51e79db3e · resolved
Quel niveau de parallélisme d'exécuteurs dsh pour la consolidation ?
Choice: p2

## 2ceeafdb-53ac-4722-b006-24d532b8caf1 · resolved
Politique disque /home après chaque merge ?
Choice: cleanup

## 5fac0b48-dfcc-4d50-a916-5ca098af56a9 · resolved
Escalade en cas de blocage durable d'une part ?
Choice: codex-xhigh

## 806b36f3-12b9-4cf9-9e9f-54179e3dcdcf · resolved
Comment fournir à J02-P2 le raccordement nécessaire entre %ABORT et RunOutcome.exit_code sans modifier un fichier hors périmètre ?
Choice: prerequisite

## f95f45b8-5174-4fa4-aca5-04176b45c9a2 · resolved
Comment fournir à J02-P2 le raccordement %ABORT n -> code retour sans fichier hors périmètre (src/lib.rs appartient à J02-P1) ?
Choice: prerequisite

## 2d353886-6ada-4fe4-8f42-1b1112276999 · resolved
Comment livrer le canal de code retour explicite absent de la base, alors que src/lib.rs et src/session.rs sont interdits à J02-P2 ?
Choice: prerequisite

## 9881c531-ef05-490c-8903-84c8ccee68bc · resolved
Comment livrer le canal de code retour explicite absent de la base (src/lib.rs calcule RunOutcome.exit_code uniquement en 0/1/2, J02-P1 n'a rien livré), alors que src/lib.rs et src/session.rs sont interdits à J02-P2 ?
Choice: prerequisite

## 46fe2f07-84fe-4c23-b327-07fb7e8c9d3e · resolved
Comment retablir un environnement Cargo autorise pour poursuivre J02-P2 apres la panne du montage overlay de ombre-mingw ?
Choice: repair_container

## 652ab873-0e8a-461a-b880-f268df6b44f0 · resolved
Comment réviser le périmètre de J02-P3 pour autoriser le raccordement des helpers aux procédures et le transport des WARNING/instructions globales avant une nouvelle tentative ?
Choice: extend_scope

## cde127f4-2636-4ee3-9da0-ea7dc628e564 · resolved
Faut-il autoriser la mise à jour du test TRANSPOSE qui impose actuellement l’acceptation silencieuse d’une instruction inconnue ?
Choice: extend_test_scope

## fd7fcfa7-796e-4d49-bc2a-df8497673592 · resolved
Comment rendre le périmètre compatible avec l’exigence UNIVARIATE NOPRINT ?
Choice: extend_univariate_scope

## c8371f5a-d1dc-4e26-a9e5-c6a2923e1043 · resolved
Le quota du compte Codex est épuisé jusqu'au 27/09 16:56 : comment poursuivre les exécuteurs du plan consolidation ?
Choice: switch_claude

## 55a73852-c7f1-4dcf-9ed9-5c078b0f4b75 · resolved
Règle du 19/09 (politique permanente) : quota Codex épuisé = relance immédiate sous dsh sans attendre le reset. Quels exécuteurs pour la suite du plan consolidation ?
Choice: switch_dsh

## aee41512-79ba-4b73-b85b-123b3e4a9a1e · resolved
CI main rouge depuis le merge J02-P4 : ods_image_write_failure_is_error (tests/cli.rs:439) échoue sous --features graphics — NOPRINT supprime le rendu des images ODS Graphics d'UNIVARIATE. Comment corriger ?
Choice: corrective_unit_j02_p11

## 108c6385-39f1-4fce-928c-0f0fc73e1b51 · resolved
J03-P6 bloqué : UPDATEMODE=NOMISSINGCHECK exige de toucher src/parser et src/ast (parse_dataset_ref, DatasetOptions, DsStmt::Update), hors préfixes autorisés. Comment débloquer ?
Choice: extend_parser_scope

## f6aa1e45-f2a8-4dbf-98d1-17fd70a64c3b · resolved
J03-P6 bloqué : UPDATEMODE=NOMISSINGCHECK exige de toucher src/parser et src/ast (parse_dataset_ref, DatasetOptions, DsStmt::Update), hors préfixes autorisés. Comment débloquer ?
Choice: extend_parser_scope

## 0a017d22-92a4-471f-8826-7f83124d6151 · resolved
J03-P2 bloqué : weighted_quantile_def5 doit vivre dans src/procs/common/stats.rs et être utilisé par MEANS/SUMMARY, mais src/procs/common/mod.rs (hors périmètre) déclare « mod stats; » en privé — l'export « pub use stats::weighted_quantile_def5; » est indispensable (E0603 reproduit puis reverté). Comment débloquer ?
Choice: extend_common_mod

## a8788660-2f2d-41db-a685-ef93804bd758 · resolved
J03-P2 bloqué : weighted_quantile_def5 doit vivre dans src/procs/common/stats.rs et être utilisé par MEANS/SUMMARY, mais src/procs/common/mod.rs (hors périmètre) déclare « mod stats; » en privé — l'export « pub use stats::weighted_quantile_def5; » est indispensable (E0603 reproduit puis reverté). Comment débloquer ?
Choice: extend_common_mod

