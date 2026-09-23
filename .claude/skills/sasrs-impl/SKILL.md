---
name: sasrs-impl
description: REMPLACÉE (2026-09-23) — ne plus invoquer. La roadmap M1–M66 est gelée ; le projet sasrs est piloté par le plan de consolidation dans docs/plans/consolidation/. Utiliser /milestone-continue consolidation à la place.
---

# sasrs-impl — skill remplacée

Cette skill est **remplacée par `/milestone-continue consolidation`** depuis le
2026-09-23. La feuille de route M1–M66 qu'elle pilotait (`PLAN.md`/`PROGRESS.md` racine)
est **gelée** ; le projet suit désormais le plan de consolidation :

1. `docs/plans/consolidation/PLAN.md` — périmètre, architecture, risques, conventions ;
2. `docs/plans/consolidation/PROGRESS.md` — statut des jalons J01–J08 et de leurs parts
   (géré par l'orchestrateur, seule source de vérité du curseur) ;
3. `docs/plans/consolidation/jalons/J0N-*.md` — contrat de chaque part (scope, files,
   acceptance, contexte).

Ne pas reprendre le protocole historique de cette skill : il lisait `PROGRESS.md`
racine comme curseur et poussait jalon par jalon sur une branche unique, ce qui n'est
plus le modèle (work par parts, fusion une part = un commit de merge, CI bloquante
`ci-ok` comme arbitre — voir `CONTRIBUTING.md` pour les règles de développement).
Le remapping des jalons restants M46–M66 est décrit dans `PLAN.md` racine
§ « Correspondance M46–M66 → consolidation ».

**Republication du wrapper Python** (`python/`) : PAS gérée par cette skill. Un watchdog
Hermes indépendant (`~/.hermes/scripts/cron-sasrs/rebuild-python-release.sh`, cron
`*/5 * * * *`) détecte tout nouveau commit poussé sur ce dépôt (par cette skill ou
autrement), recompile `sasrs.exe` pour Windows, resynchronise le SHA-256 dans
`python/src/sasrs_py/cli.py` et republie sur la release `python-v0.1.0`. Ne PAS
dupliquer cette logique ici — l'orchestrateur n'a rien à faire de spécifique pour que
le wrapper Python reste à jour, au-delà de committer et pousser normalement.
