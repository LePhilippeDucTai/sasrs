#!/usr/bin/env python3
# conformance_require.py — exige qu'une liste de cas du corpus de conformité
# existe avec le statut attendu (roadmap-avancee J01-P1).
#
# Outil d'acceptation : une unité qui promet « le cas X est validé » (ou
# « documenté comme divergence ») le prouve mécaniquement, sans relire le
# case.json à la main.
#
# Bibliothèque standard uniquement (aucune dépendance).
#
# Usage :
#   python3 scripts/conformance_require.py --status validated CAS…
#   python3 scripts/conformance_require.py --status known-divergence CAS…
#   python3 scripts/conformance_require.py --status any CAS…
#   python3 scripts/conformance_require.py --self-test
#
# CAS = chemin du répertoire du cas relatif à conformance/cases (p. ex.
# `compat/means/means-class-missing-nway` ; le préfixe `conformance/cases/`
# est toléré). Pour chaque cas sont exigés :
#   - le répertoire et son case.json (JSON valide, id = nom du répertoire) ;
#   - le statut demandé (`any` = validated ou known-divergence) ;
#   - provenance.kind et provenance.source non vides ;
#   - un champ issue non vide si le statut est known-divergence ;
#   - au moins un expected/*.csv.
#
# Intégrité (J01-P8) :
#   python3 scripts/conformance_require.py --verify-manifest MANIFESTE…
# vérifie un manifeste au format `sha256sum` (p. ex.
# conformance/cases/compat/ORACLE.sha256 : `<sha256>  <chemin>` par ligne,
# chemins relatifs au répertoire du manifeste) : chaque fichier listé doit
# exister sous ce répertoire et avoir exactement l'empreinte figée. Les
# attendus (expected/, data/, program.sas) d'un cas épinglé ne peuvent donc
# pas changer sans que le manifeste — donc la revue — le montre.
#
# Codes retour : 0 = toutes les exigences tenues ; 1 = au moins une exigence
# violée ; 2 = erreur d'usage (arguments invalides, corpus ou manifeste
# introuvable).

import argparse
import hashlib
import json
import re
import shutil
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CASES_DIR = REPO_ROOT / "conformance" / "cases"

STATUSES = ("validated", "known-divergence")
CASES_PREFIX = "conformance/cases/"


def normalize(case_ref):
    """Chemin relatif sous conformance/cases, ou None s'il sort du corpus."""
    ref = case_ref.strip().replace("\\", "/").strip("/")
    if ref.startswith(CASES_PREFIX):
        ref = ref[len(CASES_PREFIX):]
    parts = [part for part in ref.split("/") if part not in ("", ".")]
    if not parts or ".." in parts:
        return None
    return "/".join(parts)


def check_case(cases_dir, case_ref, wanted):
    """Problèmes (liste vide = exigences tenues) pour un cas."""
    rel = normalize(case_ref)
    if rel is None:
        return [f"{case_ref} : chemin invalide (attendu : chemin sous conformance/cases)"]
    case_dir = cases_dir / rel
    case_json = case_dir / "case.json"
    if not case_dir.is_dir():
        return [f"{rel} : répertoire du cas absent ({case_dir})"]
    if not case_json.is_file():
        return [f"{rel} : case.json absent"]
    try:
        data = json.loads(case_json.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, ValueError) as error:
        return [f"{rel} : case.json illisible ({error})"]
    if not isinstance(data, dict):
        return [f"{rel} : case.json n'est pas un objet JSON"]

    problems = []
    if data.get("id") != case_dir.name:
        problems.append(
            f"{rel} : id « {data.get('id')} » ≠ nom du répertoire « {case_dir.name} »"
        )
    status = data.get("status")
    if status not in STATUSES:
        problems.append(f"{rel} : statut inconnu « {status} »")
    elif wanted != "any" and status != wanted:
        problems.append(f"{rel} : statut « {status} », attendu « {wanted} »")
    provenance = data.get("provenance")
    if not isinstance(provenance, dict):
        provenance = {}
    for key in ("kind", "source"):
        value = provenance.get(key)
        if not isinstance(value, str) or not value.strip():
            problems.append(f"{rel} : provenance.{key} absent ou vide")
    if status == "known-divergence":
        issue = data.get("issue")
        if not isinstance(issue, str) or not issue.strip():
            problems.append(f"{rel} : known-divergence sans « issue »")
    expected_dir = case_dir / "expected"
    if not expected_dir.is_dir() or not any(expected_dir.glob("*.csv")):
        problems.append(f"{rel} : aucun expected/*.csv")
    return problems


def require(cases_dir, wanted, case_refs):
    """(code retour, problèmes) pour une liste de cas."""
    if not cases_dir.is_dir():
        return 2, [f"corpus introuvable : {cases_dir}"]
    problems = []
    for case_ref in case_refs:
        problems.extend(check_case(cases_dir, case_ref, wanted))
    return (1 if problems else 0), problems


def run(wanted, case_refs):
    code, problems = require(CASES_DIR, wanted, case_refs)
    if code == 0:
        print(
            f"conformance_require.py: OK — {len(case_refs)} cas au statut "
            f"« {wanted} »."
        )
        return 0
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    print(
        f"conformance_require.py: ÉCHEC — {len(problems)} exigence(s) violée(s).",
        file=sys.stderr,
    )
    return code


MANIFEST_LINE_RE = re.compile(r"^([0-9a-f]{64}) [ *](.+)$")


def verify_manifest(manifest_path):
    """(code retour, problèmes, nombre de fichiers vérifiés) d'un manifeste
    `sha256sum` ; chemins relatifs à son répertoire, sans `..` ni absolu."""
    if not manifest_path.is_file():
        return 2, [f"manifeste introuvable : {manifest_path}"], 0
    base = manifest_path.parent.resolve()
    try:
        lines = manifest_path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeError) as error:
        return 2, [f"manifeste illisible : {manifest_path} ({error})"], 0
    problems = []
    checked = 0
    for number, line in enumerate(lines, start=1):
        if not line.strip():
            continue
        at = f"{manifest_path.name}:{number}"
        match = MANIFEST_LINE_RE.match(line)
        if not match:
            problems.append(f"{at} : ligne mal formée « {line} »")
            continue
        digest, rel = match.groups()
        parts = rel.replace("\\", "/").split("/")
        if rel.startswith("/") or ".." in parts:
            problems.append(f"{at} : chemin hors du manifeste « {rel} »")
            continue
        target = base / rel
        if not target.is_file():
            problems.append(f"{at} : fichier épinglé absent « {rel} »")
            continue
        actual = hashlib.sha256(target.read_bytes()).hexdigest()
        if actual != digest:
            problems.append(
                f"{at} : empreinte modifiée « {rel} » (sha256 {actual}, "
                f"épinglé {digest})"
            )
            continue
        checked += 1
    if checked == 0 and not problems:
        problems.append(f"{manifest_path} : manifeste vide")
    return (1 if problems else 0), problems, checked


def run_manifests(manifests):
    code = 0
    for manifest in manifests:
        path = Path(manifest)
        if not path.is_absolute():
            path = REPO_ROOT / path
        manifest_code, problems, checked = verify_manifest(path)
        if manifest_code == 0:
            print(
                f"conformance_require.py: OK — {manifest} : {checked} fichier(s) "
                "conforme(s) à leur empreinte."
            )
            continue
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        print(
            f"conformance_require.py: ÉCHEC — {manifest} : {len(problems)} "
            "problème(s) d'intégrité.",
            file=sys.stderr,
        )
        code = max(code, manifest_code)
    return code


def self_test_manifest():
    """Reproducers d'intégrité sur une copie de conformance/cases/compat."""
    failures = []

    def expect(label, got, want):
        if got != want:
            failures.append(f"{label} : code {got}, attendu {want}")
        else:
            print(f"conformance_require.py: self-test OK — {label} (code {got}).")

    source = CASES_DIR / "compat"
    if not (source / "ORACLE.sha256").is_file():
        return [f"manifeste de référence absent : {source / 'ORACLE.sha256'}"]
    with tempfile.TemporaryDirectory(prefix="conformance-manifest-") as directory:
        copy = Path(directory, "compat")
        shutil.copytree(source, copy)
        manifest = copy / "ORACLE.sha256"
        original = manifest.read_text(encoding="utf-8")
        first_rel = MANIFEST_LINE_RE.match(original.splitlines()[0]).group(2)
        pinned = copy / first_rel
        pinned_bytes = pinned.read_bytes()

        expect("manifeste intact", verify_manifest(manifest)[0], 0)

        pinned.write_bytes(pinned_bytes + b"\n")
        expect("fichier épinglé modifié refusé", verify_manifest(manifest)[0], 1)
        pinned.unlink()
        expect("fichier épinglé absent refusé", verify_manifest(manifest)[0], 1)
        pinned.write_bytes(pinned_bytes)
        expect("fichier épinglé restauré", verify_manifest(manifest)[0], 0)

        manifest.write_text(original + "pas une empreinte\n", encoding="utf-8")
        expect("ligne mal formée refusée", verify_manifest(manifest)[0], 1)
        manifest.write_text(
            original + f"{'0' * 64}  ../outside.csv\n", encoding="utf-8"
        )
        expect("chemin hors du manifeste refusé", verify_manifest(manifest)[0], 1)
        manifest.write_text("\n", encoding="utf-8")
        expect("manifeste vide refusé", verify_manifest(manifest)[0], 1)
        manifest.unlink()
        expect("manifeste absent", verify_manifest(manifest)[0], 2)
    return failures


def self_test():
    """Reproducers sur des copies temporaires d'un cas réel de chaque statut."""
    failures = []

    def expect(label, got, want):
        if got != want:
            failures.append(f"{label} : code {got}, attendu {want}")
        else:
            print(f"conformance_require.py: self-test OK — {label} (code {got}).")

    def pick(status):
        for case_json in sorted(CASES_DIR.rglob("case.json")):
            try:
                data = json.loads(case_json.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                continue
            if data.get("status") == status:
                return case_json.parent
        return None

    with tempfile.TemporaryDirectory(prefix="conformance-require-") as directory:
        cases = Path(directory, "cases")
        sources = {"validated": pick("validated"), "known-divergence": pick("known-divergence")}
        refs = {}
        for status, source in sources.items():
            if source is None:
                # Corpus sans cas de ce statut : fabriquer la copie depuis un
                # cas validé, statut réécrit (le test porte sur l'outil).
                source = sources["validated"]
            if source is None:
                print(
                    "conformance_require.py: self-test ÉCHEC — aucun cas "
                    f"exploitable sous {CASES_DIR}.",
                    file=sys.stderr,
                )
                return 1
            ref = f"probe/{status}/probe-{status}"
            target = cases / ref
            shutil.copytree(source, target)
            case_json = target / "case.json"
            data = json.loads(case_json.read_text(encoding="utf-8"))
            data["id"] = target.name
            data["status"] = status
            if status == "known-divergence":
                data["issue"] = data.get("issue") or "self-test"
            case_json.write_text(json.dumps(data), encoding="utf-8")
            refs[status] = ref

        def mutate(ref, change):
            case_json = cases / ref / "case.json"
            data = json.loads(case_json.read_text(encoding="utf-8"))
            change(data)
            case_json.write_text(json.dumps(data), encoding="utf-8")

        valid, divergent = refs["validated"], refs["known-divergence"]
        expect("validated conforme", require(cases, "validated", [valid])[0], 0)
        expect(
            "préfixe conformance/cases/ toléré",
            require(cases, "validated", [CASES_PREFIX + valid])[0],
            0,
        )
        expect(
            "known-divergence conforme",
            require(cases, "known-divergence", [divergent])[0],
            0,
        )
        expect("any accepte les deux", require(cases, "any", [valid, divergent])[0], 0)
        expect(
            "statut inattendu refusé",
            require(cases, "validated", [valid, divergent])[0],
            1,
        )
        expect("cas absent refusé", require(cases, "any", ["probe/absent/x"])[0], 1)
        expect("chemin hors corpus refusé", require(cases, "any", ["../cases"])[0], 1)
        expect("corpus introuvable", require(cases / "nope", "any", [valid])[0], 2)

        mutate(divergent, lambda data: data.pop("issue", None))
        expect(
            "known-divergence sans issue refusé",
            require(cases, "known-divergence", [divergent])[0],
            1,
        )
        mutate(divergent, lambda data: data.update(issue="self-test"))
        expect(
            "issue restaurée",
            require(cases, "known-divergence", [divergent])[0],
            0,
        )

        mutate(valid, lambda data: data["provenance"].update(source="  "))
        expect("provenance.source vide refusée", require(cases, "validated", [valid])[0], 1)
        mutate(valid, lambda data: data.update(provenance={"source": "x"}))
        expect("provenance.kind absent refusé", require(cases, "validated", [valid])[0], 1)
        mutate(valid, lambda data: data.update(provenance={"kind": "sas-doc", "source": "x"}))
        expect("provenance restaurée", require(cases, "validated", [valid])[0], 0)

        mutate(valid, lambda data: data.update(id="autre-id"))
        expect("id ≠ répertoire refusé", require(cases, "validated", [valid])[0], 1)
        mutate(valid, lambda data: data.update(id=Path(valid).name))

        for csv in (cases / valid / "expected").glob("*.csv"):
            csv.rename(csv.with_suffix(".bak"))
        expect("aucun expected/*.csv refusé", require(cases, "validated", [valid])[0], 1)

        (cases / divergent / "case.json").write_text("{", encoding="utf-8")
        expect("case.json illisible refusé", require(cases, "any", [divergent])[0], 1)

    failures.extend(self_test_manifest())
    if failures:
        for failure in failures:
            print(f"conformance_require.py: self-test ÉCHEC — {failure}", file=sys.stderr)
        return 1
    return 0


def main():
    parser = argparse.ArgumentParser(
        description="Exiger des cas du corpus de conformité au statut donné."
    )
    parser.add_argument("--status", choices=(*STATUSES, "any"))
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument(
        "--verify-manifest",
        nargs="+",
        metavar="MANIFESTE",
        help="vérifier des manifestes sha256sum (p. ex. conformance/cases/compat/ORACLE.sha256)",
    )
    parser.add_argument("cases", nargs="*", metavar="CAS")
    args = parser.parse_args()
    if args.self_test:
        if args.status or args.cases or args.verify_manifest:
            parser.error("--self-test ne prend ni --status, ni CAS, ni --verify-manifest")
        return self_test()
    if args.verify_manifest:
        if args.status or args.cases:
            parser.error("--verify-manifest ne prend ni --status ni CAS")
        return run_manifests(args.verify_manifest)
    if not args.status or not args.cases:
        parser.error("--status et au moins un CAS sont requis")
    return run(args.status, args.cases)


if __name__ == "__main__":
    sys.exit(main())
