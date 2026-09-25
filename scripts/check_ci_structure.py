#!/usr/bin/env python3
# check_ci_structure.py — garde structurel du workflow CI (J02-P0, F1/F5).
#
# L'arbitre `ci-ok` valide la STRUCTURE du workflow avant de déclarer vert :
# ce script parse `.github/workflows/ci.yml` et échoue si le workflow a été
# affaibli (revue J01, F5/M2-v1 : les greps d'acceptation ne détectent pas la
# disparition d'un job entier ; F1 : la protection de main garantit le nom du
# check, pas son contenu).
#
# Il échoue (exit 1, message explicite) si :
#   1. le job `ci-ok` est absent du workflow ;
#   2. un job référencé dans `needs` de `ci-ok` n'existe pas ;
#   3. `ci-ok` porte un `continue-on-error` (niveau job, ou step — un step du
#      garde lui-même en `continue-on-error` rendrait ce contrôle théâtral) ;
#   4. `ci-ok` n'agrège pas TOUS les autres jobs (needs absent, vide, ou
#      incomplet) — un job hors `needs` peut échouer sans faire échouer
#      `ci-ok`.
#
# Sorties : exit 0 = structure intacte ; exit 1 = structure affaiblie ;
# exit 2 = erreur d'outil (fichier absent, YAML illisible, PyYAML manquant).
#
# Environnement : PyYAML requis (le python3 des runners ubuntu-latest de la
# CI l'a ; sur l'hôte de dev, utiliser /usr/bin/python3 — F10).
#
# Usage : python3 scripts/check_ci_structure.py [chemin/vers/ci.yml]
#         (défaut : <racine du dépôt>/.github/workflows/ci.yml)

import sys
from pathlib import Path

try:
    import yaml
except ImportError:
    print(
        "check_ci_structure.py: ÉCHEC (outil) — le module PyYAML est requis "
        "(python3 -m pip install pyyaml, ou utiliser un python3 qui l'a ; "
        "sur l'hôte de dev : /usr/bin/python3).",
        file=sys.stderr,
    )
    sys.exit(2)

DEFAULT_WORKFLOW = Path(__file__).resolve().parent.parent / ".github" / "workflows" / "ci.yml"
AGGREGATOR = "ci-ok"


def fail(messages):
    for message in messages:
        print(f"check_ci_structure.py: ÉCHEC — {message}", file=sys.stderr)
    print(
        "check_ci_structure.py: structure du workflow CI affaiblie "
        f"({len(messages)} problème(s)) — voir ci-dessus.",
        file=sys.stderr,
    )
    sys.exit(1)


def as_job_list(value):
    """`needs: fmt` (chaîne) et `needs: [fmt, test]` (liste) → liste."""
    if value is None:
        return []
    if isinstance(value, str):
        return [value.strip()] if value.strip() else []
    return [str(item) for item in value]


def carries_continue_on_error(value):
    """Vrai si la valeur affaiblit le job/step : true, expression, etc.

    Un `continue-on-error: false` explicite équivaut à l'absence de la clé
    et n'affaiblit rien ; toute autre valeur (true, ${{ ... }}) est refusée.
    """
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        return value.strip().lower() != "false"
    return bool(value)


def main():
    workflow_path = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_WORKFLOW
    if not workflow_path.is_file():
        print(
            f"check_ci_structure.py: ÉCHEC (outil) — workflow introuvable : {workflow_path}",
            file=sys.stderr,
        )
        sys.exit(2)
    try:
        document = yaml.safe_load(workflow_path.read_text(encoding="utf-8"))
    except yaml.YAMLError as error:
        print(
            f"check_ci_structure.py: ÉCHEC (outil) — YAML illisible ({workflow_path}) : {error}",
            file=sys.stderr,
        )
        sys.exit(2)

    jobs = document.get("jobs") if isinstance(document, dict) else None
    if not isinstance(jobs, dict) or not jobs:
        fail([f"aucun job défini dans {workflow_path} — la CI a été vidée."])

    problems = []

    # 1. Le job agrégé `ci-ok` doit exister.
    if AGGREGATOR not in jobs:
        fail(
            [
                f"le job « {AGGREGATOR} » est absent de {workflow_path} : "
                "l'agrégateur obligatoire (seul check de la protection de branche) a été retiré."
            ]
        )
    ci_ok = jobs[AGGREGATOR] or {}

    # 2. Chaque job référencé dans needs de ci-ok doit exister.
    needs = as_job_list(ci_ok.get("needs"))
    missing = [job for job in needs if job not in jobs]
    if missing:
        problems.append(
            f"ci-ok.needs référence des jobs inexistants : {', '.join(missing)} "
            "(job supprimé sans retirer son entrée de needs ?)."
        )

    # 3. Pas de continue-on-error sur ci-ok (job ni steps).
    if carries_continue_on_error(ci_ok.get("continue-on-error")):
        problems.append(
            f"le job « {AGGREGATOR} » porte « continue-on-error » : le check "
            "obligatoire peut alors passer vert malgré ses propres échecs."
        )
    for index, step in enumerate(ci_ok.get("steps") or [], start=1):
        if not isinstance(step, dict):
            continue
        if carries_continue_on_error(step.get("continue-on-error")):
            name = step.get("name") or f"step #{index}"
            problems.append(
                f"le step « {name} » de {AGGREGATOR} porte « continue-on-error » : "
                "l'agrégation (ou le garde structurel) pourrait échouer en silence."
            )

    # 4. ci-ok doit agréger TOUS les autres jobs.
    others = [job for job in jobs if job != AGGREGATOR]
    if not needs:
        problems.append(
            f"« {AGGREGATOR} » n'agrège aucun job (needs absent ou vide) : "
            "il serait vert sans attendre le moindre job."
        )
    not_aggregated = [job for job in others if job not in needs]
    if not_aggregated:
        problems.append(
            f"des jobs du workflow ne sont pas agrégés par « {AGGREGATOR} » "
            f"(absents de needs) : {', '.join(not_aggregated)} — leur échec ne "
            "ferait pas échouer le check obligatoire."
        )

    if problems:
        fail(problems)

    print(
        f"check_ci_structure.py: OK — {workflow_path.name} : {len(jobs)} jobs ; "
        f"« {AGGREGATOR} » agrège les {len(others)} autres ; "
        "aucun continue-on-error ; toutes les entrées de needs existent."
    )
    sys.exit(0)


if __name__ == "__main__":
    main()
