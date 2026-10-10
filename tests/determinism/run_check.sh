#!/usr/bin/env bash
# Check j03-determinism (issue #20) : le même programme SAS exécuté 5 fois
# (binaire sasrs reconstruit via cargo, dans des répertoires temporaires
# distincts) doit produire des artefacts STRICTEMENT identiques byte-à-byte :
#   - la log (--log), le listing (--print),
#   - la table Parquet publiée ET son sidecar .sasmeta.json
#     (clés sérialisées triées depuis l'issue #20).
# Exit 0 seulement si TOUS les artefacts de tous les runs sont identiques.
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
SCRIPT="$HERE/reproducer.sas"
RUNS=5
ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT

CARGO_ENV='export CI=true INSTA_UPDATE=no SCCACHE_SERVER_PORT=8231 SCCACHE_DIR=/home/deck/.cache/sasrs-sccache'
DEV_CONTAINER="${SASRS_DEV_CONTAINER:-dev}"

# Si on tourne déjà dans un conteneur, exécute les commandes directement ;
# sinon, passe par distrobox pour atteindre le conteneur de dev.
if [ -f /run/.containerenv ] || [ -n "${CONTAINER_ID:-}" ]; then
    run_in_container() { bash -lc "$1"; }
else
    run_in_container() { distrobox enter "$DEV_CONTAINER" -- bash -lc "$1"; }
fi

# Pré-compile une fois (les 5 runs doivent tester le MÊME binaire).
run_in_container "$CARGO_ENV; cd '$REPO' && cargo build --locked -p sasrs" \
    || { echo "FAIL: cargo build sasrs"; exit 1; }
BIN="$REPO/target/debug/sasrs"

declare -a DIRS
for i in $(seq 1 "$RUNS"); do
    d="$ROOT/run$i"
    mkdir -p "$d/out"
    DIRS+=("$d")
    cp "$SCRIPT" "$d/reproducer.sas"
    run_in_container "cd '$d' && '$BIN' ./reproducer.sas \
        --work work --log run.log --print run.lst --deterministic" \
        > /dev/null 2>&1 \
        || { echo "FAIL: run $i a échoué (code retour $? )"; exit 1; }
done

# Artefacts comparés pour chaque run : log, print, table parquet + sidecar.
ARTIFACTS=("out/unioned.parquet" "out/unioned.parquet.sasmeta.json")

rc=0
for base in run.log run.lst "${ARTIFACTS[@]}"; do
    ref="$ROOT/run1/$base"
    if [ ! -f "$ref" ]; then
        echo "FAIL: artefact attendu absent du run 1 : $base"
        rc=1
        continue
    fi
    for i in $(seq 2 "$RUNS"); do
        if ! cmp -s "$ref" "$ROOT/run$i/$base"; then
            echo "FAIL: $base diffère entre run1 et run$i"
            cmp "$ref" "$ROOT/run$i/$base" | head -3
            rc=1
        fi
    done
done

if [ "$rc" -eq 0 ]; then
    echo "OK: $RUNS exécutions, tous les artefacts identiques byte-à-byte"
fi
exit "$rc"
