#!/usr/bin/env python3
"""Oracle indépendant pour les statistiques pondérées (J03-P1).

Référence : documentation SAS 9.4 PROC MEANS / PROC UNIVARIATE.
  - PROC MEANS, WEIGHT statement (règles de sélection des observations,
    VARDEF=DF/WEIGHT, SUMWGT) :
    https://documentation.sas.com/doc/en/pgmsascdc/9.4_3.2/proc/p1ays1la1f3e2tn1m8owq8h9n1df.htm
  - PROC UNIVARIATE, PROC statement (option EXCLNPWGT : exclure les poids
    nuls, négatifs ou manquants) :
    https://documentation.sas.com/doc/en/pgmsascdc/9.4_3.2/proc/p0vawoeemeemvn13cxap99m1z3th.htm
  - Dictionary of formulas for PROC MEANS statistical keywords (formules
    des statistiques pondérées MEAN/VAR/STD/SUMWGT) :
    https://documentation.sas.com/doc/en/pgmsascdc/9.4_3.2/proc/n1y2f6nudl7zfjn1joclatu2h3zh.htm
  - Quantile definitions (PCTLDEF/QNTLDEF=5, « empirical distribution
    function with averaging ») :
    https://support.sas.com/documentation/cdl/en/proc/61895/HTML/default/a002473616.htm

Bibliothèque standard uniquement. `python3 tools/oracles/weighted_stats.py
--check` recalcule toutes les statistiques de `tests/oracles/weighted_stats.json`
et compare aux valeurs `expected` (écrites indépendamment, à la main, à partir
des formules documentées ci-dessus) ; exit ≠ 0 en cas d'écart.

Mode « means » (PROC MEANS par défaut, WEIGHT) : une observation est utilisée
si x est non manquant ET le poids est non manquant et strictement positif ;
les poids nuls, négatifs ou manquants excluent l'observation de l'analyse.

Mode « univariate » (PROC UNIVARIATE sans EXCLNPWGT) : les observations avec
poids nul, négatif ou manquant et x non manquant restent comptées dans N,
mais leur contribution pondérée est nulle (leur poids compte pour 0 dans les
formules pondérées et dans SUMWGT).

Mode « univariate-exclnpwgt » (EXCLNPWGT) : comme « means » — les poids
non strictement positifs ou manquants excluent l'observation.

Quantiles pondérés (definition 5 pondérée, MEDIAN=P50, Q1=P25, Q3=P75,
QRANGE=Q3-Q1) : en triant les observations utilisées par x croissant, avec
cum_k = somme cumulée des poids et W = somme totale des poids, la cible est
t = p*W ; soit j le plus petit indice tel que cum_j >= t :
  - si cum_j == t exactement (et qu'il existe une observation suivante),
    le quantile est la moyenne de x_j et x_(j+1) ;
  - sinon le quantile est x_j.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path

JSON_PATH = Path(__file__).resolve().parents[2] / "tests" / "oracles" / "weighted_stats.json"

TOLERANCE = 1e-9


def _used_pairs(data, mode):
    """Retourne [(x, w_effectif)] des observations contribuant aux stats pondérées."""
    pairs = []
    for x, w in zip(data["x"], data["w"]):
        if x is None:
            continue
        if mode == "univariate":
            w_eff = w if (w is not None and w > 0) else 0.0
            pairs.append((float(x), float(w_eff)))
        else:  # "means" ou "univariate-exclnpwgt" : poids non manquant et > 0
            if w is not None and w > 0:
                pairs.append((float(x), float(w)))
    return pairs


def _nmiss(data):
    return sum(1 for x in data["x"] if x is None)


def _n(data, mode):
    if mode == "univariate":
        return sum(1 for x, w in zip(data["x"], data["w"])
                   if x is not None)
    return sum(1 for x, w in zip(data["x"], data["w"])
               if x is not None and w is not None and w > 0)


def _weighted_quantile(pairs, p):
    """Quantile pondéré, definition 5 pondérée. None si indéfini."""
    if not pairs:
        return None
    total = sum(w for _, w in pairs)
    if total <= 0:
        return None
    target = p * total
    cum = 0.0
    for i, (x, w) in enumerate(pairs):
        cum += w
        if cum >= target:
            if cum == target and i + 1 < len(pairs):
                return (x + pairs[i + 1][0]) / 2.0
            return x
    return pairs[-1][0]


def compute(case):
    data = case["data"]
    mode = case.get("mode", "means")
    vardef = case.get("vardef", "df")
    pairs = _used_pairs(data, mode)
    # Les quantiles pondérés ne portent que sur les observations à poids
    # strictement positif (une observation à poids nul n'y participe pas,
    # même en mode univariate sans EXCLNPWGT).
    sorted_pairs = sorted(
        ((x, w) for x, w in pairs if w > 0), key=lambda pw: pw[0])

    total_w = sum(w for _, w in pairs)
    n = _n(data, mode)
    nmiss = _nmiss(data)
    # En mode univariate par défaut, SUMWGT ne compte que la contribution
    # pondérée effective (les poids <= 0 ou manquants pèsent 0).
    sumwgt = total_w

    mean = None
    var = None
    std = None
    if total_w > 0 and pairs:
        mean = sum(w * x for x, w in pairs) / total_w
        css = sum(w * (x - mean) ** 2 for x, w in pairs)
        if vardef == "df":
            denom = total_w - 1.0
        else:  # VARDEF=WEIGHT
            denom = total_w - sum(w * w for _, w in pairs) / total_w
        # SAS : VAR/STD ne sont calculées que si n >= 2 (observations
        # utilisées), sinon valeur manquante.
        if denom > 0 and n >= 2:
            var = css / denom
            std = math.sqrt(var)

    result = {
        "n": n,
        "nmiss": nmiss,
        "sumwgt": sumwgt,
        "mean": mean,
        "var": var,
        "std": std,
    }
    for key, p in (("median", 0.5), ("q1", 0.25), ("q3", 0.75),
                   ("p1", 0.01), ("p99", 0.99)):
        result[key] = _weighted_quantile(sorted_pairs, p)
    result["qrange"] = (
        result["q3"] - result["q1"]
        if result["q3"] is not None and result["q1"] is not None else None
    )
    return result


def _eq(actual, expected):
    if actual is None or expected is None:
        return actual is None and expected is None
    return abs(actual - expected) <= TOLERANCE * max(1.0, abs(expected))


def check():
    cases = json.loads(JSON_PATH.read_text(encoding="utf-8"))["cases"]
    failures = 0
    for case in cases:
        actual = compute(case)
        expected = case["expected"]
        bad = [(k, actual.get(k), expected.get(k))
               for k in sorted(expected)
               if not _eq(actual.get(k), expected.get(k))]
        if bad:
            failures += 1
            print(f"ECHEC {case['id']} (mode={case.get('mode', 'means')},"
                  f" vardef={case.get('vardef', 'df')})")
            for k, a, e in bad:
                print(f"  {k}: calcule={a!r} attendu={e!r}")
        else:
            print(f"ok     {case['id']}")
    if failures:
        print(f"\n{failures} cas en échec")
        return 1
    print(f"\ntous les {len(cases)} cas sont conformes")
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true",
                        help="recalcule les cas du corpus et compare à expected")
    args = parser.parse_args(argv)
    if not args.check:
        parser.print_help()
        return 2
    return check()


if __name__ == "__main__":
    sys.exit(main())
