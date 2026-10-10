#!/usr/bin/env python3
# replay_oracles.py — rejoue les oracles indépendants committés avec le
# corpus de conformité (roadmap-avancee J01-P5).
#
# Convention (conformance/README.md, conformance/schema.md) : un cas
# `independent-oracle` peut porter `oracle/oracle.py` — un script Python
# bibliothèque standard seulement, déterministe, sans réseau ni
# sous-processus, qui n'exécute jamais `sasrs`. Lancé comme
# `python3 -I -B oracle/oracle.py <dir>` (cwd = le répertoire du cas), il
# écrit `<dir>/<dataset>.csv` pour chaque `expected/<dataset>.csv` du cas.
# L'oracle est la preuve que les attendus du cas sont reproductibles par un
# calcul indépendant de `sasrs`, pas par la relecture d'une sortie figée.
#
# Ce script découvre tous les `conformance/cases/**/oracle/oracle.py`,
# refuse statiquement (via `ast`, sans l'exécuter) tout script qui importe
# un module hors `sys.stdlib_module_names`, ou qui importe
# subprocess/socket/ctypes/multiprocessing, ou qui utilise
# `os.system`/`os.popen` — ces modules/fonctions permettraient de sortir du
# bac à sable (réseau, sous-processus, donc indirectement `sasrs`). Un
# script accepté est exécuté dans un répertoire temporaire frais avec un
# délai, puis chaque `<tmp>/<dataset>.csv` produit est comparé à
# `expected/<dataset>.csv` avec les règles de tolérance de `case.json` —
# mêmes règles que l'exécuteur `tests/conformance.rs` (missings SAS exacts,
# colonnes dans l'ordre, tolérance abs/rel par défaut et par colonne).
#
# Bibliothèque standard uniquement (aucune dépendance).
#
# Usage :
#   python3 -B scripts/replay_oracles.py              # rejoue tout le corpus
#   python3 -B scripts/replay_oracles.py --self-test
#
# Codes retour : 0 = tous les oracles rejoués concordent avec leurs attendus
# (0 oracle → « 0 oracle(s) rejoué(s) », exit 0) ; 1 = au moins une
# divergence, un import interdit ou une erreur d'exécution d'un oracle.

import argparse
import ast
import json
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CASES_DIR = REPO_ROOT / "conformance" / "cases"

ORACLE_TIMEOUT = 30.0
DEFAULT_TOL = 1e-9

# Modules dont l'IMPORT est interdit même s'ils sont dans la bibliothèque
# standard : accès réseau ou sous-processus, donc sortie possible du bac à
# sable (et moyen indirect d'exécuter `sasrs`).
FORBIDDEN_IMPORT_MODULES = {"subprocess", "socket", "ctypes", "multiprocessing"}
# Attributs de `os` interdits : lancement de sous-processus par un autre
# chemin que le module `subprocess`.
FORBIDDEN_OS_ATTRS = {"system", "popen"}


# ── Vérification statique (ast, sans exécution) ────────────────────────


def static_check(source, label):
    """Violations de la convention bac à sable (liste vide = accepté)."""
    try:
        tree = ast.parse(source, filename=label)
    except SyntaxError as error:
        return [f"{label} : syntaxe Python invalide ({error})"]

    problems = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                top = alias.name.split(".")[0]
                if top in FORBIDDEN_IMPORT_MODULES:
                    problems.append(f"{label} : import interdit « {alias.name} »")
                elif top not in sys.stdlib_module_names:
                    problems.append(
                        f"{label} : import hors bibliothèque standard « {alias.name} »"
                    )
        elif isinstance(node, ast.ImportFrom):
            if node.level:
                problems.append(f"{label} : import relatif interdit")
                continue
            top = (node.module or "").split(".")[0]
            if not top:
                problems.append(f"{label} : import invalide")
            elif top in FORBIDDEN_IMPORT_MODULES:
                problems.append(f"{label} : import interdit « {node.module} »")
            elif top not in sys.stdlib_module_names:
                problems.append(
                    f"{label} : import hors bibliothèque standard « {node.module} »"
                )
            elif top == "os":
                for alias in node.names:
                    if alias.name in FORBIDDEN_OS_ATTRS:
                        problems.append(f"{label} : import interdit « os.{alias.name} »")
        elif isinstance(node, ast.Attribute):
            if (
                node.attr in FORBIDDEN_OS_ATTRS
                and isinstance(node.value, ast.Name)
                and node.value.id == "os"
            ):
                problems.append(f"{label} : usage interdit « os.{node.attr} »")
    return problems


# ── Comparaison CSV (mêmes règles que tests/conformance.rs) ────────────


def parse_num_cell(cell):
    """`.`/vide → missing ordinaire, `._`/`.A`..`.Z` → missing spécial,
    sinon un flottant. Rend None si la cellule n'est pas numérique."""
    cell = cell.strip()
    if cell == "" or cell == ".":
        return ("missing", ".")
    if cell == "._":
        return ("missing", "._")
    if len(cell) == 2 and cell[0] == "." and cell[1].isascii() and cell[1].isupper():
        return ("missing", cell)
    try:
        return ("value", float(cell))
    except ValueError:
        return None


def close_enough(got, want, abs_tol, rel_tol):
    diff = abs(got - want)
    return diff <= abs_tol or diff <= rel_tol * max(abs(got), abs(want))


def is_numeric_column(cells):
    """Une colonne est numérique si TOUTES ses cellules (attendues) sont
    des missings ou des flottants — type inféré de l'attendu, à défaut du
    schéma du dataset (l'oracle ne passe jamais par `sasrs::dataset`)."""
    for cell in cells:
        if parse_num_cell(cell) is None:
            return False
    return True


def split_csv(text):
    """Lignes non vides → cellules, mêmes conventions que l'exécuteur
    (séparateur `,` nu, chaque cellule strippée, pas d'échappement)."""
    return [
        [cell.strip() for cell in line.split(",")]
        for line in text.splitlines()
        if line.strip()
    ]


def column_tolerance(tolerance, column):
    columns = tolerance.get("columns") or {}
    override = columns.get(column)
    if override is not None:
        return override.get("abs", DEFAULT_TOL), override.get("rel", DEFAULT_TOL)
    return tolerance.get("abs", DEFAULT_TOL), tolerance.get("rel", DEFAULT_TOL)


def compare_csv(expected_text, produced_text, tolerance):
    """Problèmes (liste vide = concorde) entre un `expected/<ds>.csv` et le
    `<ds>.csv` produit par l'oracle."""
    expected_rows = split_csv(expected_text)
    produced_rows = split_csv(produced_text)
    if not expected_rows:
        return ["CSV attendu vide"]
    if not produced_rows:
        return ["l'oracle n'a produit aucune ligne"]

    header = [cell.upper() for cell in expected_rows[0]]
    produced_header = [cell.upper() for cell in produced_rows[0]]
    if header != produced_header:
        return [f"colonnes : produites {produced_header}, attendues {header}"]

    expected_body = expected_rows[1:]
    produced_body = produced_rows[1:]
    if len(expected_body) != len(produced_body):
        return [
            f"observations : {len(produced_body)} produites, "
            f"{len(expected_body)} attendues"
        ]

    diffs = []
    for j, column in enumerate(header):
        cells = [row[j] if j < len(row) else "" for row in expected_body]
        numeric = is_numeric_column(cells)
        abs_tol, rel_tol = column_tolerance(tolerance, column)
        for i, (erow, prow) in enumerate(zip(expected_body, produced_body)):
            want_cell = erow[j] if j < len(erow) else ""
            got_cell = prow[j] if j < len(prow) else ""
            at = f"obs {i + 1}, colonne {column} : "
            if numeric:
                want = parse_num_cell(want_cell)
                got = parse_num_cell(got_cell)
                if want is None:
                    diffs.append(f"{at}attendu illisible « {want_cell} »")
                elif got is None:
                    diffs.append(f"{at}produit illisible « {got_cell} »")
                elif want[0] == "missing" and got[0] == "missing":
                    if want[1] != got[1]:
                        diffs.append(f"{at}missing {want[1]} attendu, {got[1]} produit")
                elif want[0] == "missing":
                    diffs.append(f"{at}missing {want[1]} attendu, valeur {got[1]} produite")
                elif got[0] == "missing":
                    diffs.append(f"{at}valeur {want[1]} attendue, missing {got[1]} produit")
                elif not close_enough(got[1], want[1], abs_tol, rel_tol):
                    diffs.append(
                        f"{at}{got[1]} produit, {want[1]} attendu "
                        f"(|écart| > abs {abs_tol} / rel {rel_tol})"
                    )
            elif got_cell != want_cell:
                diffs.append(f"{at}« {got_cell} » produit, « {want_cell} » attendu")
    return diffs


# ── Découverte et exécution des oracles ─────────────────────────────────


def discover_oracles(root):
    """`conformance/cases/**/oracle/oracle.py`, ordre stable par chemin."""
    return sorted(root.rglob("oracle/oracle.py"))


def run_oracle_case(case_dir, oracle_path, timeout=ORACLE_TIMEOUT):
    """(ok, problèmes) pour un cas portant un oracle."""
    case_json_path = case_dir / "case.json"
    if not case_json_path.is_file():
        return False, [f"{case_dir} : case.json absent"]
    try:
        case_spec = json.loads(case_json_path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        return False, [f"{case_dir} : case.json illisible ({error})"]
    tolerance = case_spec.get("tolerance") or {}

    # Vérification statique AVANT toute exécution : un import interdit ne
    # lance jamais le script, même dans un bac à sable temporaire.
    source = oracle_path.read_text(encoding="utf-8")
    violations = static_check(source, str(oracle_path))
    if violations:
        return False, violations

    expected_dir = case_dir / "expected"
    expected_csvs = sorted(expected_dir.glob("*.csv")) if expected_dir.is_dir() else []
    if not expected_csvs:
        return False, [f"{case_dir} : aucun expected/*.csv"]

    with tempfile.TemporaryDirectory(prefix="replay-oracle-") as out_dir:
        rel_oracle = oracle_path.relative_to(case_dir)
        try:
            result = subprocess.run(
                [sys.executable, "-I", "-B", str(rel_oracle), out_dir],
                cwd=case_dir,
                capture_output=True,
                text=True,
                timeout=timeout,
            )
        except subprocess.TimeoutExpired:
            return False, [f"{oracle_path} : délai dépassé ({timeout}s)"]
        if result.returncode != 0:
            return False, [
                f"{oracle_path} : code retour {result.returncode} — stderr : "
                f"{result.stderr.strip()}"
            ]

        problems = []
        out_path = Path(out_dir)
        for expected_csv in expected_csvs:
            dataset = expected_csv.stem
            produced_csv = out_path / f"{dataset}.csv"
            if not produced_csv.is_file():
                problems.append(f"{case_dir} : oracle n'a pas écrit {dataset}.csv")
                continue
            expected_text = expected_csv.read_text(encoding="utf-8")
            produced_text = produced_csv.read_text(encoding="utf-8")
            for diff in compare_csv(expected_text, produced_text, tolerance):
                problems.append(f"{case_dir} : {dataset} : {diff}")
        return (not problems), problems


def validate(root, timeout=ORACLE_TIMEOUT):
    """(oracles découverts, lignes de rapport, problèmes) pour tout `root`."""
    oracles = discover_oracles(root)
    lines = []
    problems = []
    for oracle_path in oracles:
        case_dir = oracle_path.parent.parent
        ok, case_problems = run_oracle_case(case_dir, oracle_path, timeout)
        lines.append(f"  {'PASS' if ok else 'FAIL'}  {case_dir}")
        problems.extend(case_problems)
    return oracles, lines, problems


# ── Auto-test ────────────────────────────────────────────────────────────


def self_test():
    """Reproducers sur des cas fabriqués : conforme, valeur divergente,
    import interdit (jamais exécuté)."""
    failures = []

    def expect(label, got, want):
        if got != want:
            failures.append(f"{label} : {got!r}, attendu {want!r}")
        else:
            print(f"replay_oracles.py: self-test OK — {label}")

    with tempfile.TemporaryDirectory(prefix="replay-oracles-selftest-") as tmp:
        root = Path(tmp)

        def make_case(name, oracle_source, expected_csv):
            case_dir = root / name
            (case_dir / "oracle").mkdir(parents=True)
            (case_dir / "expected").mkdir(parents=True)
            (case_dir / "oracle" / "oracle.py").write_text(oracle_source, encoding="utf-8")
            (case_dir / "expected" / "result.csv").write_text(expected_csv, encoding="utf-8")
            (case_dir / "case.json").write_text(json.dumps({"id": name}), encoding="utf-8")
            return case_dir

        conforming_source = (
            "import sys\n"
            "from pathlib import Path\n"
            "out = Path(sys.argv[1])\n"
            "out.joinpath('result.csv').write_text('A,B\\n1,2\\n3,4\\n', encoding='utf-8')\n"
        )
        conforming_dir = make_case("conforming", conforming_source, "A,B\n1,2\n3,4\n")

        divergent_source = conforming_source.replace("3,4", "3,999")
        divergent_dir = make_case("divergent", divergent_source, "A,B\n1,2\n3,4\n")

        forbidden_source = (
            "import socket\n"
            "import sys\n"
            "from pathlib import Path\n"
            "Path('executed.marker').write_text('ran', encoding='utf-8')\n"
            "out = Path(sys.argv[1])\n"
            "out.joinpath('result.csv').write_text('A,B\\n1,2\\n3,4\\n', encoding='utf-8')\n"
        )
        forbidden_dir = make_case("forbidden", forbidden_source, "A,B\n1,2\n3,4\n")

        ok, problems = run_oracle_case(conforming_dir, conforming_dir / "oracle" / "oracle.py")
        expect("cas conforme accepté", ok, True)
        if problems:
            failures.append(f"cas conforme : problèmes inattendus {problems}")

        ok, problems = run_oracle_case(divergent_dir, divergent_dir / "oracle" / "oracle.py")
        expect("valeur divergente détectée", ok, False)
        expect(
            "valeur divergente : message explicite",
            any("produit" in p and "attendu" in p for p in problems),
            True,
        )

        ok, problems = run_oracle_case(forbidden_dir, forbidden_dir / "oracle" / "oracle.py")
        expect("import interdit refusé", ok, False)
        expect(
            "import interdit : message explicite",
            any("socket" in p for p in problems),
            True,
        )
        expect(
            "import interdit : script jamais exécuté",
            (forbidden_dir / "executed.marker").exists(),
            False,
        )

        found = discover_oracles(root)
        expect(
            "découverte récursive de oracle/oracle.py",
            {path.parent.parent.name for path in found},
            {"conforming", "divergent", "forbidden"},
        )

        empty_root = root / "empty"
        empty_root.mkdir()
        oracles, _lines, empty_problems = validate(empty_root)
        expect("0 oracle détecté", len(oracles), 0)
        expect("0 oracle sans problème", empty_problems, [])

    if failures:
        for failure in failures:
            print(f"replay_oracles.py: self-test ÉCHEC — {failure}", file=sys.stderr)
        return 1
    return 0


def main():
    parser = argparse.ArgumentParser(
        description="Rejouer les oracles indépendants (conformance/cases/**/oracle/oracle.py)."
    )
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()

    oracles, lines, problems = validate(CASES_DIR)
    if lines:
        print("Rejeu des oracles indépendants :")
        print("\n".join(lines))
    if problems:
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        print(
            f"replay_oracles.py: ÉCHEC — {len(problems)} problème(s) sur "
            f"{len(oracles)} oracle(s).",
            file=sys.stderr,
        )
        return 1
    print(f"replay_oracles.py: {len(oracles)} oracle(s) rejoué(s).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
