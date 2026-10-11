//! ── J02-P14 : fenêtre OBS=/FIRSTOBS= sur les lectures BASE=/COMPARE= ────
//!
//! Base (ancien silence) : PROC COMPARE lisait BASE= et COMPARE= par
//! `provider.read` direct — les tables ENTIÈRES étaient comparées, la
//! fenêtre OBS=/FIRSTOBS= en vigueur était ignorée (5 observations lues et
//! 2 jugées inégales sous `obs=3`).
//!
//! Oracle : doc SAS 9.4, System Options, OBS= / FIRSTOBS= — les options
//! système valent pour TOUTE lecture d'une table par un PROC :
//! <https://documentation.sas.com/doc/en/pgmsascdc/9.4_3.5/lesysoptsref/p0c2av6mvf2zsrn1b48go7vk6bro.htm>

/// `options obs=3;` avant PROC COMPARE : seules les 3 premières observations
/// de BASE= et COMPARE= sont lues — la divergence des observations 4–5
/// n'est plus vue, le listing rapporte les comptes FENÊTRÉS.
#[test]
fn ra_j02_p14_compare_obs_window_base_compare() {
    let out = crate::run(
        "data work.a; do i = 1 to 5; x = i; output; end; run;\n\
         data work.b; do i = 1 to 5; x = i; if i >= 4 then x = 99; output; end; run;\n\
         options obs=3;\n\
         proc compare base=work.a compare=work.b; run;\n",
        crate::RunOptions {
            work_dir: None,
            base_dir: None,
            deterministic: true,
            vectorize: false,
        },
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(!out.log.contains("ERROR:"), "log:\n{}", out.log);
    // Les deux tables sont lues sur la fenêtre : 3 observations chacune
    // (ancien silence : « Read from WORK.A: 5 »).
    assert!(
        out.listing
            .contains("Total Number of Observations Read from WORK.A: 3"),
        "listing:\n{}",
        out.listing
    );
    assert!(
        out.listing
            .contains("Total Number of Observations Read from WORK.B: 3"),
        "listing:\n{}",
        out.listing
    );
    // Les observations 4–5 (seules divergentes) sont hors fenêtre : aucune
    // paire inégale (ancien silence : « … Unequal: 2 »).
    assert!(
        out.listing
            .contains("Number of Observations with Some Compared Variables Unequal: 0"),
        "listing:\n{}",
        out.listing
    );
    assert!(
        !out.listing
            .contains("Number of Observations with Some Compared Variables Unequal: 2"),
        "window ignored by PROC COMPARE:\n{}",
        out.listing
    );
}

/// `options firstobs=4;` : la comparaison ne porte que sur les observations
/// 4–5 — les seules divergentes ; deux paires inégales sont jugées, et le
/// listing rend le compte fenêtré (2 lues de chaque table).
#[test]
fn ra_j02_p14_compare_firstobs_window_base_compare() {
    let out = crate::run(
        "data work.a; do i = 1 to 5; x = i; output; end; run;\n\
         data work.b; do i = 1 to 5; x = i; if i >= 4 then x = 99; output; end; run;\n\
         options firstobs=4;\n\
         proc compare base=work.a compare=work.b; run;\n",
        crate::RunOptions {
            work_dir: None,
            base_dir: None,
            deterministic: true,
            vectorize: false,
        },
    );
    assert_eq!(out.exit_code, 0, "log:\n{}", out.log);
    assert!(
        out.listing
            .contains("Total Number of Observations Read from WORK.A: 2"),
        "listing:\n{}",
        out.listing
    );
    assert!(
        out.listing
            .contains("Number of Observations with Some Compared Variables Unequal: 2"),
        "listing:\n{}",
        out.listing
    );
}
