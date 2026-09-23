# LONG_TERM_RECO — consolidation

One line per item. Remove an item when done and note it in DECISIONS_LOG.md.

- R-001 (D-001, 2026-09-23): couper le watchdog Hermes `~/.hermes/scripts/cron-sasrs/rebuild-python-release.sh` (commits SHA sur main, asset remplacé en place) au profit des releases versionnées ; when: première release `v*` publiée par `release.yml` (J06-P5) ; est. T5/S
- R-002 (D-001, 2026-09-23): planifier la suite avancée avec `/milestone-plan` à partir de `docs/roadmap/avancee.md` (LOGISTIC…GLIMMIX, multivarié, IML, S3, graphiques, ex-M46–M49, M51–M66) ; when: J08 clos ; est. T1/M
- R-003 (D-001, 2026-09-23): fusionner consolidation → main par PR avec `ci-ok` vert, puis supprimer les branches distantes obsolètes (`claude/sasrs-impl-96tjok`, `claude/update-plan-progress-4ec8ll`, `wip/m41.1`) ; when: fin de chaque jalon validé ou J08 ; est. T5/S
