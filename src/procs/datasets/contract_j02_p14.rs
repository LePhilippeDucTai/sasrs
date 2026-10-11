//! ── J02-P14 : la fenêtre OBS=/FIRSTOBS= et les lectures de membres ──────
//!
//! PROC DATASETS est une proc de GESTION de bibliothèque : ses lectures de
//! membres (COPY, MODIFY) passent désormais par
//! `procs::common::open_resolved_full` — le helper de MÉTADONNÉES de
//! J02-P9, SANS fenêtre OBS=/FIRSTOBS=. Ces tests figent cette décision de
//! contrat.
//!
//! Base (ancien silence) : les lectures de COPY/MODIFY appelaient
//! `provider.read` directement, HORS du périmètre du contrat de fenêtre —
//! non couvertes, donc indocumentées ; une migration aveugle vers le
//! helper fenêtré `open_resolved` aurait TRONQUÉ les membres copiés ou
//! réécrits.
//!
//! Après : la lecture est explicitement une lecture de membre (métadonnées
//! et recopie intégrale), documentée ici et dans docs/support-contract.md,
//! section « Replis silencieux supprimés ». SAS 9.4 : PROC DATASETS recopie
//! et modifie des MEMBRES entiers (proc de gestion, comme PROC CONTENTS
//! préservé par J02-P9) ; les options système OBS=/FIRSTOBS= régissent les
//! lectures de DONNÉES par les procs de traitement (doc SAS 9.4, System
//! Options, OBS= / FIRSTOBS= :
//! <https://documentation.sas.com/doc/en/pgmsascdc/9.4_3.5/lesysoptsref/p0c2av6mvf2zsrn1b48go7vk6bro.htm>).

use crate::RunOptions;
use crate::dataset::SasDataset;
use std::path::Path;

/// Exécute `program` (WORK dans `<tmp>/work`, base_dir `<tmp>`) et rend
/// (issue, tables de `dir` indexées par nom minuscule).
fn run_in(dir_name: &str, program: &str) -> (crate::RunOutcome, SasDataset) {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let dir = tmp.path().join(dir_name);
    std::fs::create_dir_all(&dir).unwrap();
    let out = crate::run(
        program,
        RunOptions {
            work_dir: Some(work),
            base_dir: Some(tmp.path().to_path_buf()),
            deterministic: true,
            vectorize: false,
        },
    );
    let ds = read_single(&dir);
    (out, ds)
}

fn read_single(dir: &Path) -> SasDataset {
    let mut found: Vec<SasDataset> = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_some_and(|e| e == "parquet")
            && let Ok((ds, _)) = SasDataset::read_parquet(&p)
        {
            found.push(ds);
        }
    }
    assert_eq!(found.len(), 1, "expected exactly one copied member");
    found.pop().unwrap()
}

/// MODIFY sous `options obs=2;` : le membre garde ses 5 observations (seul
/// son descripteur change — variable renommée) ; une fenêtre sur la lecture
/// réécrirait le membre TRONQUÉ. Dans le même programme, un PROC MEANS
/// (lecture de DONNÉES) ne voit que 2 observations : la fenêtre est ACTIVE
/// et épargne uniquement la gestion de membres.
#[test]
fn ra_j02_p14_datasets_member_reads_documented_unwindowed() {
    let (out, t) = run_in(
        "work",
        "data work.t; do i = 1 to 5; output; end; run;\n\
         options obs=2;\n\
         proc datasets lib=work nolist;\n\
           modify t;\n\
             rename i=j;\n\
         quit;\n\
         proc means data=work.t;\n\
           var j;\n\
         run;\n",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(!out.log.contains("ERROR:"), "log:\n{}", out.log);
    // Le membre n'est PAS tronqué : MODIFY est une édition de descripteur.
    assert_eq!(t.n_obs(), 5, "MODIFY must not truncate the member");
    let names: Vec<String> =
        t.df.get_columns()
            .iter()
            .map(|c| c.name().to_string())
            .collect();
    assert_eq!(names, vec!["j"], "columns: {names:?}");
    // La fenêtre obs=2 est bien ACTIVE dans la même session : PROC MEANS
    // (lecture de données via open_input, J02-P9) ne voit que 2 obs → N=2.
    let means_line = out
        .listing
        .lines()
        .find(|l| l.trim_start().to_ascii_lowercase().starts_with("j "))
        .unwrap_or_else(|| panic!("MEANS row not found:\n{}", out.listing));
    let nums: Vec<&str> = means_line.split_whitespace().collect();
    assert_eq!(nums[1], "2", "MEANS N must be 2:\n{}", out.listing);
}

/// COPY out= sous `options obs=2;` : le membre est recopié ENTIER (5
/// observations) — recopie de membre, pas une lecture de données.
#[test]
fn ra_j02_p14_datasets_copy_member_unwindowed() {
    let (out, copied) = run_in(
        "out",
        "data work.t; do i = 1 to 5; output; end; run;\n\
         libname outlib 'out';\n\
         options obs=2;\n\
         proc datasets lib=work nolist; copy out=outlib; quit;\n",
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(!out.log.contains("ERROR:"), "log:\n{}", out.log);
    assert_eq!(copied.n_obs(), 5, "COPY must copy the whole member");
}
