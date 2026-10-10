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
  test   cargo test -p sasrs ; --features graphics ; --features s3 --lib ;
         --features fault-injection --test storage_integrity ;
         --features fault-injection --lib ;
         --test conformance + conformance_report.py --check ;
         --self-test de conformance_report.py et conformance_require.py,
         conformance_require.py --verify-manifest compat/ORACLE.sha256 (J01-P8) ;
         replay_oracles.py --self-test + replay_oracles.py (J01-P5) ;
         --test properties ; --test differential ;
         --test e2e (J08-P6) ; coverage-claims (check_coverage_claims.py) ;
         tests unittest du wrapper Python (python/tests)
  build  cargo build --features graphics ; --features s3
  install cargo install (préfixe jetable) + exemple CLI getting-started
          (J08-P6, même séquence que le job CI « install »)
  all    lint, puis test, puis build, puis install
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
    echo '==> cargo test -p sasrs --features fault-injection --test storage_integrity'
    cargo test --locked -p sasrs --features fault-injection --test storage_integrity
    assert_tree_unchanged "$before"
    echo '==> cargo test -p sasrs --features fault-injection --lib'
    cargo test --locked -p sasrs --features fault-injection --lib
    assert_tree_unchanged "$before"
    # J05-P6 : suites de validation indépendante — mêmes cibles que la CI.
    echo '==> cargo test -p sasrs --test conformance'
    cargo test --locked -p sasrs --test conformance
    assert_tree_unchanged "$before"
    echo '==> python3 scripts/conformance_report.py --check'
    python3 scripts/conformance_report.py --check
    # J01-P8 : auto-tests des outils de conformité + intégrité des attendus
    # épinglés — mêmes steps que le job CI « conformance ».
    echo '==> python3 -B scripts/conformance_report.py --self-test'
    python3 -B scripts/conformance_report.py --self-test
    echo '==> python3 -B scripts/conformance_require.py --self-test'
    python3 -B scripts/conformance_require.py --self-test
    echo '==> python3 -B scripts/conformance_require.py --verify-manifest (compat/ORACLE.sha256)'
    python3 -B scripts/conformance_require.py --verify-manifest conformance/cases/compat/ORACLE.sha256
    # J01-P5 : oracles indépendants committés avec les cas (oracle/oracle.py).
    echo '==> python3 -B scripts/replay_oracles.py --self-test'
    python3 -B scripts/replay_oracles.py --self-test
    echo '==> python3 -B scripts/replay_oracles.py'
    python3 -B scripts/replay_oracles.py
    echo '==> cargo test -p sasrs --test properties'
    cargo test --locked -p sasrs --test properties
    assert_tree_unchanged "$before"
    echo '==> cargo test -p sasrs --test differential'
    cargo test --locked -p sasrs --test differential
    assert_tree_unchanged "$before"
    # J08-P6 : E2E migration (binaire + façade api) et garde des promesses
    # « validated » du README contre le corpus conformance.
    echo '==> cargo test -p sasrs --test e2e'
    cargo test --locked -p sasrs --test e2e
    assert_tree_unchanged "$before"
    echo '==> python3 -B scripts/check_coverage_claims.py --self-test'
    python3 -B scripts/check_coverage_claims.py --self-test
    echo '==> python3 -B scripts/check_coverage_claims.py'
    python3 -B scripts/check_coverage_claims.py
    # J06-P4 : wrapper Python (bibliothèque standard, réseau simulé).
    echo '==> python3 -m unittest discover -s python/tests (wrapper Python)'
    env PYTHONPATH=python/src python3 -B -m unittest discover -s python/tests -v
}

run_build() {
    echo '==> cargo build --features graphics'
    cargo build --locked --features graphics
    echo '==> cargo build --features s3'
    cargo build --locked --features s3
}

# J08-P6 : installation vierge — même séquence que le job CI « install » :
# cargo install dans un préfixe jetable, puis l'exemple CLI référencé par
# docs/getting-started.md (§2 : examples/cli/analysis.sas, exécuté dans un
# répertoire temporaire pour ne rien écrire dans l'arbre du dépôt).
run_install() {
    echo '==> cargo install (préfixe jetable)'
    cargo install --locked --path . --root "$RUNNER_TEMP/sasrs-install"
    echo '==> exemple CLI de docs/getting-started.md (examples/cli/analysis.sas)'
    sandbox="$(mktemp -d)"
    cp -r examples "$sandbox/examples"
    mkdir -p "$sandbox/examples/out"
    (
        cd "$sandbox/examples"
        export PATH="$RUNNER_TEMP/sasrs-install/bin:$PATH"
        sasrs --version
        sasrs cli/analysis.sas --log run.log --print run.lst
        test -s out/summary.csv
        test -s run.log
    )
    rm -rf "$sandbox"
}

RUNNER_TEMP="${RUNNER_TEMP:-$(mktemp -d)}"

[ "$#" -eq 1 ] || usage
MODE="$1"
case "$MODE" in
lint) run_lint ;;
test) run_test ;;
build) run_build ;;
install) run_install ;;
all)
    run_lint
    run_test
    run_build
    run_install
    ;;
*) usage ;;
esac

echo "check.sh ${MODE} : OK"
