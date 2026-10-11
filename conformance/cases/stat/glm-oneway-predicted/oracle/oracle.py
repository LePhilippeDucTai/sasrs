#!/usr/bin/env python3
# Oracle indépendant du cas stat/glm-oneway-predicted (roadmap-avancee J02-P13).
#
# Provenance : SAS/STAT 9.4 User's Guide, « The GLM Procedure », statement
# OUTPUT — P= écrit la valeur prédite, R= le résidu :
# https://documentation.sas.com/doc/en/pgmsascdc/9.4_3.5/statug/statug_glm_syntax.htm
#
# Identité arithmétique (ANOVA à un facteur sans covariable, estimations par
# moindres carrés) : la valeur prédite d'une observation est la moyenne de
# son groupe, le résidu est la valeur observée moins cette moyenne. Les deux
# se recalculent depuis data/drug.csv en arithmétique pure : l'oracle ne
# relit jamais la sortie de référence du cas et n'exécute jamais `sasrs`.
#
# Contrat (conformance/schema.md § oracle/oracle.py, scripts/replay_oracles.py) :
# bibliothèque standard Python uniquement, déterministe, sans réseau ni
# sous-processus ; invoqué comme `python3 -I -B oracle/oracle.py <dir>` avec
# pour répertoire courant un bac à sable ne contenant que des copies de
# data/ et oracle/ du cas ; écrit <dir>/pred.csv (colonnes drug, absorb,
# phat, resid) au format des jeux de données du corpus.

import sys
from pathlib import Path

INPUT = Path("data") / "drug.csv"
OUTPUT_NAME = "pred.csv"
HEADER = ("drug", "absorb", "phat", "resid")


def read_observations():
    """(groupe, valeur) de data/drug.csv, dans l'ordre du fichier."""
    lines = [
        line
        for line in INPUT.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    header = [cell.strip().lower() for cell in lines[0].split(",")]
    if header[:2] != ["drug", "absorb"]:
        raise SystemExit(
            f"en-tête inattendu {lines[0]!r} : colonnes drug, absorb attendues"
        )
    observations = []
    for line in lines[1:]:
        cells = [cell.strip() for cell in line.split(",")]
        observations.append((cells[0], float(cells[1])))
    return observations


def group_means(observations):
    """Moyenne de absorb par groupe (drug), sommes et divisions exactes."""
    sums = {}
    counts = {}
    for group, value in observations:
        sums[group] = sums.get(group, 0.0) + value
        counts[group] = counts.get(group, 0) + 1
    return {group: sums[group] / counts[group] for group in sums}


def fmt(value):
    """Représentation décimale stable d'un flottant (repr Python)."""
    return repr(float(value))


def main(argv):
    if len(argv) != 2:
        print("usage : oracle.py <repertoire-de-sortie>", file=sys.stderr)
        return 2
    observations = read_observations()
    means = group_means(observations)
    rows = [",".join(HEADER)]
    for group, value in observations:
        predicted = means[group]
        residual = value - predicted
        rows.append(f"{group},{fmt(value)},{fmt(predicted)},{fmt(residual)}")
    (Path(argv[1]) / OUTPUT_NAME).write_text("\n".join(rows) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
