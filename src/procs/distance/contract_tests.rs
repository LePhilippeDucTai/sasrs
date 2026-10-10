// ── J02-P4 : contrat DISTANCE (replis silencieux supprimés) ──────────────
//
// Chaque test reproduit l'ancien silence (commentaire « Base : … ») et fige
// le diagnostic ou la correction (texte, valeurs, code de sortie). Oracle de
// sévérité : CONTRIBUTING §5 ; référence : SAS/STAT 14.1 (SAS 9.4) User's
// Guide, The DISTANCE Procedure, Syntax and « Proximity Measures »
// https://support.sas.com/documentation/cdl/en/statug/68162/HTML/default/statug_distance_syntax.htm
// https://support.sas.com/documentation/cdl/en/statug/68162/HTML/default/statug_distance_details01.htm

use crate::missing::value_to_num;
use crate::procs::common::decode_column;
use crate::value::Value;

/// Exécute un programme complet (mode déterministe) : log, listing, code.
fn run_sas(src: &str) -> crate::RunOutcome {
    crate::run(
        src,
        crate::RunOptions {
            deterministic: true,
            ..Default::default()
        },
    )
}

/// Asserts a rejected step: exit code 2, `needle` in the log, no listing.
fn assert_error(out: &crate::RunOutcome, needle: &str, ctx: &str) {
    assert_eq!(out.exit_code, 2, "{ctx}: {}", out.log);
    assert!(
        out.log.contains(needle),
        "{ctx}: missing «{needle}»\n{}",
        out.log
    );
    assert!(
        !out.listing.contains("The DISTANCE Procedure"),
        "{ctx}: the step must be rejected before any output\n{}",
        out.listing
    );
}

/// Base : une valeur manquante d'une variable VAR se propageait en NaN dans
/// la matrice et dans OUT= sans diagnostic. ERROR jusqu'à roadmap-avancee
/// J09-P7 (poids des manquants de la doc).
#[test]
fn ra_j02_p4_distance_missing_values() {
    let out = run_sas(
        "data m; input x y; datalines;
1 2
. 3
4 5
;
run;
proc distance data=m out=d; var x y; run;
proc print data=d; run;",
    );
    assert_error(
        &out,
        "ERROR: A missing value in a VAR variable (X, observation 2) is not supported in PROC \
         DISTANCE; it can affect results and cannot be ignored (planned: roadmap-avancee \
         J09-P7).",
        "missing",
    );
    assert!(!out.log.contains("WORK.D has"), "{}", out.log);
}

/// OUT= matrix as rows of (`_TYPE_`, Col1..Coln), read back through the
/// session of a library-level test.
fn out_matrix(src: &str, table: &str, n: usize) -> (Vec<String>, Vec<Vec<f64>>) {
    let dir = tempfile::tempdir().unwrap();
    let out = crate::run(
        src,
        crate::RunOptions {
            work_dir: Some(dir.path().to_path_buf()),
            deterministic: true,
            ..Default::default()
        },
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    let session = crate::session::Session::new(
        Some(dir.path().to_path_buf()),
        dir.path().to_path_buf(),
        true,
    )
    .unwrap();
    let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
    let col = |name: &str| {
        let idx = ds.vars.iter().position(|v| v.name == name).unwrap();
        decode_column(&ds, idx).unwrap()
    };
    let types = col("_TYPE_")
        .into_iter()
        .map(|v| match v {
            Value::Char(s) => s,
            other => panic!("_TYPE_ {other:?}"),
        })
        .collect();
    let cols: Vec<Vec<f64>> = (1..=n)
        .map(|j| {
            col(&format!("Col{j}"))
                .iter()
                .map(|v| value_to_num(v).unwrap())
                .collect()
        })
        .collect();
    // rows[i][j] = Col(j+1) of observation i+1.
    let rows = (0..n)
        .map(|i| (0..n).map(|j| cols[j][i]).collect())
        .collect();
    (types, rows)
}

/// Base : METHOD=COSINE et METHOD=CORR rendaient les dissimilarités 1 − cos
/// et 1 − r (0 quand indéfinies, diagonale 0). SAS/STAT 9.4, PROC DISTANCE,
/// « Proximity Measures » : COSINE = coefficient cosinus
/// s20(x,y) = Σ xⱼyⱼ / √(Σ xⱼ² Σ yⱼ²) et CORR = coefficient de corrélation
/// s8(x,y) = Σ (xⱼ−x̄)(yⱼ−ȳ) / √(Σ (xⱼ−x̄)² Σ (yⱼ−ȳ)²), tous deux de TYPE=
/// SIMILAR. Correction directe : similarités, diagonale s(x,x) = 1, `_TYPE_`
/// « SIMILAR » ; dénominateur nul → ERROR (J09-P7). EUCLID inchangé.
#[test]
fn ra_j02_p4_distance_cosine_corr_similarity() {
    let s = 0.5_f64.sqrt();
    let (types, m) = out_matrix(
        "data ok; input x y; datalines;
1 0
0 1
1 1
;
run;
proc distance data=ok out=cs method=cosine; var x y; run;",
        "CS",
        3,
    );
    assert!(types.iter().all(|t| t == "SIMILAR"), "{types:?}");
    let expect = [[1.0, 0.0, s], [0.0, 1.0, s], [s, s, 1.0]];
    for i in 0..3 {
        for j in 0..3 {
            assert!(
                (m[i][j] - expect[i][j]).abs() < 1e-12,
                "cos[{i}][{j}] = {}",
                m[i][j]
            );
        }
    }

    // CORR across the coordinates of each observation: (1,2,3) and (2,4,6)
    // are perfectly correlated, (3,2,1) anti-correlated with both.
    let (types, m) = out_matrix(
        "data r; input a b c; datalines;
1 2 3
3 2 1
2 4 6
;
run;
proc distance data=r out=cr method=corr; var a b c; run;",
        "CR",
        3,
    );
    assert!(types.iter().all(|t| t == "SIMILAR"), "{types:?}");
    let expect = [[1.0, -1.0, 1.0], [-1.0, 1.0, -1.0], [1.0, -1.0, 1.0]];
    for i in 0..3 {
        for j in 0..3 {
            assert!(
                (m[i][j] - expect[i][j]).abs() < 1e-12,
                "r[{i}][{j}] = {}",
                m[i][j]
            );
        }
    }

    // The listing names a similarity matrix.
    let out = run_sas(
        "data ok; input x y; datalines;
1 0
0 1
1 1
;
run;
proc distance data=ok method=cosine; var x y; run;",
    );
    assert_eq!(out.exit_code, 0, "{}", out.log);
    assert!(out.listing.contains("Similarity Matrix"), "{}", out.listing);
    assert!(
        out.listing.contains("Row1    1.0000    0.0000    0.7071"),
        "{}",
        out.listing
    );

    // Zero denominator: undefined similarity, no invented value.
    let out = run_sas(
        "data z; input x y; datalines;
1 2
0 0
3 1
;
run;
proc distance data=z method=cosine; var x y; run;",
    );
    assert_error(
        &out,
        "ERROR: The METHOD=COSINE similarity of observations 1 and 2 is undefined (zero \
         denominator), which is not supported in PROC DISTANCE; it can affect results and \
         cannot be ignored (planned: roadmap-avancee J09-P7).",
        "cosine zero",
    );
    let out = run_sas(
        "data r; input a b; datalines;
1 2
3 3
;
run;
proc distance data=r method=corr; var a b; run;",
    );
    assert_error(
        &out,
        "ERROR: The METHOD=CORR similarity of observations 1 and 2 is undefined",
        "corr constant",
    );

    // EUCLID stays a distance (zero diagonal, `_TYPE_` DISTANCE).
    let (types, m) = out_matrix(
        "data ok; input x y; datalines;
1 0
0 1
1 1
;
run;
proc distance data=ok out=eu method=euclid; var x y; run;",
        "EU",
        3,
    );
    assert!(types.iter().all(|t| t == "DISTANCE"), "{types:?}");
    assert_eq!(m[0][0], 0.0);
    assert!((m[0][1] - 2.0_f64.sqrt()).abs() < 1e-12);
}

/// Base : l'instruction DISTANCE valide non implémentée COPY était signalée
/// « 180-322 … not valid ». Message du catalogue « not supported … cannot
/// be ignored » (BY, FREQ, ID, WEIGHT : déjà le message partagé) ; une
/// instruction inventée reste une 180-322.
#[test]
fn ra_j02_p4_distance_unsupported_statements() {
    let data = "data ok; input name $ x y; datalines;
P1 1 0
P2 0 1
;
run;
";
    for stmt in ["copy name", "by name", "freq x", "id name", "weight x"] {
        let kw = stmt.split_whitespace().next().unwrap().to_uppercase();
        let out = run_sas(&format!(
            "{data}proc distance data=ok; var x y; {stmt}; run;"
        ));
        assert_error(
            &out,
            &format!(
                "ERROR: The {kw} statement is not supported in PROC DISTANCE; it can affect \
                 results and cannot be ignored."
            ),
            stmt,
        );
        assert!(!out.log.contains("180-322"), "{stmt}: {}", out.log);
    }
    let out = run_sas(&format!(
        "{data}proc distance data=ok; var x y; invented x; run;"
    ));
    assert_error(
        &out,
        "180-322: Statement 'INVENTED' is not valid or it is used out of proper order in PROC \
         DISTANCE.",
        "invented",
    );
}
