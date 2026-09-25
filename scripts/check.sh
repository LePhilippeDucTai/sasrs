#!/usr/bin/env bash
# check.sh — mêmes vérifications que la CI (.github/workflows/ci.yml), en local.
#
# Usage :
#   scripts/check.sh lint    cargo fmt --check, puis cargo clippy --all-targets
#                           -- -D warnings (défaut, --features graphics, --features s3)
#   scripts/check.sh test    cargo test -p sasrs, puis --features graphics, puis
#                           --features s3 --lib
#   scripts/check.sh build   cargo build --features graphics, puis --features s3
#   scripts/check.sh all     lint, puis test, puis build
#
# Même environnement que la CI : CI=true et INSTA_UPDATE=no — insta n'écrit
# jamais de `.snap.new`, un écart de snapshot fait échouer le test. Comme dans
# la CI, échec si les tests salissent l'arbre de travail (`git status
# --porcelain` doit être inchangé). Sort au premier échec (code retour non
# nul).
#
# L'hôte n'a pas d'éditeur de liens C : lancer depuis le conteneur, p. ex. :
#   distrobox enter dataflowrs-dev -- bash -lc \
#     'cd <worktree> && export CARGO_TARGET_DIR=<cache> && scripts/check.sh all'

set -euo pipefail

# Racine du dépôt (le script vit dans scripts/).
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Même environnement que la CI.
export CI="${CI:-true}"
export INSTA_UPDATE="${INSTA_UPDATE:-no}"

usage() {
    cat >&2 <<'EOF'
usage: scripts/check.sh lint|test|build|all

  lint   cargo fmt --check ; cargo clippy --all-targets -- -D warnings
         (défaut, --features graphics, --features s3)
  test   cargo test -p sasrs ; --features graphics ; --features s3 --lib
  build  cargo build --features graphics ; --features s3
  all    lint, puis test, puis build
EOF
    exit 2
}

# Échoue si les tests ont modifié l'arbre de travail (ex. .snap.new laissé
# par insta) — même contrôle que `git status --porcelain` dans la CI.
assert_tree_unchanged() {
    local after
    after="$(git status --porcelain)"
    if [ "$after" != "$1" ]; then
        echo "check.sh: fichiers créés/modifiés pendant les tests (.snap.new oublié ?) :" >&2
        comm -13 <(printf '%s\n' "$1" | sort) <(printf '%s\n' "$after" | sort) >&2 || true
        return 1
    fi
}

run_lint() {
    echo '==> cargo fmt --check'
    cargo fmt --check
    echo '==> cargo clippy --all-targets -- -D warnings (défaut)'
    cargo clippy --locked --all-targets -- -D warnings
    echo '==> cargo clippy --all-targets --features graphics -- -D warnings'
    cargo clippy --locked --all-targets --features graphics -- -D warnings
    echo '==> cargo clippy --all-targets --features s3 -- -D warnings'
    cargo clippy --locked --all-targets --features s3 -- -D warnings
}

run_test() {
    local before
    before="$(git status --porcelain)"
    echo '==> cargo test -p sasrs'
    cargo test --locked -p sasrs
    assert_tree_unchanged "$before"
    echo '==> cargo test -p sasrs --features graphics'
    cargo test --locked -p sasrs --features graphics
    assert_tree_unchanged "$before"
    echo '==> cargo test --features s3 --lib'
    cargo test --locked --features s3 --lib
    assert_tree_unchanged "$before"
}

run_build() {
    echo '==> cargo build --features graphics'
    cargo build --locked --features graphics
    echo '==> cargo build --features s3'
    cargo build --locked --features s3
}

[ "$#" -eq 1 ] || usage
MODE="$1"
case "$MODE" in
lint) run_lint ;;
test) run_test ;;
build) run_build ;;
all)
    run_lint
    run_test
    run_build
    ;;
*) usage ;;
esac

echo "check.sh ${MODE} : OK"
