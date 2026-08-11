//! M39.1 — oracle round-trip du sidecar de catalogue de formats par libref.
//!
//! Scénario : une PREMIÈRE session (« session A ») définit un format
//! utilisateur dans une bibliothèque PERMANENTE (`PROC FORMAT LIBRARY=perm;`),
//! l'applique et produit un listing. Une SECONDE session (« session B »),
//! process Rust totalement indépendant (nouveau `Session`/`run`, aucun état
//! partagé), se contente d'un `LIBNAME` sur le MÊME répertoire — sans
//! relancer `PROC FORMAT` — et doit résoudre le format identiquement :
//! même listing, caractère pour caractère.

use sasrs::{RunOptions, run};

fn run_det(src: &str) -> sasrs::RunOutcome {
    run(
        src,
        RunOptions {
            work_dir: None,
            base_dir: None,
            deterministic: true,
            vectorize: false,
        },
    )
}

#[test]
fn format_catalog_round_trips_across_independent_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let libdir = tmp.path().join("permlib");
    std::fs::create_dir_all(&libdir).unwrap();
    let lib_path = libdir.display().to_string();

    // ── Session A : définit GRADEF dans la bibliothèque permanente, l'utilise.
    let src_a = format!(
        r#"
libname perm '{lib_path}';
proc format library=perm;
  value gradef 1='Pass' 2='Fail' other='Unknown';
run;
data work.t;
  input g;
  format g gradef.;
  datalines;
1
2
3
;
run;
proc print data=work.t noobs;
run;
"#
    );
    let out_a = run_det(&src_a);
    assert_eq!(out_a.exit_code, 0, "session A log:\n{}", out_a.log);
    assert!(
        libdir.join("formats.sascat.json").is_file(),
        "PROC FORMAT LIBRARY=perm should have written the sidecar catalog"
    );
    assert!(
        out_a.listing.contains("Pass"),
        "listing A:\n{}",
        out_a.listing
    );
    assert!(
        out_a.listing.contains("Fail"),
        "listing A:\n{}",
        out_a.listing
    );
    assert!(
        out_a.listing.contains("Unknown"),
        "listing A:\n{}",
        out_a.listing
    );

    // ── Session B : PROCESSUS INDÉPENDANT (nouveau run()). Aucune PROC FORMAT
    // ici — seul le LIBNAME sur le même répertoire doit suffire à résoudre
    // GRADEF depuis le sidecar persisté par la session A.
    let src_b = format!(
        r#"
libname perm '{lib_path}';
data work.t;
  input g;
  format g gradef.;
  datalines;
1
2
3
;
run;
proc print data=work.t noobs;
run;
"#
    );
    let out_b = run_det(&src_b);
    assert_eq!(out_b.exit_code, 0, "session B log:\n{}", out_b.log);
    assert!(
        !out_b.log.to_uppercase().contains("ERROR"),
        "session B log should be clean:\n{}",
        out_b.log
    );

    assert_eq!(
        out_a.listing, out_b.listing,
        "round-trip: session B (LIBNAME-only) must render exactly like session A \
         (which defined the format) — session A listing:\n{}\n---\nsession B listing:\n{}",
        out_a.listing, out_b.listing
    );
}

/// Complément : sans persistance (LIB par défaut = WORK), la bibliothèque ne
/// reçoit AUCUN sidecar — la session B (LIBNAME seul, pas de PROC FORMAT)
/// ne doit PAS résoudre GRADEF (repli sur le format brut, comme aujourd'hui).
#[test]
fn work_only_format_does_not_leak_to_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let libdir = tmp.path().join("permlib2");
    std::fs::create_dir_all(&libdir).unwrap();
    let lib_path = libdir.display().to_string();

    let src_a = format!(
        r#"
libname perm '{lib_path}';
proc format;
  value gradef 1='Pass' 2='Fail' other='Unknown';
run;
"#
    );
    let out_a = run_det(&src_a);
    assert_eq!(out_a.exit_code, 0, "session A log:\n{}", out_a.log);
    assert!(
        !libdir.join("formats.sascat.json").exists(),
        "a WORK-only PROC FORMAT (no LIBRARY=) must never write a sidecar, even \
         when a permanent libref happens to be assigned"
    );
}
