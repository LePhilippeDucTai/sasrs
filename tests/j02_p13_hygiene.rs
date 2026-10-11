//! J02-P13 — tests d'hygiène de la reprise 17b55b1f (revue J01, plan
//! roadmap-avancee révision 11).
//!
//! Trois livrables de l'attempt précédent (f0619029, commit 7cf92ab2) sont
//! verrouillés ici en tests cargo nommés `ra_j02_p13_*` (le périmètre de la
//! révision 11 ajoute `tests/j02_p13_hygiene.rs` précisément pour cela) :
//!
//! 1. le champ `issue` est réservé aux cas `known-divergence`
//!    (conformance/schema.md) : les trois cas compat **validated** qui le
//!    portaient encore ne doivent plus le porter (les attendus, `data/` et
//!    `program.sas` restent intacts — `ORACLE.sha256` continue de passer) ;
//!
//! 2. la section GLM/ANOVA de `docs/roadmap/avancee.md` décrit l'état du
//!    code : `stat/glm-oneway-predicted` est **validé** depuis la base du
//!    plan (J02-P2, issue sasrs #17, `OUTPUT OUT=` `P=`/`R=` implémenté),
//!    et ne doit plus être annoncée comme « divergence connue » ;
//!
//! 3. ce même cas porte le premier oracle indépendant rejouable du corpus
//!    (`oracle/oracle.py`, bibliothèque standard seulement, recalcule les
//!    moyennes de groupe et résidus depuis `data/`) : `scripts/replay_oracles.py`
//!    doit le rejouer PASS.

use serde_json::Value as Json;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Racine du crate (le dossier qui porte `Cargo.toml`).
fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Les trois cas compat validated dont le champ `issue` périmé a été retiré.
const ISSUE_FIELD_CASES: [&str; 3] = [
    "conformance/cases/compat/compare/compare-outbase-outcomp-id/case.json",
    "conformance/cases/compat/compare/compare-outdif-by-criterion/case.json",
    "conformance/cases/compat/transpose/transpose-copy-suffix/case.json",
];

/// Section `## GLM / ANOVA` du document (du titre au prochain titre `## `).
fn glm_section(doc: &str) -> &str {
    let start = doc
        .find("## GLM / ANOVA")
        .expect("section « ## GLM / ANOVA » absente de docs/roadmap/avancee.md");
    let end = doc[start + 1..]
        .find("\n## ")
        .map(|offset| start + 1 + offset + 1)
        .unwrap_or(doc.len());
    &doc[start..end]
}

/// Livrable 1 — plus aucun champ `issue` sur les trois cas compat validated
/// (le champ décrit une divergence ; ces cas n'en sont pas un).
#[test]
fn ra_j02_p13_issue_fields_removed() {
    for rel in ISSUE_FIELD_CASES {
        let path = repo_root().join(rel);
        let text =
            fs::read_to_string(&path).unwrap_or_else(|error| panic!("{rel} illisible : {error}"));
        let case: Json = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("{rel} n'est pas un JSON valide : {error}"));

        let object = case
            .as_object()
            .unwrap_or_else(|| panic!("{rel} : la racine n'est pas un objet"));
        assert!(
            !object.contains_key("issue"),
            "{rel} porte encore un champ `issue` — réservé aux cas known-divergence \
             (conformance/schema.md) ; ce cas est validated, sans divergence"
        );

        // Le retrait ne doit pas avoir dégradé le cas : même identité, même
        // statut (les attendus sont couverts par ORACLE.sha256, inchangé).
        let id = case["id"].as_str().unwrap_or_default();
        let expected_id = Path::new(rel)
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        assert_eq!(
            id, expected_id,
            "{rel} : id de cas incohérent avec le dossier"
        );
        assert_eq!(
            case["status"].as_str(),
            Some("validated"),
            "{rel} : le cas doit rester validated"
        );
    }
}

/// Livrable 2 — la section GLM/ANOVA de la roadmap décrit l'état réel du
/// code : le cas `glm-oneway-predicted` est validé, plus une divergence.
#[test]
fn ra_j02_p13_glm_doc_state_matches_code() {
    let doc_path = repo_root().join("docs/roadmap/avancee.md");
    let doc = fs::read_to_string(&doc_path)
        .unwrap_or_else(|error| panic!("docs/roadmap/avancee.md illisible : {error}"));
    let section = glm_section(&doc);

    assert!(
        section.contains("glm-oneway-predicted"),
        "la section GLM/ANOVA doit nommer le cas corpus glm-oneway-predicted"
    );
    assert!(
        section.contains("validé"),
        "la section GLM/ANOVA doit décrire glm-oneway-predicted comme validé"
    );
    assert!(
        section.contains("implémenté"),
        "la section GLM/ANOVA doit refléter OUTPUT OUT= P=/R= implémenté (J02-P2)"
    );
    assert!(
        !section.contains("divergence"),
        "la section GLM/ANOVA ne doit plus présenter glm-oneway-predicted \
         comme divergence connue (état périmé : « OUTPUT non supporté »)"
    );
    assert!(
        !section.contains("non supporté"),
        "la section GLM/ANOVA ne doit plus promettre l'absence de OUTPUT"
    );
}

/// Livrable 3 — l'oracle `stat/glm-oneway-predicted` existe, cite sa
/// provenance, et `scripts/replay_oracles.py` le rejoue PASS (le corpus
/// compte exactement 1 oracle à ce jour ; l'assertion accepte davantage
/// pour ne pas briser l'ajout d'oracles futurs).
#[test]
fn ra_j02_p13_oracle_replayable() {
    let oracle_rel = "conformance/cases/stat/glm-oneway-predicted/oracle/oracle.py";
    let oracle_path = repo_root().join(oracle_rel);
    assert!(
        oracle_path.is_file(),
        "{oracle_rel} absent — le cas doit porter son oracle indépendant"
    );

    let source = fs::read_to_string(&oracle_path)
        .unwrap_or_else(|error| panic!("{oracle_rel} illisible : {error}"));
    assert!(
        source.contains("Provenance"),
        "{oracle_rel} doit citer sa provenance (doc SAS/STAT, statement OUTPUT)"
    );
    assert!(
        source.contains("documentation.sas.com"),
        "{oracle_rel} doit référencer la documentation SAS officielle"
    );

    // Rejeu complet : contrôle statique (imports bibliothèque standard
    // seulement, pas de sous-processus) puis exécution en bac à sable sans
    // `expected/`, comparaison aux attendus du cas. Déterministe et rapide
    // (arithmétique pure sur data/drug.csv).
    let output = Command::new("python3")
        .arg("-B")
        .arg("scripts/replay_oracles.py")
        .current_dir(repo_root())
        .output()
        .expect("python3 doit être disponible pour rejouer les oracles");
    let code = output.status.code();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "replay_oracles.py a échoué (code {code:?})\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );

    assert!(
        stdout
            .lines()
            .any(|line| line.contains("PASS") && line.contains("glm-oneway-predicted")),
        "le rejeu ne rapporte pas PASS pour stat/glm-oneway-predicted :\n{stdout}"
    );

    let replayed: usize = stdout
        .lines()
        .filter(|line| {
            line.starts_with("replay_oracles.py:") && line.contains("oracle(s) rejoué(s)")
        })
        .filter_map(|line| {
            line.split_whitespace()
                .find_map(|token| token.parse::<usize>().ok())
        })
        .sum();
    assert!(
        replayed >= 1,
        "replay_oracles.py doit rapporter au moins 1 oracle rejoué, reçu : {replayed}\n{stdout}"
    );
}
