//! build.rs — embarque le commit git et les features actives du build.
//!
//! `sasrs --version` doit rendre `sasrs <version> (commit <sha|unknown>,
//! features: …)`. Le conteneur de build (distrobox, image sans git ni
//! métadonnées `.git`) doit rester supporté : sans git, le commit vaut
//! `unknown`, jamais un échec de build.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let commit = git_commit().unwrap_or_else(|| "unknown".to_string());
    let features = active_features();

    println!("cargo:rustc-env=SASRS_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=SASRS_BUILD_FEATURES={features}");

    // Le commit embarqué doit suivre les mouvements de HEAD : re-exécuter le
    // script quand `.git/HEAD` change (checkout, commit, rebase). En worktree
    // git, HEAD est un fichier texte « ref: refs/heads/… » qui change à chaque
    // mouvement — suffisant comme déclencheur, sans recompiler à l'excès.
    if let Some(head) = git_head_path() {
        println!("cargo:rerun-if-changed={}", head.display());
    }
}

/// SHA-1 complet du commit courant (`git rev-parse HEAD`), ou `None` si git
/// est absent, si le répertoire n'est pas un dépôt, ou si la sortie n'est pas
/// un sha plausible.
fn git_commit() -> Option<String> {
    let dir = env::var("CARGO_MANIFEST_DIR").ok()?;
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8(output.stdout).ok()?.trim().to_string();
    let plausible = sha.len() >= 7
        && sha.len() <= 40
        && sha
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
    plausible.then_some(sha)
}

/// Chemin vers `.git/HEAD` du dépôt courant, s'il est directement lisible
/// (dépôt simple ; ignoré sinon — les worktrees git pointent via un fichier).
fn git_head_path() -> Option<PathBuf> {
    let dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").ok()?);
    let head = dir.join(".git").join("HEAD");
    head.is_file().then_some(head)
}

/// Liste triée des features actives du build, lues des variables
/// `CARGO_FEATURE_<NOM>` posées par cargo (dont `DEFAULT`). Rendues au format
/// du manifeste (`fault-injection`, pas `FAULT_INJECTION`). Build sans
/// feature optionnelle → `default`.
fn active_features() -> String {
    let mut features: Vec<String> = env::vars()
        .filter_map(|(key, value)| {
            if value != "1" {
                return None;
            }
            key.strip_prefix("CARGO_FEATURE_")
                .map(|name| name.to_ascii_lowercase().replace('_', "-"))
        })
        .collect();
    features.sort();
    features.dedup();
    let joined = features.join(",");
    if joined.is_empty() {
        "default".to_string()
    } else {
        joined
    }
}
