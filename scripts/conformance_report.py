#!/usr/bin/env python3
# conformance_report.py — génère conformance/STATUS.md à partir du corpus
# de conformité (J05-P6).
#
# Source unique de vérité : les fichiers `conformance/cases/**/<id>/case.json`,
# découverts récursivement (`<groupe>/<id>/`, `compat/<proc>/<id>/`…).
# Le rapport agrège, par PROC/zone, les cas validés, les divergences connues
# et la provenance de chaque attendu. Aucune information n'est saisie à la
# main ici : si le contenu change, c'est le corpus qui a changé.
#
# Bibliothèque standard uniquement (aucune dépendance).
#
# Usage :
#   python3 scripts/conformance_report.py            # écrit conformance/STATUS.md
#   python3 scripts/conformance_report.py --check    # exit 1 si STATUS.md est périmé
#   python3 scripts/conformance_report.py --self-test
#
# Codes retour : 0 = OK ; 1 = STATUS.md périmé ou corpus invalide ;
# 2 = zone d'un cas inconnue (ni champ `zone`, ni entrée de ZONE_BY_CASE_ID).
#
# `--check` régénère le rapport en mémoire et le compare au fichier commité :
# toute différence (contenu périmé, édition à la main, cas oublié) échoue.
# C'est le check CI `j05-conformance-status`.

import argparse
import json
import shutil
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CASES_DIR = REPO_ROOT / "conformance" / "cases"
STATUS_PATH = REPO_ROOT / "conformance" / "STATUS.md"

STATUS_SENTINEL_BEGIN = "<!-- conformance_report:begin -->"
STATUS_SENTINEL_END = "<!-- conformance_report:end -->"

# Zone (PROC / zone du langage) couverte par chaque cas : champ `zone` du
# case.json s'il est présent, sinon cette table indexée par l'id. Un cas sans
# l'un ni l'autre est une erreur outillée (exit 2) : la zone doit être
# déclarée explicitement, pas devinée.
ZONE_BY_CASE_ID = {
    # base — langage
    "char-date-functions": "DATA step — fonctions caractère",
    "data-array-impute": "DATA step — tableaux",
    "first-last-groups": "DATA step — FIRST./LAST.",
    "format-value-put": "FORMAT / PUT",
    "freq-chisq-output": "PROC FREQ",
    "freq-tables-out": "PROC FREQ",
    "means-class-output": "PROC MEANS",
    "merge-by-match": "DATA step — MERGE",
    "retain-cumulative": "DATA step — RETAIN",
    "sort-nodupkey": "PROC SORT",
    "sql-join-remerge": "PROC SQL",
    "sql-select-computed": "PROC SQL",
    "transpose-var": "PROC TRANSPOSE",
    "update-master": "DATA step — UPDATE",
    # example — démonstration
    "bmi-calculation": "DATA step — arithmétique",
    "special-missing-flag": "DATA step — missing spéciaux",
    # stat — statistiques
    "corr-pearson-outp": "PROC CORR",
    "corr-spearman-outs": "PROC CORR",
    "freq-fisher-2x2": "PROC FREQ",
    "glm-oneway-predicted": "PROC GLM",
    "logistic-binary-output": "PROC LOGISTIC",
    "npar1way-wilcoxon-out": "PROC NPAR1WAY",
    "reg-simple-lineart": "PROC REG",
    "ttest-twosample-pooled": "PROC TTEST",
    "univariate-moments-output": "PROC UNIVARIATE",
    "univariate-weighted-output": "PROC UNIVARIATE",
}

STATUS_LABEL = {
    "validated": "validé",
    "known-divergence": "divergence connue",
}

PROVENANCE_LABEL = {
    "sas-doc": "documentation SAS publiée",
    "independent-oracle": "oracle indépendant",
    "sas-run": "exécution SAS réelle",
}


class UnknownZoneError(ValueError):
    """Cas sans zone déclarée (ni champ `zone`, ni ZONE_BY_CASE_ID) : exit 2."""


def fail(message, code=1):
    print(f"conformance_report.py: ÉCHEC — {message}", file=sys.stderr)
    return code


def corpus_error_code(error):
    """Corpus illisible : exit 2 si une zone est inconnue, 1 sinon."""
    return 2 if isinstance(error, UnknownZoneError) else 1


def fail_corpus(error):
    return fail(f"corpus illisible : {error}", corpus_error_code(error))


def discover_case_files(cases_dir):
    """Tous les case.json sous cases_dir, à toute profondeur (ordre stable).

    Un répertoire qui contient un case.json est un cas : on ne descend pas
    en dessous (data/, expected/ lui appartiennent).
    """
    found = []

    def walk(directory):
        case_path = directory / "case.json"
        if case_path.is_file():
            found.append(case_path)
            return
        for child in sorted(directory.iterdir()):
            if child.is_dir():
                walk(child)

    walk(cases_dir)
    return sorted(found, key=lambda path: path.relative_to(cases_dir).as_posix())


def load_cases(cases_dir=CASES_DIR):
    """Lire tous les case.json, triés de façon stable par chemin relatif."""
    if not cases_dir.is_dir():
        raise FileNotFoundError(f"corpus introuvable : {cases_dir}")
    cases = []
    seen_ids = {}
    for case_path in discover_case_files(cases_dir):
        case_dir = case_path.parent
        relpath = case_dir.relative_to(cases_dir).as_posix()
        group = case_dir.parent.relative_to(cases_dir).as_posix()
        case_id = case_dir.name
        if case_id in seen_ids:
            raise ValueError(
                f"id « {case_id} » dupliqué : {seen_ids[case_id]} et {relpath}"
            )
        seen_ids[case_id] = relpath
        with case_path.open(encoding="utf-8") as handle:
            data = json.load(handle)
        if data.get("id") != case_id:
            raise ValueError(
                f"{case_path} : id « {data.get('id')} » ≠ répertoire « {case_id} »"
            )
        status = data.get("status")
        if status not in STATUS_LABEL:
            raise ValueError(f"{case_path} : statut inconnu « {status} »")
        provenance = data.get("provenance") or {}
        kind = provenance.get("kind")
        if kind not in PROVENANCE_LABEL:
            raise ValueError(f"{case_path} : provenance.kind inconnu « {kind} »")
        zone = data.get("zone", ZONE_BY_CASE_ID.get(case_id))
        if zone is not None and (not isinstance(zone, str) or not zone.strip()):
            raise ValueError(f"{case_path} : champ « zone » vide ou non textuel")
        if zone is None:
            raise UnknownZoneError(
                f"{case_path} : cas « {case_id} » sans zone — déclarer le champ "
                "« zone » du case.json (ou l'entrée de ZONE_BY_CASE_ID dans "
                "scripts/conformance_report.py)."
            )
        cases.append(
            {
                "group": group,
                "path": relpath,
                "id": case_id,
                "title": data.get("title", ""),
                "status": status,
                "provenance_kind": kind,
                "provenance_source": provenance.get("source", ""),
                "issue": data.get("issue", ""),
                "zone": zone,
            }
        )
    if not cases:
        raise ValueError(f"aucun cas dans {cases_dir}")
    return cases


def group_by_zone(cases):
    zones = {}
    for case in cases:
        zones.setdefault(case["zone"], []).append(case)
    return {zone: zones[zone] for zone in sorted(zones)}


def render_report(cases):
    """Rendre le contenu intégral de conformance/STATUS.md (déterministe)."""
    lines = []
    total = len(cases)
    validated = sum(1 for case in cases if case["status"] == "validated")
    divergent = total - validated
    lines.append(STATUS_SENTINEL_BEGIN)
    lines.append("<!-- Généré par scripts/conformance_report.py à partir des")
    lines.append("     conformance/cases/**/case.json — NE PAS ÉDITER À LA MAIN. -->")
    lines.append("<!-- Régénérer : python3 scripts/conformance_report.py -->")
    lines.append("")
    lines.append("# Statut de conformité sasrs ↔ SAS 9.4")
    lines.append("")
    lines.append(
        f"Corpus : **{total} cas** — {validated} validés, "
        f"{divergent} divergences connues."
    )
    lines.append("")
    lines.append(
        "Un cas `validated` **doit passer** (un échec est une régression) ; un cas "
        "`known-divergence` **doit échouer** (divergence documentée entre `sasrs` "
        "et SAS, cf. la colonne Issue). La définition des statuts et la provenance "
        "des attendus sont détaillées dans [`conformance/README.md`](README.md) ; "
        "c'est ce rapport que la marque « *validated against a reference* » du "
        "[`README.md`](../README.md) désigne."
    )
    lines.append("")
    lines.append("## Vue d'ensemble par zone")
    lines.append("")
    lines.append("| Zone | Cas | Validés | Divergences connues |")
    lines.append("|---|---|---|---|")
    for zone, zone_cases in group_by_zone(cases).items():
        ok = sum(1 for case in zone_cases if case["status"] == "validated")
        lines.append(
            f"| {zone} | {len(zone_cases)} | {ok} | {len(zone_cases) - ok} |"
        )
    lines.append("")
    lines.append("## Détail par zone")
    for zone, zone_cases in group_by_zone(cases).items():
        lines.append("")
        lines.append(f"### {zone}")
        lines.append("")
        lines.append("| Groupe | Cas | Statut | Provenance |")
        lines.append("|---|---|---|---|")
        for case in sorted(zone_cases, key=lambda item: (item["id"], item["path"])):
            lines.append(
                f"| `{case['group']}` | `{case['id']}` | "
                f"{STATUS_LABEL[case['status']]} | "
                f"{PROVENANCE_LABEL[case['provenance_kind']]} |"
            )
        divergences = [c for c in sorted(zone_cases, key=lambda i: i["id"]) if c["status"] == "known-divergence"]
        if divergences:
            lines.append("")
            lines.append("Divergences connues :")
            for case in divergences:
                lines.append(f"- `{case['path']}` — {case['issue']}")
    lines.append("")
    lines.append("## Provenance des attendus")
    lines.append("")
    lines.append(
        "Chaque `case.json` cite sa source sous `provenance` (documentation SAS "
        "publiée, oracle indépendant ou exécution SAS réelle) ; l'attendu n'est "
        "jamais la sortie courante de `sasrs`. Les valeurs d'un cas "
        "`known-divergence` ne sont jamais modifiées par l'implémenteur de la "
        "correction — seul le statut change, avec justification."
    )
    lines.append("")
    lines.append(STATUS_SENTINEL_END)
    lines.append("")
    return "\n".join(lines)


def write_report():
    try:
        content = render_report(load_cases())
    except (OSError, ValueError) as error:
        return fail_corpus(error)
    STATUS_PATH.write_text(content, encoding="utf-8")
    print(
        f"conformance_report.py: OK — {STATUS_PATH.relative_to(REPO_ROOT)} écrit "
        f"({len(load_cases())} cas)."
    )
    return 0


def check_report():
    try:
        expected = render_report(load_cases())
    except (OSError, ValueError) as error:
        return fail_corpus(error)
    if not STATUS_PATH.is_file():
        return fail(f"{STATUS_PATH.relative_to(REPO_ROOT)} absent — lancer "
                    "`python3 scripts/conformance_report.py` et committer.")
    actual = STATUS_PATH.read_text(encoding="utf-8")
    if actual != expected:
        expected_lines = expected.splitlines()
        actual_lines = actual.splitlines()
        diffs = [
            f"  -{expected_lines[index]}"
            for index in range(min(len(expected_lines), len(actual_lines)))
            if expected_lines[index] != actual_lines[index]
        ][:10]
        print(
            "conformance_report.py: ÉCHEC — conformance/STATUS.md est périmé "
            f"({len(expected_lines)} lignes attendues, {len(actual_lines)} commises). "
            "Lancer `python3 scripts/conformance_report.py` et committer le "
            "rapport régénéré.",
            file=sys.stderr,
        )
        for line in diffs:
            print(line, file=sys.stderr)
        return 1
    print(
        f"conformance_report.py: OK — {STATUS_PATH.relative_to(REPO_ROOT)} à jour "
        f"({len(load_cases())} cas)."
    )
    return 0


def self_test():
    """Reproducer sur une copie du corpus : rendu déterministe + --check outillé."""
    with tempfile.TemporaryDirectory(prefix="conformance-report-") as directory:
        first = render_report(load_cases())
        second = render_report(load_cases())
        if first != second:
            print(
                "conformance_report.py: self-test ÉCHEC — rendu non déterministe.",
                file=sys.stderr,
            )
            return 1
        print("conformance_report.py: self-test OK — rendu déterministe.")
        sentinel_count = first.count(STATUS_SENTINEL_BEGIN)
        if sentinel_count != 1 or STATUS_SENTINEL_END not in first:
            print(
                "conformance_report.py: self-test ÉCHEC — sentinelles du rapport "
                f"absentes ou dupliquées ({sentinel_count}).",
                file=sys.stderr,
            )
            return 1
        print("conformance_report.py: self-test OK — sentinelles présentes.")
        cases = load_cases()
        if sum(len(zone) for zone in group_by_zone(cases).values()) != len(cases):
            print(
                "conformance_report.py: self-test ÉCHEC — un cas est perdu "
                "par le regroupement par zone.",
                file=sys.stderr,
            )
            return 1
        print(
            f"conformance_report.py: self-test OK — {len(cases)} cas répartis "
            f"sur {len(group_by_zone(cases))} zones."
        )
        Path(directory, "probe").write_text(first, encoding="utf-8")

        # Copie du corpus : découverte récursive, champ `zone`, zone inconnue.
        copy = Path(directory, "cases")
        shutil.copytree(CASES_DIR, copy)
        nested = [case for case in load_cases(copy) if case["path"].count("/") >= 2]
        if not nested:
            print(
                "conformance_report.py: self-test ÉCHEC — aucun cas de profondeur "
                "≥ 3 découvert (découverte récursive cassée).",
                file=sys.stderr,
            )
            return 1
        print(
            f"conformance_report.py: self-test OK — {len(nested)} cas imbriqués "
            "découverts récursivement."
        )
        probe_dir = copy / "probe-group" / "sub" / "probe-unknown-zone"
        shutil.copytree(copy / nested[0]["path"], probe_dir)
        probe_json = probe_dir / "case.json"
        probe = json.loads(probe_json.read_text(encoding="utf-8"))
        probe["id"] = "probe-unknown-zone"
        probe["zone"] = "Zone sonde"
        probe_json.write_text(json.dumps(probe), encoding="utf-8")
        zones = {case["id"]: case["zone"] for case in load_cases(copy)}
        if zones.get("probe-unknown-zone") != "Zone sonde":
            print(
                "conformance_report.py: self-test ÉCHEC — champ « zone » du "
                "case.json ignoré.",
                file=sys.stderr,
            )
            return 1
        print("conformance_report.py: self-test OK — champ « zone » honoré.")
        del probe["zone"]
        probe_json.write_text(json.dumps(probe), encoding="utf-8")
        try:
            load_cases(copy)
        except UnknownZoneError as error:
            if corpus_error_code(error) != 2:
                print(
                    "conformance_report.py: self-test ÉCHEC — zone inconnue ne "
                    "rend pas exit 2.",
                    file=sys.stderr,
                )
                return 1
        else:
            print(
                "conformance_report.py: self-test ÉCHEC — cas sans zone accepté.",
                file=sys.stderr,
            )
            return 1
        print("conformance_report.py: self-test OK — zone inconnue → exit 2.")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--check", action="store_true", help="vérifier sans écrire")
    group.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if args.check:
        return check_report()
    return write_report()


if __name__ == "__main__":
    sys.exit(main())
