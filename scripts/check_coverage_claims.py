#!/usr/bin/env python3
# check_coverage_claims.py — garde de cohérence README.md ↔ conformance (J08-P6).
#
# Chaque ligne du README.md qui pose une promesse « validée » — au sens de la
# marque « *validated against a reference* » — doit être adossée à AU MOINS UN
# cas `validated` correspondant dans le corpus de conformance (inventaire
# rendu dans conformance/STATUS.md, généré à partir des fichiers
# conformance/cases/<groupe>/<id>/case.json, source unique de vérité).
#
# Forme des promesses reconnues dans README.md :
#   … (validated cases `compat/means/*`, see conformance/STATUS.md) …
# c.-à-d. la mention « validated cases » suivie de références
# `<groupe>/<sous-groupe>/*` (séparées par des virgules ou des espaces).
#
# Le check échoue (exit 1) si :
#   1. aucune promesse « validated cases » n'est trouvée (régression du
#      repérage : le README doit documenter ses cas validés) ;
#   2. une référence pointe vers un groupe sans aucun cas `validated`
#      (répertoire absent, cas `known-divergence` uniquement, ou cas retiré
#      du corpus sans mise à jour du README).
#
# Sorties : exit 0 = cohérent ; exit 1 = promesse non adossée ;
# exit 2 = erreur d'outil (README ou corpus illisible).
#
# Self-test (--self-test) : reproducer sur des copies temporaires — l'arbre
# du dépôt n'est jamais modifié — vérifiant qu'une promesse privée de son cas
# validé fait échouer le check (un test qui prouve une propriété doit échouer
# quand la propriété est brisée, cf. CONTRIBUTING.md).
#
# Usage : python3 scripts/check_coverage_claims.py [--self-test]

import json
import re
import shutil
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
README = REPO_ROOT / "README.md"
CASES_DIR = REPO_ROOT / "conformance" / "cases"
STATUS = REPO_ROOT / "conformance" / "STATUS.md"

# « validated cases `compat/means/*`, … » — la liste peut enchaîner plusieurs
# références séparées par des virgules ; chaque référence finit par « /* ».
CLAIM_RE = re.compile(r"validated cases\s+([^)|\n]*)")
REF_RE = re.compile(r"([A-Za-z0-9_\-]+(?:/[A-Za-z0-9_\-]+)+)/\*")


def fail(messages):
    for message in messages:
        print(f"check_coverage_claims.py: ÉCHEC — {message}", file=sys.stderr)
    print(
        "check_coverage_claims.py: promesses README non adossées au corpus "
        f"({len(messages)} problème(s)) — voir ci-dessus.",
        file=sys.stderr,
    )
    return 1


def validated_groups(cases_root):
    """Groupes (chemin relatif sous cases/) comptant au moins un cas `validated`."""
    groups = set()
    # Les groupes peuvent être imbriqués (ex. compat/means/<cas>/case.json) :
    # le groupe est le parent du répertoire du cas, à toute profondeur.
    for case_path in sorted(cases_root.rglob("case.json")):
        try:
            data = json.loads(case_path.read_text(encoding="utf-8"))
        except (OSError, ValueError) as error:
            raise ValueError(f"{case_path} illisible : {error}") from error
        if data.get("status") == "validated":
            group = case_path.parent.parent.relative_to(cases_root).as_posix()
            groups.add(group)
    return groups


def status_validated_case_ids(status_path):
    """Identifiants de cas marqués « validé » dans conformance/STATUS.md.

    Le STATUS.md est l'inventaire rendu du corpus ; le lire ici garantit que
    le check s'aligne sur ce que le README désigne. Retourne None si le
    fichier est absent (le corpus case.json reste alors la seule source).
    """
    if not status_path.is_file():
        return None
    ids = set()
    for line in status_path.read_text(encoding="utf-8").splitlines():
        cells = [cell.strip() for cell in line.split("|")]
        if len(cells) >= 5 and cells[3] == "validé":
            ids.add(cells[2].strip("`"))
    return ids


def readme_claims(readme_path):
    """(numéro de ligne, [références]) des promesses « validated cases »."""
    claims = []
    text = readme_path.read_text(encoding="utf-8")
    for number, line in enumerate(text.splitlines(), start=1):
        for claim in CLAIM_RE.finditer(line):
            refs = REF_RE.findall(claim.group(1))
            if refs:
                claims.append((number, refs))
    return claims


def validate(readme_path, cases_root, status_path):
    """Renvoyer (code retour, problèmes) sur des fichiers réels ou copies."""
    if not readme_path.is_file():
        return 2, [f"README introuvable : {readme_path}"]
    if not cases_root.is_dir():
        return 2, [f"corpus introuvable : {cases_root}"]
    try:
        claims = readme_claims(readme_path)
        groups = validated_groups(cases_root)
        status_ids = status_validated_case_ids(status_path)
    except (OSError, UnicodeError, ValueError) as error:
        return 2, [f"fichier illisible : {error}"]

    if not claims:
        return 1, [
            "aucune promesse « validated cases » trouvée dans "
            f"{readme_path} — le repérage des promesses a-t-il régressé ?"
        ]

    problems = []
    for number, refs in claims:
        for ref in refs:
            if ref in groups:
                continue
            if status_ids and status_ids & _case_ids_for(cases_root, ref):
                continue
            problems.append(
                f"{readme_path.name}:{number} : promesse « validated cases "
                f"`{ref}/*` » sans aucun cas `validated` correspondant dans "
                f"{cases_root} (inventaire : {status_path.name})."
            )
    return (1 if problems else 0), problems


def _case_ids_for(cases_root, group):
    """Identifiants des cas `validated` d'un groupe (rapprochement STATUS.md)."""
    ids = set()
    for case_path in (cases_root / group).glob("*/case.json"):
        data = json.loads(case_path.read_text(encoding="utf-8"))
        if data.get("status") == "validated":
            ids.add(case_path.parent.name)
    return ids


def self_test():
    """Reproducer et régression sur des copies temporaires (dépôt intact)."""
    with tempfile.TemporaryDirectory(prefix="check-coverage-claims-") as directory:
        root = Path(directory)
        readme = root / "README.md"
        cases = root / "cases"
        status = root / "STATUS.md"
        shutil.copyfile(README, readme)
        shutil.copytree(CASES_DIR, cases)
        if STATUS.is_file():
            shutil.copyfile(STATUS, status)

        def assert_case(name, expected_code, expected_message=None):
            code, problems = validate(readme, cases, status)
            matches = expected_message is None or any(
                expected_message in problem for problem in problems
            )
            if code != expected_code or not matches:
                print(
                    f"check_coverage_claims.py: self-test ÉCHEC — {name} : "
                    f"exit {code}, attendu {expected_code} ; {problems}",
                    file=sys.stderr,
                )
                return False
            print(f"check_coverage_claims.py: self-test OK — {name} : exit {code}")
            return True

        # 1. Arbre intact : toutes les promesses du README sont adossées.
        passed = assert_case("promesses adossées", 0)

        original_readme = readme.read_text(encoding="utf-8")

        # 2. Régression : une promesse dont le groupe perd TOUS ses cas
        #    validés (case.json renommés en .bak) doit faire échouer le check.
        first_ref = readme_claims(readme)[0][1][0]
        case_files = sorted((cases / first_ref).glob("*/case.json"))
        assert case_files, f"corpus de test vide pour {first_ref}"
        for case_file in case_files:
            case_file.rename(case_file.with_suffix(".json.bak"))
        passed = (
            assert_case("cas validés retirés", 1, first_ref) and passed
        )
        for case_file in case_files:
            case_file.with_suffix(".json.bak").rename(case_file)
        readme.write_text(original_readme, encoding="utf-8")

        # 3. Régression : une promesse inconnue dans le README doit échouer.
        readme.write_text(
            original_readme.replace(
                "validated cases",
                "validated cases `compat/inexistant/*`, validated cases",
                1,
            ),
            encoding="utf-8",
        )
        passed = (
            assert_case("groupe inexistant promis", 1, "compat/inexistant") and passed
        )
        readme.write_text(original_readme, encoding="utf-8")

    return 0 if passed else 1


def main():
    if len(sys.argv) > 2 or (len(sys.argv) == 2 and not sys.argv[1].startswith("--")):
        print("usage: check_coverage_claims.py [--self-test]", file=sys.stderr)
        return 2
    if len(sys.argv) == 2 and sys.argv[1] != "--self-test":
        print("usage: check_coverage_claims.py [--self-test]", file=sys.stderr)
        return 2
    if len(sys.argv) == 2:
        return self_test()
    code, problems = validate(README, CASES_DIR, STATUS)
    if code == 2:
        for problem in problems:
            print(f"check_coverage_claims.py: ÉCHEC (outil) — {problem}", file=sys.stderr)
        return code
    if problems:
        return fail(problems)
    claims = readme_claims(README)
    refs = sorted({ref for _, group in claims for ref in group})
    print(
        f"check_coverage_claims.py: OK — {len(claims)} promesse(s) "
        f"« validated cases » ({', '.join(refs)}) adossées à au moins un cas "
        "validated du corpus conformance (inventaire : STATUS.md)."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
