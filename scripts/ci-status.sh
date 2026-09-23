#!/usr/bin/env bash
# ci-status.sh — état de la CI GitHub Actions (workflow ci.yml) sur le dernier
# commit poussé d'une branche.
#
# Réussit (exit 0) uniquement si le dernier run du workflow ci.yml de la
# branche porte le même headSha que `git rev-parse origin/<branche>` ET s'est
# terminé avec la conclusion « success ». Dans tous les autres cas (run
# rouge, run en cours, run obsolète, aucun run), affiche l'état et échoue.
#
# Codes de retour : 0 = CI verte et à jour ; 1 = pas verte / pas à jour /
# aucun run ; 2 = erreur d'usage ou d'outil.
#
# `git` et `gh` vivent sur l'hôte : ce script ne se lance PAS dans le
# conteneur.

set -euo pipefail

WORKFLOW="ci.yml"
DEFAULT_BRANCH="consolidation"

usage() {
    cat <<'EOF'
Usage: scripts/ci-status.sh [branche]

Vérifie la CI GitHub Actions de `branche` (défaut : consolidation) :
le dernier run du workflow ci.yml doit porter le même headSha que
origin/<branche> et être conclu « success ».

Sorties : exit 0 si la CI du dernier commit poussé est verte, exit 1 sinon
(run rouge, en cours, obsolète ou absent), exit 2 en cas d'erreur d'outil.
EOF
}

BRANCH="${DEFAULT_BRANCH}"
case "${1:-}" in
-h | --help)
    usage
    exit 0
    ;;
"") ;;
*) BRANCH="$1" ;;
esac

for tool in git gh jq; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "ci-status.sh: « ${tool} » n'est pas installé ou pas dans le PATH." >&2
        exit 2
    fi
done

remote_ref="origin/${BRANCH}"
if ! remote_sha="$(git rev-parse --verify "${remote_ref}^{commit}")"; then
    echo "ci-status.sh: référence introuvable : ${remote_ref} (lancer « git fetch origin » ?)." >&2
    exit 2
fi

if ! json="$(gh run list --workflow "${WORKFLOW}" --branch "${BRANCH}" --limit 1 \
    --json headSha,conclusion,status,databaseId,displayTitle,url)"; then
    echo "ci-status.sh: impossible de lister les runs de ${WORKFLOW} sur ${BRANCH} (workflow absent de la branche par défaut du dépôt ?)." >&2
    exit 1
fi

if [ "$(printf '%s' "$json" | jq 'length')" -eq 0 ]; then
    echo "ci-status.sh: aucun run du workflow ${WORKFLOW} sur ${BRANCH}." >&2
    exit 1
fi

head_sha="$(printf '%s' "$json" | jq -r '.[0].headSha // ""')"
run_status="$(printf '%s' "$json" | jq -r '.[0].status // ""')"
conclusion="$(printf '%s' "$json" | jq -r '.[0].conclusion // ""')"
run_url="$(printf '%s' "$json" | jq -r '.[0].url // ""')"

echo "branche           : ${BRANCH}"
echo "origin/${BRANCH}  : ${remote_sha:0:12}"
echo "dernier run       : ${head_sha:0:12} (${run_status}${conclusion:+, conclusion: }${conclusion:-aucune})"
echo "run               : ${run_url}"

if [ "$head_sha" != "$remote_sha" ]; then
    echo "ci-status.sh: ÉCHEC — le dernier run de ${BRANCH} ne porte pas le headSha de ${remote_ref}." >&2
    exit 1
fi
if [ "$conclusion" != "success" ]; then
    echo "ci-status.sh: ÉCHEC — la CI du dernier commit poussé n'est pas verte (status: ${run_status}, conclusion: ${conclusion:-<aucune>})." >&2
    exit 1
fi

echo "ci-status.sh: OK — CI verte sur le dernier commit poussé de ${BRANCH}."
exit 0
