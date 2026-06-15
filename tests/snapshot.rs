//! Harnais de snapshots : chaque fixture .sas est exécutée en mode
//! déterministe et le couple log + listing est verrouillé par insta.
//!
//! RÉACTIVER (retirer #[ignore]) dès que executor::run_program est
//! implémenté (fin du jalon M1) ; puis `cargo insta review` pour
//! accepter les snapshots initiaux.

mod common;

use sas_interpreter::{RunOptions, run};

#[test]
fn fixtures() {
    insta::glob!("fixtures/**/*.sas", |path| {
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
