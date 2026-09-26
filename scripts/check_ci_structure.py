#!/usr/bin/env python3
# check_ci_structure.py — garde structurel du workflow CI (J02-P0, J02-P9).
# Tout ajout de job CI doit aussi mettre à jour EXPECTED_JOBS ci-dessous.
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
#   5. un des huit jobs obligatoires disparaît du workflow ou de `needs` ;
#   6. une commande cargo d'un step `run` manque dans scripts/check.sh.
#
# Sorties : exit 0 = structure intacte ; exit 1 = structure affaiblie ;
# exit 2 = erreur d'outil (fichier absent, YAML illisible, PyYAML manquant).
#
# Environnement : PyYAML requis (le python3 des runners ubuntu-latest de la
# CI l'a ; sur l'hôte de dev, utiliser /usr/bin/python3 — F10).
#
# Usage : python3 scripts/check_ci_structure.py [chemin/vers/ci.yml]
#         python3 scripts/check_ci_structure.py --self-test
#         (défaut : <racine du dépôt>/.github/workflows/ci.yml)

import shutil
import sys
import tempfile
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
DEFAULT_CHECK_SCRIPT = Path(__file__).resolve().parent / "check.sh"
AGGREGATOR = "ci-ok"
EXPECTED_JOBS = (
    "fmt",
    "clippy",
    "clippy-graphics",
    "clippy-s3",
    "test",
    "test-graphics",
    "test-s3",
    "test-fault-injection",
)


def fail(messages):
    for message in messages:
        print(f"check_ci_structure.py: ÉCHEC — {message}", file=sys.stderr)
    print(
        "check_ci_structure.py: structure du workflow CI affaiblie "
        f"({len(messages)} problème(s)) — voir ci-dessus.",
        file=sys.stderr,
    )
    return 1


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


def cargo_commands(source):
    """Commandes cargo autonomes ; ignorer commentaires et echo du shell."""
    return {
        line.strip()
        for line in source.splitlines()
        if line.lstrip().startswith("cargo ")
    }


def validate(workflow_path, check_script_path):
    """Renvoyer (code retour, problèmes) pour des fichiers réels ou temporaires."""
    if not workflow_path.is_file():
        return 2, [f"workflow introuvable : {workflow_path}"]
    if not check_script_path.is_file():
        return 2, [f"script local introuvable : {check_script_path}"]
    try:
        document = yaml.safe_load(workflow_path.read_text(encoding="utf-8"))
        check_script = check_script_path.read_text(encoding="utf-8")
    except (OSError, UnicodeError, yaml.YAMLError) as error:
        return 2, [f"fichier illisible ({workflow_path} ou {check_script_path}) : {error}"]

    jobs = document.get("jobs") if isinstance(document, dict) else None
    if not isinstance(jobs, dict) or not jobs:
        return 1, [f"aucun job défini dans {workflow_path} — la CI a été vidée."]

    problems = []

    # 1. Le job agrégé `ci-ok` doit exister.
    if AGGREGATOR not in jobs:
        return 1, [
            f"le job « {AGGREGATOR} » est absent de {workflow_path} : "
            "l'agrégateur obligatoire (seul check de la protection de branche) a été retiré."
        ]
    ci_ok = jobs[AGGREGATOR] or {}
    if not isinstance(ci_ok, dict):
        return 1, [f"le job « {AGGREGATOR} » n'a pas de définition valide."]

    # 2. Chaque job référencé dans needs de ci-ok doit exister.
    needs = as_job_list(ci_ok.get("needs"))
    missing = [job for job in needs if job not in jobs]
    if missing:
        problems.append(
            f"ci-ok.needs référence des jobs inexistants : {', '.join(missing)} "
            "(job supprimé sans retirer son entrée de needs ?)."
        )

    # La liste figée empêche la suppression simultanée d'un job et de needs.
    absent_jobs = [job for job in EXPECTED_JOBS if job not in jobs]
    if absent_jobs:
        problems.append(f"jobs CI obligatoires absents : {', '.join(absent_jobs)}.")
    absent_needs = [job for job in EXPECTED_JOBS if job not in needs]
    if absent_needs:
        problems.append(
            f"jobs CI obligatoires absents de {AGGREGATOR}.needs : {', '.join(absent_needs)}."
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

    local_commands = cargo_commands(check_script)
    for job_name, job in jobs.items():
        if not isinstance(job, dict):
            problems.append(f"le job « {job_name} » n'a pas de définition valide.")
            continue
        for step in job.get("steps") or []:
            if not isinstance(step, dict) or not isinstance(step.get("run"), str):
                continue
            for command in sorted(cargo_commands(step["run"])):
                if command not in local_commands:
                    problems.append(
                        f"commande cargo du job « {job_name} » absente de "
                        f"{check_script_path.name} : {command}"
                    )

    return (1 if problems else 0), problems


def self_test():
    """Reproducer et régression sur des copies, sans modifier le dépôt."""
    with tempfile.TemporaryDirectory(prefix="check-ci-structure-") as directory:
        root = Path(directory)
        workflow = root / "ci.yml"
        check_script = root / "check.sh"
        shutil.copyfile(DEFAULT_WORKFLOW, workflow)
        shutil.copyfile(DEFAULT_CHECK_SCRIPT, check_script)

        def assert_case(name, expected_code, expected_message=None):
            code, problems = validate(workflow, check_script)
            matches = expected_message is None or any(
                expected_message in problem for problem in problems
            )
            if code != expected_code or not matches:
                print(
                    f"check_ci_structure.py: self-test ÉCHEC — {name} : "
                    f"exit {code}, attendu {expected_code} ; {problems}",
                    file=sys.stderr,
                )
                return False
            print(f"check_ci_structure.py: self-test OK — {name} : exit {code}")
            return True

        passed = assert_case("arbre intact", 0)
        original_workflow = workflow.read_text(encoding="utf-8")
        original_check_script = check_script.read_text(encoding="utf-8")

        document = yaml.safe_load(original_workflow)
        del document["jobs"]["fmt"]
        document["jobs"][AGGREGATOR]["needs"].remove("fmt")
        workflow.write_text(yaml.safe_dump(document, sort_keys=False), encoding="utf-8")
        passed = assert_case("job retiré partout", 1, "fmt") and passed
        workflow.write_text(original_workflow, encoding="utf-8")

        command = "cargo fmt --check"
        check_script.write_text(
            original_check_script.replace(f"    {command}\n", "", 1),
            encoding="utf-8",
        )
        passed = assert_case("commande cargo retirée de check.sh", 1, command) and passed
        check_script.write_text(original_check_script, encoding="utf-8")

        document = yaml.safe_load(original_workflow)
        document["jobs"][AGGREGATOR]["continue-on-error"] = True
        workflow.write_text(yaml.safe_dump(document, sort_keys=False), encoding="utf-8")
        passed = assert_case("continue-on-error sur ci-ok", 1, "continue-on-error") and passed

    return 0 if passed else 1


def main():
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    if len(sys.argv) > 2 or (len(sys.argv) == 2 and sys.argv[1].startswith("-")):
        print("usage: check_ci_structure.py [ci.yml|--self-test]", file=sys.stderr)
        return 2
    workflow_path = Path(sys.argv[1]) if len(sys.argv) == 2 else DEFAULT_WORKFLOW
    code, problems = validate(workflow_path, DEFAULT_CHECK_SCRIPT)
    if code == 2:
        for problem in problems:
            print(f"check_ci_structure.py: ÉCHEC (outil) — {problem}", file=sys.stderr)
        return code
    if problems:
        return fail(problems)

    print(
        f"check_ci_structure.py: OK — {workflow_path.name} : "
        f"« {AGGREGATOR} » agrège tous les jobs obligatoires ; "
        "aucun continue-on-error ; commandes cargo présentes dans check.sh."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
