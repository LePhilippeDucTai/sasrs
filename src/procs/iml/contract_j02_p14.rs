//! ── J02-P14 : fenêtre OBS=/FIRSTOBS= sur la lecture `USE`/`READ` d'IML ──
//!
//! Base (ancien silence) : `READ ALL VAR … INTO` lisait la table par
//! `provider.read` direct — la table ENTIÈRE entrait dans la matrice, la
//! fenêtre OBS=/FIRSTOBS= en vigueur était ignorée (5 lignes sous
//! `firstobs=2 obs=4`).
//!
//! Oracle : doc SAS 9.4, System Options, OBS= / FIRSTOBS= — les options
//! système valent pour TOUTE lecture d'une table par un PROC :
//! <https://documentation.sas.com/doc/en/pgmsascdc/9.4_3.5/lesysoptsref/p0c2av6mvf2zsrn1b48go7vk6bro.htm>

use crate::RunOptions;
use crate::dataset::SasDataset;

/// Exécute `program` dans un WORK isolé, puis relit la table `name` (parquet).
fn run_and_read(program: &str, name: &str) -> (crate::RunOutcome, SasDataset) {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let out = crate::run(
        program,
        RunOptions {
            work_dir: Some(work.clone()),
            base_dir: Some(tmp.path().to_path_buf()),
            deterministic: true,
            vectorize: false,
        },
    );
    let p = work.join(format!("{name}.parquet"));
    let (ds, _) = SasDataset::read_parquet(&p)
        .unwrap_or_else(|e| panic!("cannot read {} ({e}); log:\n{}", p.display(), out.log));
    (out, ds)
}

fn first_num_col(ds: &SasDataset) -> Vec<Option<f64>> {
    ds.df
        .get_columns()
        .first()
        .unwrap()
        .as_materialized_series()
        .f64()
        .unwrap()
        .into_iter()
        .collect()
}

/// `options firstobs=2 obs=4;` avant `USE`/`READ INTO` : la matrice ne
/// reçoit que les observations 2–4 de la table (fenêtre [2,4]), pas les 5
/// stockées. La table réécrite via CREATE/APPEND fige ce compte.
#[test]
fn ra_j02_p14_iml_obs_window_use_read() {
    let (out, ds) = run_and_read(
        "data work.s; do i = 1 to 5; output; end; run;\n\
         options firstobs=2 obs=4;\n\
         proc iml;\n\
           use work.s;\n\
           read all var {i} into m;\n\
           create work.out from m;\n\
           append from m;\n\
         quit;\n",
        "out",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(!out.log.contains("ERROR:"), "log:\n{}", out.log);
    // NOTE de fermeture IML : la table écrite porte le compte FENÊTRÉ.
    assert!(
        out.log
            .contains("The data set WORK.OUT has 3 observations and 1 variables."),
        "log:\n{}",
        out.log
    );
    // Ancien silence : les 5 observations entraient dans la matrice.
    assert_eq!(
        first_num_col(&ds),
        vec![Some(2.0), Some(3.0), Some(4.0)],
        "READ INTO must honor firstobs=2 obs=4"
    );
}

/// `options obs=2;` seul : la fenêtre garde les 2 premières observations.
#[test]
fn ra_j02_p14_iml_obs_only_window_use_read() {
    let (out, ds) = run_and_read(
        "data work.s; do i = 1 to 5; output; end; run;\n\
         options obs=2;\n\
         proc iml;\n\
           use work.s;\n\
           read all var {i} into m;\n\
           create work.out from m;\n\
           append from m;\n\
         quit;\n",
        "out",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert_eq!(
        first_num_col(&ds),
        vec![Some(1.0), Some(2.0)],
        "READ INTO must honor obs=2"
    );
}

/// La table source reste INTACTE sur disque : la fenêtre est une vue de
/// lecture, jamais une troncature du membre stocké (5 observations relues
/// hors fenêtre dans un second programme).
#[test]
fn ra_j02_p14_iml_window_does_not_truncate_source() {
    let (out, ds) = run_and_read(
        "data work.s; do i = 1 to 5; output; end; run;\n\
         options obs=2;\n\
         proc iml;\n\
           use work.s;\n\
           read all var {i} into m;\n\
         quit;\n\
         options firstobs=1 obs=max;\n\
         proc iml;\n\
           use work.s;\n\
           read all var {i} into m2;\n\
           create work.out from m2;\n\
           append from m2;\n\
         quit;\n",
        "out",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert_eq!(ds.n_obs(), 5, "stored table must keep all 5 observations");
}
