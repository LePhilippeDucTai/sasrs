//! Harnais de snapshots : chaque fixture .sas est exécutée en mode
//! déterministe et le couple log + listing est verrouillé par insta.
//!
//! Ajouter une fixture : déposer le `.sas` sous `tests/fixtures/<jalon>/`,
//! lancer `cargo test`, VÉRIFIER À LA MAIN la plausibilité SAS du `.snap.new`
//! produit, puis `cargo insta accept`.

mod common;

use sasrs::{RunOptions, run};

/// Fixtures qui MATÉRIALISENT de vraies images sous `--features graphics` :
/// leur log diverge alors du snapshot capturé pour le build PAR DÉFAUT
/// (NOTE « image deferred » / « Output '...' written »). Le `.snap` verrouille
/// le build par défaut ; la génération réelle d'image est couverte par les
/// tests unitaires des procs concernées (`src/procs/sgplot/tests.rs`,
/// `src/procs/gplot/tests.rs`, `src/procs/reg/tests/m3610.rs`, …). Ces
/// fixtures sont donc sautées UNIQUEMENT quand la feature graphics est
/// active — toutes les autres continuent de tourner et vérifient l'invariant
/// byte-identique du build graphics.
///
/// Liste explicite des chemins relatifs à ce fichier — la maintenir à jour
/// quand une fixture se met à écrire des images (ou cesse d'en écrire) :
/// - `fixtures/m29/sgplot_basic.sas`        — SGPLOT après `ods graphics on`.
/// - `fixtures/m29/sgplot_univar_reg.sas`   — UNIVARIATE histogram/qqplot + diagnostic REG, sous ODS ON.
/// - `fixtures/m30/gplot_gchart.sas`        — GPLOT + GCHART après `ods graphics on`.
/// - `fixtures/m30/proc_plot.sas`           — PROC PLOT après `ods graphics on`.
/// - `fixtures/m33/univariate_weighted.sas` — UNIVARIATE PROBPLOT sous ODS ON.
/// - `fixtures/m36/plots.sas`               — REG PLOTS=(…) + PLOT (rendus même sans ODS ON).
/// - `fixtures/m45/univariate_normal_plot.sas` — UNIVARIATE HISTOGRAM/QQPLOT/CDFPLOT, section sous ODS ON.
#[cfg(feature = "graphics")]
const IMAGE_FIXTURES: &[&str] = &[
    "fixtures/m29/sgplot_basic.sas",
    "fixtures/m29/sgplot_univar_reg.sas",
    "fixtures/m30/gplot_gchart.sas",
    "fixtures/m30/proc_plot.sas",
    "fixtures/m33/univariate_weighted.sas",
    "fixtures/m36/plots.sas",
    "fixtures/m45/univariate_normal_plot.sas",
];

#[test]
fn fixtures() {
    insta::glob!("fixtures/**/*.sas", |path| {
        #[cfg(feature = "graphics")]
        if IMAGE_FIXTURES.iter().any(|f| path.ends_with(f)) {
            return;
        }
        let source = std::fs::read_to_string(path).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        common::write_class_parquet(&tmp.path().join("data"));
        common::write_pets_csv(&tmp.path().join("data"));

        let outcome = run(
            &source,
            RunOptions {
                work_dir: None,
                base_dir: Some(tmp.path().to_path_buf()),
                deterministic: true,
                vectorize: false,
            },
        );
        insta::assert_snapshot!(format!(
            "==== LOG ====\n{}\n==== LISTING ====\n{}\n==== EXIT {} ====",
            outcome.log, outcome.listing, outcome.exit_code
        ));
    });
}
