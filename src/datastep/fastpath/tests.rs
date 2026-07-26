use super::*;
use crate::datastep::compile;
use crate::datastep::exec::execute;
use crate::missing::encode_special;
use crate::parser::StatementStream;
use crate::source::SourceFile;
use crate::value::MissingKind;
use polars::df;
use std::path::PathBuf;

fn session(vectorize: bool) -> Session {
    let mut s = Session::new(None, PathBuf::from("."), true).unwrap();
    s.vectorize = vectorize;
    s
}

/// Entrée avec un missing ordinaire (`.`) ET un spécial (`.A`) en numérique,
/// plus une colonne caractère — pour éprouver la propagation des missings
/// et la préservation du NaN-payload.
fn write_input(session: &Session) {
    let df = df!(
        "Age" => [
            Some(14.0),
            None,
            Some(encode_special(MissingKind::Letter(0))), // .A
            Some(13.0),
        ],
        "Name" => ["Alfred", "Alice", "Carol", "Barbara"],
    )
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "Age".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
        VarMeta {
            name: "Name".into(),
            ty: VarType::Char,
            length: 7,
            format: None,
            label: None,
        },
    ];
    session
        .libs
        .get("WORK")
        .unwrap()
        .write("inp", &SasDataset { df, vars })
        .unwrap();
}

fn parse_compile(src: &str, session: &mut Session) -> StepProgram {
    let file = SourceFile::new(src);
    let mut ts = StatementStream::new(&file).unwrap();
    assert!(ts.next().is_kw("data"));
    let ast = crate::parser::datastep::parse_data_step(&mut ts).unwrap();
    compile(&ast, session).unwrap()
}

/// Exécute `src` sur une session neuve (fast-path selon `vectorize`) et
/// renvoie (sortie WORK.OUT, log). Le log d'`execute` ne contient que des
/// NOTEs (l'écho est fait par l'exécuteur de haut niveau), donc les deux
/// chemins sont directement comparables.
fn run_capture(src: &str, vectorize: bool) -> (SasDataset, String) {
    let mut s = session(vectorize);
    write_input(&s);
    let prog = parse_compile(src, &mut s);
    if vectorize {
        assert!(
            eligible(&prog),
            "étape attendue éligible au fast-path : {src}"
        );
    }
    execute(prog, &mut s).unwrap();
    let out = s.libs.get("WORK").unwrap().read("out").unwrap().0;
    (out, s.log.into_string())
}

fn is_eligible(src: &str) -> bool {
    let mut s = session(true);
    write_input(&s);
    let prog = parse_compile(src, &mut s);
    eligible(&prog)
}

/// Compare deux sorties colonne par colonne — les f64 comparés BIT À BIT
/// pour distinguer `.` (null) de `.A` (NaN-payload) et de tout nombre.
fn assert_same_output(a: &SasDataset, b: &SasDataset) {
    assert_eq!(a.df.width(), b.df.width(), "largeur différente");
    assert_eq!(a.n_obs(), b.n_obs(), "nb obs différent");
    for (ca, cb) in a.df.get_columns().iter().zip(b.df.get_columns()) {
        assert_eq!(ca.name(), cb.name(), "nom de colonne");
        assert_eq!(ca.dtype(), cb.dtype(), "dtype de {}", ca.name());
        match ca.dtype() {
            DataType::Float64 => {
                let fa = ca.f64().unwrap();
                let fb = cb.f64().unwrap();
                for i in 0..fa.len() {
                    match (fa.get(i), fb.get(i)) {
                        (None, None) => {}
                        (Some(x), Some(y)) => assert_eq!(
                            x.to_bits(),
                            y.to_bits(),
                            "col {} ligne {i} (bits)",
                            ca.name()
                        ),
                        _ => panic!("null/non-null divergent col {} ligne {i}", ca.name()),
                    }
                }
            }
            DataType::String => {
                let sa = ca.str().unwrap();
                let sb = cb.str().unwrap();
                for i in 0..sa.len() {
                    assert_eq!(sa.get(i), sb.get(i), "col {} ligne {i}", ca.name());
                }
            }
            other => panic!("dtype inattendu {other}"),
        }
    }
}

// ── Équivalence fast-path ⇔ boucle ligne-à-ligne ────────────────────────

#[test]
fn equivalence_arithmetic_with_missings() {
    let src = "data out; set inp; x = age * 2; run;";
    let (off, log_off) = run_capture(src, false);
    let (on, log_on) = run_capture(src, true);
    assert_same_output(&off, &on);
    assert_eq!(log_off, log_on, "logs divergents");
    // Sanity : x = age*2, missing propagé pour `.` ET `.A` ; NOTE missing.
    let x = on.df.column("x").unwrap().f64().unwrap();
    assert_eq!(x.get(0), Some(28.0));
    assert_eq!(x.get(1), None);
    assert_eq!(x.get(2), None);
    assert_eq!(x.get(3), Some(26.0));
    assert!(log_on.contains("Missing values were generated"));
}

#[test]
fn equivalence_copy_preserves_special_missing() {
    let src = "data out; set inp; y = age; run;";
    let (off, log_off) = run_capture(src, false);
    let (on, log_on) = run_capture(src, true);
    assert_same_output(&off, &on);
    assert_eq!(log_off, log_on);
    // La copie nue préserve le payload .A et ne génère AUCUN missing.
    let y = on.df.column("y").unwrap().f64().unwrap();
    assert_eq!(y.get(0), Some(14.0));
    assert_eq!(y.get(1), None);
    assert!(y.get(2).unwrap().is_nan());
    assert_eq!(
        y.get(2).unwrap().to_bits(),
        encode_special(MissingKind::Letter(0)).to_bits()
    );
    assert!(!log_on.contains("Missing values were generated"));
}

#[test]
fn equivalence_sequential_dependency() {
    // y dépend de x assignée juste avant (ordre séquentiel).
    let src = "data out; set inp; x = age + 1; y = x * 2; run;";
    let (off, _) = run_capture(src, false);
    let (on, _) = run_capture(src, true);
    assert_same_output(&off, &on);
}

#[test]
fn equivalence_literal_and_keep() {
    let src = "data out; set inp; flag = 1; keep Name flag; run;";
    let (off, log_off) = run_capture(src, false);
    let (on, log_on) = run_capture(src, true);
    assert_same_output(&off, &on);
    assert_eq!(log_off, log_on);
    assert_eq!(on.df.width(), 2); // Name, flag
    assert!(!log_on.contains("Missing values were generated"));
}

// ── Le garde-fou rejette tout ce qui sort du périmètre ──────────────────

#[test]
fn gate_rejects_subsetting_if() {
    assert!(!is_eligible("data out; set inp; if age > 13; run;"));
}

#[test]
fn gate_rejects_division() {
    assert!(!is_eligible("data out; set inp; x = age / 2; run;"));
}

#[test]
fn gate_rejects_char_assignment() {
    assert!(!is_eligible("data out; set inp; z = 'hi'; run;"));
}

#[test]
fn gate_rejects_explicit_output() {
    assert!(!is_eligible("data out; set inp; output; run;"));
}

#[test]
fn gate_rejects_no_input() {
    assert!(!is_eligible("data out; x = 1; run;"));
}

/// Avec le flag ON mais une étape NON éligible (subsetting IF), `execute`
/// doit retomber sur la boucle ligne-à-ligne et rester correct.
#[test]
fn ineligible_step_falls_back_under_flag() {
    let src = "data out; set inp; if age > 13; run;";
    assert!(!is_eligible(src));
    let mut s = session(true);
    write_input(&s);
    let prog = parse_compile(src, &mut s);
    execute(prog, &mut s).unwrap();
    let out = s.libs.get("WORK").unwrap().read("out").unwrap().0;
    // age > 13 : seul Alfred (14) ; `.`, `.A` et 13 sont faux.
    assert_eq!(out.n_obs(), 1);
    assert_eq!(
        out.df.column("Name").unwrap().str().unwrap().get(0),
        Some("Alfred")
    );
}
