#!/usr/bin/env python3
# check_coverage_claims.py — garde de cohérence README.md ↔ conformance (J08-P6,
# durci en J01-P8).
#
# Chaque ligne du README.md qui pose une promesse « validée » — au sens de la
# marque « *validated against a reference* » — doit être adossée à AU MOINS UN
# cas `validated` correspondant, à la fois dans le corpus de conformance
# (conformance/cases/**/<id>/case.json, source unique de vérité, découverte
# récursive) et dans son inventaire rendu conformance/STATUS.md (ligne
# « validé » du tableau de détail : un STATUS.md périmé ne peut pas adosser
# une promesse que le corpus ne tient plus, ni l'inverse).
#
# Forme des promesses reconnues dans README.md (casse indifférente) :
#   … backed by validated cases `compat/means/*`, `base/*`, see … …
#   … Validated case `compat/transpose/transpose-id-let-delimiter` …
# c.-à-d. la mention « validated case(s) » suivie d'une ou plusieurs
# références entre accents graves, séparées par des virgules, des espaces,
# « and » ou « et ». Une référence est :
#   - `<groupe>[/<sous-groupe>…]/*` : tout cas sous ce préfixe, à toute
#     profondeur (`compat/*` couvre `compat/<proc>/<id>`) ;
#   - `<groupe>/[…/]<id>` (au moins deux segments, sans `/*`) : ce cas précis.
#
# Le check échoue (exit 1) si :
#   1. aucune promesse « validated cases » n'est trouvée (régression du
#      repérage : le README doit documenter ses cas validés) ;
#   2. une référence est mal formée (p. ex. un segment seul sans `/*`) ;
#   3. une référence ne désigne aucun cas `validated` du corpus (répertoire
#      absent, cas `known-divergence` uniquement, ou cas retiré du corpus
#      sans mise à jour du README) ;
#   4. les cas validés désignés ne sont pas « validé » dans STATUS.md.
#
# Sorties : exit 0 = cohérent ; exit 1 = promesse non adossée ;
# exit 2 = erreur d'outil (README, corpus ou STATUS.md illisible).
#
# Self-test (--self-test) : reproducers sur des copies temporaires — l'arbre
# du dépôt n'est jamais modifié — vérifiant que chaque forme de référence est
# reconnue et qu'une promesse privée de son cas validé (corpus ou STATUS.md)
# fait échouer le check (un test qui prouve une propriété doit échouer quand
# la propriété est brisée, cf. CONTRIBUTING.md).
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

# « validated cases `compat/means/*`, `base/*` … » : la mention, puis la suite
# de références entre accents graves (séparateurs : virgule, espace, and/et).
CLAIM_RE = re.compile(
    r"validated\s+cases?\s*:?\s*"
    r"((?:`[^`\n]+`(?:\s*(?:,|\band\b|\bet\b)\s*|\s+)?)+)",
    re.IGNORECASE,
)
BACKTICK_RE = re.compile(r"`([^`\n]+)`")
# `groupe/*`, `groupe/sous-groupe/*` (préfixe) ou `groupe/…/id` (cas précis).
REF_RE = re.compile(r"^(?P<path>[A-Za-z0-9_\-]+(?:/[A-Za-z0-9_\-]+)*)(?P<glob>/\*)?$")
STATUS_VALIDATED = "validé"


def fail(messages):
    for message in messages:
        print(f"check_coverage_claims.py: ÉCHEC — {message}", file=sys.stderr)
    print(
        "check_coverage_claims.py: promesses README non adossées au corpus "
        f"({len(messages)} problème(s)) — voir ci-dessus.",
        file=sys.stderr,
    )
    return 1


def parse_ref(ref):
    """(chemin, est_un_préfixe) ou None si la référence est mal formée."""
    match = REF_RE.match(ref.strip())
    if not match:
        return None
    path, is_prefix = match.group("path"), bool(match.group("glob"))
    if not is_prefix and "/" not in path:
        # Un segment seul sans `/*` est ambigu (groupe ou cas ?) : refusé.
        return None
    return path, is_prefix


def validated_case_paths(cases_root):
    """Chemins relatifs (sous cases/) des cas `validated`, à toute profondeur.

    Un répertoire qui contient un case.json est un cas : un case.json plus
    profond (sous data/, expected/…) lui appartient et n'est pas un cas.
    """
    case_dirs = sorted(path.parent for path in cases_root.rglob("case.json"))
    paths = set()
    seen = []
    for case_dir in case_dirs:
        if any(parent in seen for parent in case_dir.parents):
            continue
        seen.append(case_dir)
        case_path = case_dir / "case.json"
        try:
            data = json.loads(case_path.read_text(encoding="utf-8"))
        except (OSError, ValueError) as error:
            raise ValueError(f"{case_path} illisible : {error}") from error
        if isinstance(data, dict) and data.get("status") == "validated":
            paths.add(case_dir.relative_to(cases_root).as_posix())
    return paths


def status_validated_paths(status_path):
    """Chemins `<groupe>/<id>` marqués « validé » dans conformance/STATUS.md.

    Lignes du tableau de détail : | `<groupe>` | `<id>` | <statut> | <provenance> |
    """
    paths = set()
    for line in status_path.read_text(encoding="utf-8").splitlines():
        cells = [cell.strip() for cell in line.split("|")]
        if (
            len(cells) >= 6
            and cells[1].startswith("`")
            and cells[2].startswith("`")
            and cells[3] == STATUS_VALIDATED
        ):
            paths.add(f"{cells[1].strip('`')}/{cells[2].strip('`')}")
    return paths


def readme_claims(readme_path):
    """(numéro de ligne, [références brutes]) des promesses « validated cases »."""
    claims = []
    text = readme_path.read_text(encoding="utf-8")
    for number, line in enumerate(text.splitlines(), start=1):
        for claim in CLAIM_RE.finditer(line):
            refs = BACKTICK_RE.findall(claim.group(1))
            if refs:
                claims.append((number, refs))
    return claims


def matching_paths(paths, path, is_prefix):
    if is_prefix:
        return {candidate for candidate in paths if candidate.startswith(path + "/")}
    return {path} & paths


def validate(readme_path, cases_root, status_path):
    """Renvoyer (code retour, problèmes) sur des fichiers réels ou copies."""
    if not readme_path.is_file():
        return 2, [f"README introuvable : {readme_path}"]
    if not cases_root.is_dir():
        return 2, [f"corpus introuvable : {cases_root}"]
    if not status_path.is_file():
        return 2, [f"inventaire introuvable : {status_path}"]
    try:
        claims = readme_claims(readme_path)
        corpus = validated_case_paths(cases_root)
        inventory = status_validated_paths(status_path)
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
            where = f"{readme_path.name}:{number} : promesse « validated cases `{ref}` »"
            parsed = parse_ref(ref)
            if parsed is None:
                problems.append(
                    f"{where} : référence mal formée (attendu `<groupe>/*`, "
                    "`<groupe>/<sous-groupe>/*` ou `<groupe>/…/<id>`)."
                )
                continue
            in_corpus = matching_paths(corpus, *parsed)
            if not in_corpus:
                problems.append(
                    f"{where} sans aucun cas `validated` correspondant dans "
                    f"{cases_root}."
                )
            elif not in_corpus & inventory:
                problems.append(
                    f"{where} : cas validés du corpus ({', '.join(sorted(in_corpus))}) "
                    f"absents des lignes « {STATUS_VALIDATED} » de {status_path.name} "
                    "— inventaire périmé (python3 scripts/conformance_report.py)."
                )
    return (1 if problems else 0), problems


def self_test():
    """Reproducer et régression sur des copies temporaires (dépôt intact)."""
    with tempfile.TemporaryDirectory(prefix="check-coverage-claims-") as directory:
        root = Path(directory)
        readme = root / "README.md"
        cases = root / "cases"
        status = root / "STATUS.md"
        shutil.copyfile(README, readme)
        shutil.copytree(CASES_DIR, cases)
        shutil.copyfile(STATUS, status)
        original_readme = readme.read_text(encoding="utf-8")
        original_status = status.read_text(encoding="utf-8")

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

        def with_claim(line):
            readme.write_text(original_readme + "\n" + line + "\n", encoding="utf-8")

        def parsed(ref):
            return any(ref in refs for _, refs in readme_claims(readme))

        # 1. Arbre intact : toutes les promesses du README sont adossées.
        passed = assert_case("promesses adossées", 0)

        # 2. Régression : une promesse dont le préfixe perd TOUS ses cas
        #    validés (case.json renommés en .bak) doit faire échouer le check.
        first_ref = readme_claims(readme)[0][1][0]
        prefix = parse_ref(first_ref)[0]
        case_files = sorted((cases / prefix).rglob("case.json"))
        assert case_files, f"corpus de test vide pour {first_ref}"
        for case_file in case_files:
            case_file.rename(case_file.with_suffix(".json.bak"))
        passed = assert_case("cas validés retirés", 1, first_ref) and passed
        for case_file in case_files:
            case_file.with_suffix(".json.bak").rename(case_file)

        # 3. Régression : un groupe inexistant promis doit échouer.
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

        # 4. CLAIM_RE insensible à la casse : « Validated Cases » est repéré.
        with_claim("Also covered by Validated Cases `compat/inexistant/*`.")
        passed = (
            assert_case("promesse en casse mixte repérée", 1, "compat/inexistant")
            and passed
        )

        # 5. Préfixes à un segment (`compat/*`, `base/*`) : reconnus et
        #    adossés récursivement (les cas compat sont à profondeur 3).
        with_claim("Backed by validated cases `compat/*`, `base/*` and `stat/*`.")
        ok = parsed("compat/*") and parsed("base/*") and parsed("stat/*")
        if not ok:
            print(
                "check_coverage_claims.py: self-test ÉCHEC — `compat/*`/`base/*` "
                "non reconnus par le repérage.",
                file=sys.stderr,
            )
        passed = ok and assert_case("préfixes `compat/*`, `base/*`", 0) and passed
        with_claim("Backed by validated cases `inexistant/*`.")
        passed = assert_case("préfixe à un segment inexistant", 1, "inexistant/*") and passed

        # 6. Chemin de cas unique (sans `/*`) : reconnu, exact.
        single = sorted(validated_case_paths(cases))[0]
        with_claim(f"See validated case `{single}`.")
        passed = (parsed(single) and assert_case("chemin de cas unique", 0)) and passed
        with_claim(f"See validated case `{single}-inexistant`.")
        passed = assert_case("chemin de cas unique inexistant", 1, single) and passed

        # 7. Référence mal formée (segment seul, sans `/*`).
        with_claim("See validated cases `compat`.")
        passed = assert_case("référence mal formée", 1, "mal formée") and passed
        readme.write_text(original_readme, encoding="utf-8")

        # 8. STATUS.md effectif : si l'inventaire ne marque plus « validé »
        #    les cas promis, le check échoue même si le corpus les tient.
        covered = matching_paths(validated_case_paths(cases), prefix, True)
        lines = []
        for line in original_status.splitlines(keepends=True):
            cells = [cell.strip() for cell in line.split("|")]
            row = (
                f"{cells[1].strip('`')}/{cells[2].strip('`')}" if len(cells) >= 6 else ""
            )
            if row in covered:
                line = line.replace(f"| {STATUS_VALIDATED} |", "| divergence connue |")
            lines.append(line)
        status.write_text("".join(lines), encoding="utf-8")
        passed = assert_case("inventaire STATUS.md périmé", 1, "STATUS.md") and passed
        status.unlink()
        passed = assert_case("inventaire STATUS.md absent", 2, "STATUS.md") and passed

    return 0 if passed else 1


def main():
    if len(sys.argv) > 2 or (len(sys.argv) == 2 and sys.argv[1] != "--self-test"):
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
        "validated du corpus conformance et de son inventaire STATUS.md."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
