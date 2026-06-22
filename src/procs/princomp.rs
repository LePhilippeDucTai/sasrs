//! PROC PRINCOMP — principal component analysis (M27).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc princomp data=<ref> [cov] [n=<k>] [out=<ref>];
//!  var <name list>; run;`
//!
//! ## Périmètre
//! - Options du statement PROC : `data=`, `cov` (matrice de covariance au lieu
//!   de corrélation), `n=` (nombre de composantes à afficher), `out=`
//!   (parse-accepté ; les scores ne sont pas calculés en v1).
//! - `var` : variables numériques analysées (obligatoire, >= 2).
//! - Différé : `partial`, `weight`, `outstat=`, entrée TYPE=CORR, ODS plots.
//!
//! ## Sortie listing (titre "The PRINCOMP Procedure"), dans l'ordre SAS :
//! 1. Observations / Variables (n et p).
//! 2. Simple Statistics : Mean et StdDev par variable (échantillon, n-1).
//! 3. Correlation Matrix (ou Covariance Matrix si COV).
//! 4. Eigenvalues of the Correlation/Covariance Matrix : Eigenvalue,
//!    Difference, Proportion, Cumulative.
//! 5. Eigenvectors : matrice p×(k) des vecteurs propres.
//!
//! ## Conventions
//! - Observations : complete-case sur l'ENSEMBLE des variables `var` (une ligne
//!   est exclue si l'une quelconque des variables est missing).
//! - Écart-type / (co)variance : dénominateur n-1.
//! - Matrice de corrélation : diagonale forcée à 1.0, symétrisation exacte
//!   avant Jacobi (évite l'asymétrie de l'arrondi et un affichage 0.9999999).
//! - Convention de signe sur chaque vecteur propre : si l'élément de valeur
//!   absolue maximale (premier indice en cas d'égalité) est négatif, on inverse
//!   la colonne entière. Rend le snapshot stable.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column};
use crate::session::Session;
use crate::stat::eigenvectors_jacobi;
use crate::token::TokenKind;
use crate::value::VarType;

// ───────────────────────── AST ─────────────────────────

pub struct PrincompAst {
    pub data: Option<DatasetRef>,
    /// Use the covariance matrix instead of the (default) correlation matrix.
    pub cov: bool,
    /// Number of components to display (None = all p).
    pub n: Option<usize>,
    /// OUT= dataset (parse-accepted; scores not produced in v1).
    pub out: Option<DatasetRef>,
    /// VAR list (analysis variables, user order preserved).
    pub var: Vec<String>,
}

// ───────────────────────── Parser ─────────────────────────

/// Parse `proc princomp [data=a] [cov] [n=k] [out=b]; [var ...;] run;`.
/// Called AFTER "proc princomp" has been consumed. Consumes through run;/quit;.
pub fn parse(ts: &mut StatementStream) -> Result<PrincompAst> {
    let mut data: Option<DatasetRef> = None;
    let mut cov = false;
    let mut n: Option<usize> = None;
    let mut out: Option<DatasetRef> = None;

    // --- PROC PRINCOMP statement options, until `;` ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            data = Some(common::parse_dataset_opt(ts, "DATA")?);
        } else if ts.peek().is_kw("cov") || ts.peek().is_kw("covariance") {
            ts.next();
            cov = true;
        } else if ts.peek().is_kw("n") {
            common::expect_eq(ts, "N")?;
            let span = ts.peek().span;
            let k = match ts.peek().kind {
                TokenKind::Num(v) => v,
                _ => return Err(SasError::parse("expected a number after N=", span)),
            };
            ts.next();
            n = Some(k as usize);
        } else if ts.peek().is_kw("out") {
            out = Some(common::parse_out_opt(ts)?);
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!(
                    "Unexpected option '{}' on PROC PRINCOMP statement.",
                    name.to_uppercase()
                ),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC PRINCOMP statement.",
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; (combinateur partagé M31) ---
    let mut var: Vec<String> = Vec::new();
    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "var" => {
                ts.next();
                var = ts.parse_name_list()?;
                ts.expect_semi()?;
                true
            }
            _ => false,
        })
    })?;

    Ok(PrincompAst {
        data,
        cov,
        n,
        out,
        var,
    })
}

// ───────────────────────── execute ─────────────────────────

pub fn execute(ast: &PrincompAst, session: &mut Session) -> Result<()> {
    // At least 2 variables required.
    if ast.var.len() < 2 {
        return Err(SasError::runtime(
            "PROC PRINCOMP requires at least 2 variables.",
        ));
    }

    let in_ref = common::resolve_last_dataset(&ast.data, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();
    let display = format!("{in_libref}.{in_table}");

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }
    let n_read = ds.n_obs();
    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        n_read, display
    ));

    // Resolve VAR columns (user order preserved), validating existence + type.
    let mut cols: Vec<usize> = Vec::with_capacity(ast.var.len());
    for nm in &ast.var {
        match ds.vars.iter().position(|m| m.name.eq_ignore_ascii_case(nm)) {
            Some(i) => {
                if ds.vars[i].ty != VarType::Num {
                    return Err(SasError::runtime(format!(
                        "Variable '{}' not found in dataset '{}'.",
                        nm, display
                    )));
                }
                cols.push(i);
            }
            None => {
                return Err(SasError::runtime(format!(
                    "Variable '{}' not found in dataset '{}'.",
                    nm, display
                )));
            }
        }
    }
    let p = cols.len();
    let names: Vec<String> = cols.iter().map(|&c| ds.vars[c].name.clone()).collect();

    // Decode each analysis column once.
    let decoded: Vec<Vec<f64>> = cols
        .iter()
        .map(|&c| {
            decode_column(&ds, c).map(|vals| {
                vals.iter()
                    .map(|v| value_to_num(v).unwrap_or(f64::NAN))
                    .collect::<Vec<f64>>()
            })
        })
        .collect::<Result<Vec<_>>>()?;

    // Complete-case rows: keep a row only if ALL selected vars are non-missing.
    let mut data_rows: Vec<Vec<f64>> = Vec::new();
    for r in 0..n_read {
        let row: Vec<f64> = decoded.iter().map(|col| col[r]).collect();
        if row.iter().all(|v| v.is_finite()) {
            data_rows.push(row);
        }
    }
    let n = data_rows.len();
    if n == 0 {
        return Err(SasError::runtime("No observations with complete data."));
    }

    // Means and sample std (n-1).
    let nf = n as f64;
    let mut means = vec![0.0_f64; p];
    for row in &data_rows {
        for j in 0..p {
            means[j] += row[j];
        }
    }
    for m in &mut means {
        *m /= nf;
    }
    // Sum of squares of deviations per variable; sample std = sqrt(SS/(n-1)).
    let mut ss = vec![0.0_f64; p];
    for row in &data_rows {
        for j in 0..p {
            let d = row[j] - means[j];
            ss[j] += d * d;
        }
    }
    let denom = if n > 1 { nf - 1.0 } else { 1.0 };
    let stds: Vec<f64> = ss.iter().map(|s| (s / denom).sqrt()).collect();

    // Covariance matrix (n-1).
    let mut covm = vec![vec![0.0_f64; p]; p];
    for row in &data_rows {
        for i in 0..p {
            let di = row[i] - means[i];
            for j in 0..p {
                let dj = row[j] - means[j];
                covm[i][j] += di * dj;
            }
        }
    }
    for i in 0..p {
        for j in 0..p {
            covm[i][j] /= denom;
        }
    }

    // Analysis matrix: covariance if COV, else correlation.
    let mut amat = vec![vec![0.0_f64; p]; p];
    if ast.cov {
        amat = covm.clone();
    } else {
        for i in 0..p {
            for j in 0..p {
                let denom_ij = stds[i] * stds[j];
                amat[i][j] = if denom_ij > 0.0 {
                    (covm[i][j] / denom_ij).clamp(-1.0, 1.0)
                } else {
                    0.0
                };
            }
        }
        // Force exact diagonal 1.0 (clean display, valid correlation matrix).
        for i in 0..p {
            amat[i][i] = 1.0;
        }
    }
    // Enforce exact symmetry before Jacobi (rounding can break it).
    for i in 0..p {
        for j in (i + 1)..p {
            let avg = 0.5 * (amat[i][j] + amat[j][i]);
            amat[i][j] = avg;
            amat[j][i] = avg;
        }
    }

    // Eigen-decomposition: V columns = eigenvectors, lambda descending.
    let (mut v, lambda) = eigenvectors_jacobi(&amat)?;

    // Sign convention: per column, if the abs-max element (first-index tie
    // break) is negative, flip the whole column.
    for col in 0..p {
        let mut max_abs = 0.0_f64;
        let mut max_val = 0.0_f64;
        for row in 0..p {
            let a = v[row][col].abs();
            if a > max_abs {
                max_abs = a;
                max_val = v[row][col];
            }
        }
        if max_val < 0.0 {
            for row in 0..p {
                v[row][col] = -v[row][col];
            }
        }
    }

    let trace: f64 = lambda.iter().sum();

    // Number of components to display.
    let k = ast.n.map(|k| k.min(p)).unwrap_or(p).max(1).min(p);

    // ───────────────────────── listing ─────────────────────────
    session.listing.page_header();
    centered(session, "The PRINCOMP Procedure");
    session.listing.blank();

    session
        .listing
        .write_line(&format!(" Observations    {:>10}", n));
    session
        .listing
        .write_line(&format!(" Variables       {:>10}", p));
    session.listing.blank();

    // Simple Statistics: rows Mean / StdDev, columns = variables.
    centered(session, "Simple Statistics");
    session.listing.blank();
    {
        let mut headers: Vec<String> = vec![String::new()];
        let mut aligns: Vec<Align> = vec![Align::Left];
        for nm in &names {
            headers.push(nm.clone());
            aligns.push(Align::Right);
        }
        let mut mean_row = vec!["Mean".to_string()];
        for j in 0..p {
            mean_row.push(format!("{:.4}", means[j]));
        }
        let mut std_row = vec!["StdDev".to_string()];
        for j in 0..p {
            std_row.push(format!("{:.4}", stds[j]));
        }
        session
            .listing
            .write_table(&headers, &aligns, &[mean_row, std_row]);
        session.listing.blank();
    }

    // Correlation / Covariance Matrix.
    let matrix_title = if ast.cov {
        "Covariance Matrix"
    } else {
        "Correlation Matrix"
    };
    centered(session, matrix_title);
    session.listing.blank();
    {
        let mut headers: Vec<String> = vec![String::new()];
        let mut aligns: Vec<Align> = vec![Align::Left];
        for nm in &names {
            headers.push(nm.clone());
            aligns.push(Align::Right);
        }
        let mut rows: Vec<Vec<String>> = Vec::with_capacity(p);
        for i in 0..p {
            let mut row = vec![names[i].clone()];
            for j in 0..p {
                row.push(format!("{:.4}", amat[i][j]));
            }
            rows.push(row);
        }
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // Eigenvalues table.
    let eig_title = if ast.cov {
        "Eigenvalues of the Covariance Matrix"
    } else {
        "Eigenvalues of the Correlation Matrix"
    };
    centered(session, eig_title);
    session.listing.blank();
    {
        let headers: Vec<String> = vec![
            String::new(),
            "Eigenvalue".into(),
            "Difference".into(),
            "Proportion".into(),
            "Cumulative".into(),
        ];
        let aligns = vec![
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        let mut rows: Vec<Vec<String>> = Vec::with_capacity(k);
        let mut cumulative = 0.0_f64;
        for i in 0..k {
            cumulative += lambda[i];
            let diff = if i + 1 < p {
                format!("{:.4}", lambda[i] - lambda[i + 1])
            } else {
                ".".to_string()
            };
            let proportion = if trace != 0.0 { lambda[i] / trace } else { 0.0 };
            let cumul = if trace != 0.0 { cumulative / trace } else { 0.0 };
            rows.push(vec![
                format!("PRIN{}", i + 1),
                format!("{:.4}", lambda[i]),
                diff,
                format!("{:.4}", proportion),
                format!("{:.4}", cumul),
            ]);
        }
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // Eigenvectors table (6 decimals).
    centered(session, "Eigenvectors");
    session.listing.blank();
    {
        let mut headers: Vec<String> = vec![String::new()];
        let mut aligns: Vec<Align> = vec![Align::Left];
        for i in 0..k {
            headers.push(format!("PRIN{}", i + 1));
            aligns.push(Align::Right);
        }
        let mut rows: Vec<Vec<String>> = Vec::with_capacity(p);
        for row in 0..p {
            let mut r = vec![names[row].clone()];
            for col in 0..k {
                r.push(format!("{:.6}", v[row][col]));
            }
            rows.push(r);
        }
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // OUT= : write input columns + Prin1..Prink component scores.
    //
    // Scoring method: for each complete-case observation, the score on
    // component j is the (standardized — or only centered, if COV) data vector
    // dotted with eigenvector column j (the SAME eigenvectors, with the SAME
    // sign convention, used for the Eigenvectors listing above). For
    // correlation-based PCA each variable is standardized by its sample mean
    // and std; for COV the variable is only centered by its mean. With this
    // convention the score on component j has sample variance equal to
    // eigenvalue_j. Observations with any missing analysis variable receive
    // missing scores (rows are kept in input order, mirroring SAS).
    if let Some(out_ref) = &ast.out {
        write_out_dataset(session, &ds, &decoded, &means, &stds, &v, ast.cov, p, k, out_ref)?;
    }

    Ok(())
}

/// Build and write the PRINCOMP OUT= dataset: every input column plus
/// `Prin1..Prink` component scores. `decoded` holds the analysis columns in
/// original-row order (NaN for missing); `means`/`stds` are the sample
/// statistics; `v` the eigenvectors (columns), already sign-fixed.
#[allow(clippy::too_many_arguments)]
fn write_out_dataset(
    session: &mut Session,
    ds: &crate::dataset::SasDataset,
    decoded: &[Vec<f64>],
    means: &[f64],
    stds: &[f64],
    v: &[Vec<f64>],
    cov: bool,
    p: usize,
    k: usize,
    out_ref: &DatasetRef,
) -> Result<()> {
    use crate::dataset::{SasDataset, VarMeta};
    use polars::prelude::*;

    let n_read = ds.n_obs();

    // Per-component score columns (Option<f64>: None => SAS missing).
    let mut score_cols: Vec<Vec<Option<f64>>> = vec![Vec::with_capacity(n_read); k];
    for r in 0..n_read {
        let row: Vec<f64> = decoded.iter().map(|col| col[r]).collect();
        if row.iter().all(|x| x.is_finite()) {
            // Standardize (or center for COV).
            let z: Vec<f64> = (0..p)
                .map(|j| {
                    let centered = row[j] - means[j];
                    if cov {
                        centered
                    } else if stds[j] > 0.0 {
                        centered / stds[j]
                    } else {
                        0.0
                    }
                })
                .collect();
            for comp in 0..k {
                let score: f64 = (0..p).map(|j| z[j] * v[j][comp]).sum();
                score_cols[comp].push(Some(score));
            }
        } else {
            for comp in 0..k {
                score_cols[comp].push(None);
            }
        }
    }

    let mut out_df = ds.df.clone();
    for comp in 0..k {
        let name = format!("Prin{}", comp + 1);
        out_df
            .with_column(Series::new(name.into(), score_cols[comp].clone()))
            .map_err(|e| SasError::runtime(format!("PRINCOMP OUT= build failed: {e}")))?;
    }

    let mut vars = ds.vars.clone();
    for comp in 0..k {
        vars.push(VarMeta {
            name: format!("Prin{}", comp + 1),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        });
    }

    let out_ds = SasDataset { df: out_df, vars };
    let out_libref = out_ref.libref_or_work();
    let out_table = out_ref.name.to_uppercase();
    let out_display = format!("{out_libref}.{out_table}");
    let n_rows = out_ds.n_obs();
    let n_vars = out_ds.vars.len();
    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(out_display.clone());
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        out_display, n_rows, n_vars
    ));
    Ok(())
}

/// Write a centered line within LINESIZE.
fn centered(session: &mut Session, text: &str) {
    let ls = session.listing.ls();
    let pad = ls.saturating_sub(text.len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), text));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::session::Session;
    use crate::source::SourceFile;
    use crate::value::VarType;
    use polars::df;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn num_meta(name: &str) -> VarMeta {
        VarMeta {
            name: name.to_string(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        }
    }

    fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
        session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
        session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
    }

    fn parse_princomp(src: &str) -> Result<PrincompAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "princomp"
        parse(&mut ts)
    }

    // ───────────── parse tests ─────────────

    #[test]
    fn parse_minimal() {
        let ast = parse_princomp("proc princomp data=a; var x y; run;").unwrap();
        assert_eq!(ast.data.as_ref().unwrap().name, "a");
        assert!(!ast.cov);
        assert_eq!(ast.n, None);
        assert!(ast.out.is_none());
        assert_eq!(ast.var, vec!["x", "y"]);
    }

    #[test]
    fn parse_options() {
        let ast =
            parse_princomp("proc princomp data=a cov n=2 out=b; var x y z; run;").unwrap();
        assert!(ast.cov);
        assert_eq!(ast.n, Some(2));
        assert_eq!(ast.out.as_ref().unwrap().name, "b");
        assert_eq!(ast.var, vec!["x", "y", "z"]);
    }

    #[test]
    fn parse_unknown_option_errors() {
        let r = parse_princomp("proc princomp data=a bogus; var x y; run;");
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("BOGUS"));
    }

    // ───────────── execute / invariant tests ─────────────

    #[test]
    fn execute_too_few_variables_errors() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64, 2.0, 3.0]].unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x")],
        };
        write_dataset(&mut session, "T", ds);
        let ast = PrincompAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            cov: false,
            n: None,
            out: None,
            var: vec!["x".into()],
        };
        let r = execute(&ast, &mut session);
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("at least 2 variables"));
    }

    #[test]
    fn execute_missing_variable_errors() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64, 2.0], "y" => [3.0_f64, 4.0]].unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x"), num_meta("y")],
        };
        write_dataset(&mut session, "T", ds);
        let ast = PrincompAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            cov: false,
            n: None,
            out: None,
            var: vec!["x".into(), "z".into()],
        };
        let r = execute(&ast, &mut session);
        assert!(r.is_err());
        let msg = r.err().unwrap().to_string();
        assert!(msg.contains("'z' not found in dataset"), "{msg}");
    }

    /// Critical invariant: for the CORRELATION matrix, Σλ == number of variables.
    /// If the code mistakenly used the covariance matrix, the sum would differ.
    #[test]
    fn correlation_eigenvalues_sum_to_p() {
        // x=[1,2,3,4,5], y=[2,3,3,5,4] (the oracle fixture data).
        let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
        let ys = [2.0, 3.0, 3.0, 5.0, 4.0];
        let n = xs.len();
        let nf = n as f64;
        let mx = xs.iter().sum::<f64>() / nf;
        let my = ys.iter().sum::<f64>() / nf;
        let mut sxx = 0.0;
        let mut syy = 0.0;
        let mut sxy = 0.0;
        for i in 0..n {
            let dx = xs[i] - mx;
            let dy = ys[i] - my;
            sxx += dx * dx;
            syy += dy * dy;
            sxy += dx * dy;
        }
        let r = sxy / (sxx.sqrt() * syy.sqrt());
        let corr = vec![vec![1.0, r], vec![r, 1.0]];
        let (_, lambda) = eigenvectors_jacobi(&corr).unwrap();
        let sum: f64 = lambda.iter().sum();
        // For a 2-variable correlation matrix, Σλ must equal p = 2.
        assert!((sum - 2.0).abs() < 1e-10, "Σλ={sum}, expected 2.0");
        // And the eigenvalues are 1±r.
        assert!((lambda[0] - (1.0 + r)).abs() < 1e-10, "λ1={}", lambda[0]);
        assert!((lambda[1] - (1.0 - r)).abs() < 1e-10, "λ2={}", lambda[1]);
        // r should be ~0.8321.
        assert!((r - 0.8321).abs() < 1e-3, "r={r}");
    }

    #[test]
    fn execute_oracle_listing() {
        let mut session = make_session();
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0],
            "y" => [2.0_f64, 3.0, 3.0, 5.0, 4.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x"), num_meta("y")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = PrincompAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            cov: false,
            n: None,
            out: None,
            var: vec!["x".into(), "y".into()],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("The PRINCOMP Procedure"), "{listing}");
        assert!(listing.contains("Correlation Matrix"), "{listing}");
        // Eigenvalues 1.8321 and 0.1679.
        assert!(listing.contains("1.8321"), "{listing}");
        assert!(listing.contains("0.1679"), "{listing}");
        // Eigenvector elements 0.707107.
        assert!(listing.contains("0.707107"), "{listing}");
        // Means 3.0000 (x) and 3.4000 (y).
        assert!(listing.contains("3.0000"), "{listing}");
        assert!(listing.contains("3.4000"), "{listing}");
    }

    /// OUT= oracle: on the 2-var fixture, each component score must have
    /// sample variance (n-1) equal to its eigenvalue (1+r and 1-r), and the
    /// score columns must have mean ≈ 0.
    #[test]
    fn out_scores_variance_equals_eigenvalues() {
        use polars::prelude::*;
        let mut session = make_session();
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0],
            "y" => [2.0_f64, 3.0, 3.0, 5.0, 4.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x"), num_meta("y")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = PrincompAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            cov: false,
            n: None,
            out: Some(DatasetRef { libref: Some("WORK".into()), name: "SCORES".into() }),
            var: vec!["x".into(), "y".into()],
        };
        execute(&ast, &mut session).unwrap();

        // _LAST_ should now be the OUT= dataset.
        assert_eq!(session.last_dataset.as_deref(), Some("WORK.SCORES"));

        let (out, _) = session.libs.get("WORK").unwrap().read("SCORES").unwrap();
        // Input columns + Prin1 + Prin2.
        assert!(out.vars.iter().any(|m| m.name == "Prin1"));
        assert!(out.vars.iter().any(|m| m.name == "Prin2"));
        assert!(out.vars.iter().any(|m| m.name.eq_ignore_ascii_case("x")));

        // Known correlation r = 0.8321 -> eigenvalues 1+r and 1-r.
        let r = 0.8320502943378437_f64;
        let expected = [1.0 + r, 1.0 - r];

        for (comp, &lam) in expected.iter().enumerate() {
            let col = out
                .df
                .column(&format!("Prin{}", comp + 1))
                .unwrap()
                .f64()
                .unwrap();
            let vals: Vec<f64> = col.into_no_null_iter().collect();
            let n = vals.len() as f64;
            let mean = vals.iter().sum::<f64>() / n;
            assert!(mean.abs() < 1e-9, "Prin{} mean={mean}", comp + 1);
            let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
            assert!(
                (var - lam).abs() < 1e-9,
                "Prin{} variance={var}, expected eigenvalue {lam}",
                comp + 1
            );
        }
    }

    /// COV scoring: scores are only centered, not standardized; their sample
    /// variances equal the covariance-matrix eigenvalues (== column variances'
    /// sum). Verify the score column means are ≈ 0.
    #[test]
    fn out_scores_cov_centered_mean_zero() {
        let mut session = make_session();
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0],
            "y" => [2.0_f64, 3.0, 3.0, 5.0, 4.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x"), num_meta("y")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = PrincompAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            cov: true,
            n: None,
            out: Some(DatasetRef { libref: Some("WORK".into()), name: "CS".into() }),
            var: vec!["x".into(), "y".into()],
        };
        execute(&ast, &mut session).unwrap();
        let (out, _) = session.libs.get("WORK").unwrap().read("CS").unwrap();
        for comp in 1..=2 {
            let col = out.df.column(&format!("Prin{comp}")).unwrap().f64().unwrap();
            let vals: Vec<f64> = col.into_no_null_iter().collect();
            let mean = vals.iter().sum::<f64>() / vals.len() as f64;
            assert!(mean.abs() < 1e-9, "Prin{comp} mean={mean}");
        }
    }
}
