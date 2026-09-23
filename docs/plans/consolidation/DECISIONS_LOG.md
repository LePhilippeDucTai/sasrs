# DECISIONS_LOG — consolidation

Append-only, one line per decision. Never edit past lines.

- 2026-09-23 D-001 Plan consolidation (issue #11, sous-issues #5–#10, M46–M66 remappés) : 8 jalons / 58 parts approuvés, ordre de l'issue sauf CI avancée en J01 (arbitre des jalons suivants) ; baseline rouge (clippy 52 imports, snapshot m36/plots sous graphics) corrigée en J01-P1 ; `VarMeta.length` et troncatures en caractères (session SAS LATIN1/WLATIN1, écart UTF-8 documenté) ; protection de main par check `ci-ok` posée via gh api en J01-P5 (reco: approuver ; J01-P1 ; caractères ; oui) (user)
- 2026-09-23 D-002 CI graphics : les 2 jobs graphics (clippy+test) échouent sur le runner ubuntu-latest (build.rs de `yeslogic-fontconfig-sys` : pkg-config/fontconfig absents, runs 35882867341 et 35883283504) ; décision = installer les paquets système dans la CI (`pkg-config`, `libfontconfig1-dev`, `libfreetype-dev`, `fonts-liberation`) via part corrective J01-P7 — la dépendance graphics reste réelle et bloquante, ni retrait du profil ni affaiblissement des jobs ; `ci-ok` reste le seul check obligatoire (reco: approuver ; J01-P7 ; fontconfig ; oui) (orchestrateur, instruction permanente)
