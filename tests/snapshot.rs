//! Harnais de snapshots : chaque fixture .sas est exécutée en mode
//! déterministe et le couple log + listing est verrouillé par insta.
//!
//! RÉACTIVER (retirer #[ignore]) dès que executor::run_program est
//! implémenté (fin du jalon M1) ; puis `cargo insta review` pour
//! accepter les snapshots initiaux.

mod common;

use sasrs::{RunOptions, run};

#[test]
fn fixtures() {
    insta::glob!("fixtures/**/*.sas", |path| {
        // Les fixtures PROC SGPLOT (M29.2) matérialisent de vraies images sous
        // `--features graphics` : leur log diverge alors du snapshot capturé
        // pour le build PAR DÉFAUT (NOTE « image deferred »). Le snapshot .snap
        // verrouille le build par défaut ; la génération réelle d'image est
        // couverte par les tests unitaires de `src/procs/sgplot.rs`. On saute
        // donc ces fixtures UNIQUEMENT quand la feature graphics est active —
        // les autres fixtures continuent de tourner pour vérifier l'invariant
        // byte-identique du build graphics.
        #[cfg(feature = "graphics")]
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| {
                n.starts_with("sgplot")
                    || n.starts_with("proc_plot")
                    || n.starts_with("gplot")
                    || n.starts_with("gchart")
                    // PROC UNIVARIATE fixtures also materialise a real image
                    // under `--features graphics` (PROBPLOT/etc.), so the
                    // default-build snapshot — which records the "image deferred"
                    // NOTE — would diverge: skip when graphics is on, like the
                    // plotting procs above.
                    || n.starts_with("univariate")
            })
        {
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
