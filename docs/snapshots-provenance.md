# Registre de provenance des snapshots

> **Mesure exceptionnelle.** Ce registre documente rétroactivement les snapshots
> (`tests/snapshots/*.snap`) dont la ligne de justification canonique
> `Snapshot: <fixture> — <raison>` (CONTRIBUTING.md §4) manque au message du commit
> d'origine. L'historique déjà poussé n'est ni réécrit ni amendé : la justification
> est consignée ici à la place.
>
> **À partir de maintenant, tout nouveau commit modifiant un `.snap` DOIT porter sa
> ligne `Snapshot: <fixture> — <raison>` dans son message de commit** (une ligne par
> fixture touchée), conformément à CONTRIBUTING.md §4. Ce registre n'est pas une
> tolérance pour les commits futurs.

## snapshot__fixtures@j01__update_by_only.sas.snap

- Snapshot : `tests/snapshots/snapshot__fixtures@j01__update_by_only.sas.snap`
- Raison : non-régression de `UPDATE` sans `KEY=` accepté avec seulement une
  instruction `BY` (issue #13) — fige le listing produit par le fixture
  `tests/fixtures/j01/update_by_only.sas`.
- Commit d'origine : `b9924dd1d2e2fad83854ce308bc59b44b3dd4e6c`
  (« fix(datastep): UPDATE sans KEY= accepte avec seulement BY (issue #13) ») — le
  snapshot a été ajouté sans la ligne `Snapshot:` dans le message de commit.
- Revue : J01-P4 (attempt `75e50b0d-ef04-4810-bc7a-b9e28c585dbb`) — finding bloquant
  signalant l'absence de justification.
- Décision : `680238b2` — correction par le présent registre rétroactif plutôt que
  par réécriture/amendement de l'historique poussé.

Ligne canonique rétroactive :

```
Snapshot: tests/snapshots/snapshot__fixtures@j01__update_by_only.sas.snap — non-régression UPDATE sans KEY= avec seulement BY (issue #13)
```

## snapshot__fixtures@j08__by_corr.sas.snap

- Snapshot : `tests/snapshots/snapshot__fixtures@j08__by_corr.sas.snap`
- Raison : non-régression du layout `OUTP=`/`OUTS=` de PROC CORR conforme à la doc
  CORR (issue #18) — fige la sortie insta du fixture
  `tests/fixtures/j08/by_corr.sas`.
- Commit d'origine : `79b3c31`
  (« corr: layout OUTP=/OUTS= conforme à la doc CORR (issue #18) ») — le snapshot a
  été modifié sans la ligne `Snapshot:` dans le message de commit.
- Revue : J02-P5 (attempt `7b4deaa6`) — finding bloquant signalant l'absence de
  justification.
- Décision : `3865498b` — correction par le présent registre rétroactif plutôt que
  par réécriture/amendement de l'historique poussé.

Ligne canonique rétroactive :

```
Snapshot: tests/snapshots/snapshot__fixtures@j08__by_corr.sas.snap — non-régression layout OUTP=/OUTS= PROC CORR (issue #18)
```

## snapshot__fixtures@m24__npar1way.sas.snap

- Snapshot : `tests/snapshots/snapshot__fixtures@m24__npar1way.sas.snap`
- Raison : non-régression de PROC NPAR1WAY WILCOXON — Z de Wilcoxon calculé sans
  correction de continuité (issue #19) — fige la sortie insta du fixture
  `tests/fixtures/m24/npar1way.sas`.
- Commit d'origine : `f9d3543`
  (« npar1way: Z de Wilcoxon sans correction de continuité (issue #19) ») — le
  snapshot a été modifié sans la ligne `Snapshot:` dans le message de commit.
- Revue : J02-P5 (attempt `7b4deaa6`) — finding bloquant signalant l'absence de
  justification.
- Décision : `3865498b` — correction par le présent registre rétroactif plutôt que
  par réécriture/amendement de l'historique poussé.

Ligne canonique rétroactive :

```
Snapshot: tests/snapshots/snapshot__fixtures@m24__npar1way.sas.snap — non-régression NPAR1WAY WILCOXON sans correction de continuité (issue #19)
```

## snapshot__fixtures@m34__npar1way_by_out.sas.snap

- Snapshot : `tests/snapshots/snapshot__fixtures@m34__npar1way_by_out.sas.snap`
- Raison : non-régression de PROC NPAR1WAY WILCOXON avec `BY` et `OUTPUT OUT=`
  (issue #19) — fige la sortie insta du fixture
  `tests/fixtures/m34/npar1way_by_out.sas`.
- Commit d'origine : `f9d3543`
  (« npar1way: Z de Wilcoxon sans correction de continuité (issue #19) ») — le
  snapshot a été modifié sans la ligne `Snapshot:` dans le message de commit.
- Revue : J02-P5 (attempt `7b4deaa6`) — finding bloquant signalant l'absence de
  justification.
- Décision : `3865498b` — correction par le présent registre rétroactif plutôt que
  par réécriture/amendement de l'historique poussé.

Ligne canonique rétroactive :

```
Snapshot: tests/snapshots/snapshot__fixtures@m34__npar1way_by_out.sas.snap — non-régression NPAR1WAY WILCOXON BY + OUTPUT OUT= (issue #19)
```

## snapshot__fixtures@m34__npar1way_scores.sas.snap

- Snapshot : `tests/snapshots/snapshot__fixtures@m34__npar1way_scores.sas.snap`
- Raison : non-régression de PROC NPAR1WAY avec options de scores (issue #19) —
  fige la sortie insta du fixture `tests/fixtures/m34/npar1way_scores.sas`.
- Commit d'origine : `f9d3543`
  (« npar1way: Z de Wilcoxon sans correction de continuité (issue #19) ») — le
  snapshot a été modifié sans la ligne `Snapshot:` dans le message de commit.
- Revue : J02-P5 (attempt `7b4deaa6`) — finding bloquant signalant l'absence de
  justification.
- Décision : `3865498b` — correction par le présent registre rétroactif plutôt que
  par réécriture/amendement de l'historique poussé.

Ligne canonique rétroactive :

```
Snapshot: tests/snapshots/snapshot__fixtures@m34__npar1way_scores.sas.snap — non-régression NPAR1WAY scores (issue #19)
```
