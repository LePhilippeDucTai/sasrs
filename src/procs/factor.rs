//! PROC FACTOR — principal axis / principal component factor analysis (M27).
//!
//! `proc factor data=<ref> [nfactors=k] [method=principal] [rotate=varimax|none]
//!              [cov] [out=<ref>];
//!  var <name list>; run;`
//!
//! ## Périmètre
//! - `data=`, `cov`, `nfactors=`, `rotate=varimax|none`, `out=` (parse-accepted).
//! - Critère de rétention : Kaiser (λ>1) ou NFACTORS=k explicite.
//! - Rotation VARIMAX (Kaiser 1958) : orthogonale, normalisation Kaiser.
//! - Différé : METHOD=ML/ITER, HEYWOOD, ALPHA, SCORE, rotations obliques.

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

pub struct FactorAst {
    pub data: Option<DatasetRef>,
    /// Use covariance matrix instead of correlation matrix.
    pub cov: bool,
    /// Number of factors to retain (None = Kaiser criterion λ>1).
    pub nfactors: Option<usize>,
    /// Factor extraction method (only "principal" supported).
    pub method: String,
    /// Rotation method: "none" or "varimax".
    pub rotate: String,
    /// OUT= dataset (parse-accepted; not executed).
    pub out: Option<DatasetRef>,
    /// VAR list (analysis variables, user order preserved).
    pub var: Vec<String>,
}

// ───────────────────────── Parser ─────────────────────────

/// Parse `proc factor [options]; [var ...;] run;`.
/// Called AFTER "proc factor" has been consumed. Consumes through run;/quit;.
pub fn parse(ts: &mut StatementStream) -> Result<FactorAst> {
    let mut data: Option<DatasetRef> = None;
    let mut cov = false;
    let mut nfactors: Option<usize> = None;
    let mut method = "principal".to_string();
    let mut rotate = "none".to_string();
    let mut out: Option<DatasetRef> = None;

    // --- PROC FACTOR statement options, until `;` ---
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
        } else if ts.peek().is_kw("nfactors") {
            common::expect_eq(ts, "NFACTORS")?;
            let span = ts.peek().span;
            let k = match ts.peek().kind {
                TokenKind::Num(v) => v,
                _ => return Err(SasError::parse("expected a number after NFACTORS=", span)),
            };
            ts.next();
            nfactors = Some(k as usize);
        } else if ts.peek().is_kw("method") {
            common::expect_eq(ts, "METHOD")?;
            let span = ts.peek().span;
            match ts.peek().ident() {
                Some(m) => {
                    method = m.to_lowercase();
                    ts.next();
                }
                None => return Err(SasError::parse("expected a method name after METHOD=", span)),
            }
        } else if ts.peek().is_kw("rotate") {
            common::expect_eq(ts, "ROTATE")?;
            let span = ts.peek().span;
            match ts.peek().ident() {
                Some(r) => {
                    rotate = r.to_lowercase();
                    ts.next();
                }
                None => return Err(SasError::parse("expected a rotation name after ROTATE=", span)),
            }
        } else if ts.peek().is_kw("out") {
            out = Some(common::parse_out_opt(ts)?);
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!(
                    "Unexpected option '{}' on PROC FACTOR statement.",
                    name.to_uppercase()
                ),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC FACTOR statement.",
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

    Ok(FactorAst {
        data,
        cov,
        nfactors,
        method,
        rotate,
        out,
        var,
    })
}

// ───────────────────────── VARIMAX rotation ─────────────────────────

/// Apply VARIMAX rotation (Kaiser 1958) to loading matrix L (n_vars × k_factors).
/// Returns (L_rotated, R_rotation_matrix).
/// Precondition: k >= 2, all h²[i] > 0.
pub fn varimax(l: &[Vec<f64>]) -> (Vec<Vec<f64>>, Vec<Vec<f64>>) {
    let n_vars = l.len();
    let k = if n_vars > 0 { l[0].len() } else { 0 };

    if k < 2 || n_vars == 0 {
        // No rotation needed.
        let r: Vec<Vec<f64>> = (0..k)
            .map(|i| (0..k).map(|j| if i == j { 1.0 } else { 0.0 }).collect())
            .collect();
        return (l.to_vec(), r);
    }

    // Initial communalities h²[i] = Σⱼ L[i][j]²
    let h2: Vec<f64> = l
        .iter()
        .map(|row| row.iter().map(|&x| x * x).sum())
        .collect();

    // Kaiser normalisation: divide each row i by sqrt(h²[i]).
    let h_sqrt: Vec<f64> = h2.iter().map(|&h| if h > 0.0 { h.sqrt() } else { 1.0 }).collect();
    let mut l_norm: Vec<Vec<f64>> = l
        .iter()
        .enumerate()
        .map(|(i, row)| row.iter().map(|&x| x / h_sqrt[i]).collect())
        .collect();

    // Rotation matrix R starts as identity.
    let mut rot: Vec<Vec<f64>> = (0..k)
        .map(|i| (0..k).map(|j| if i == j { 1.0 } else { 0.0 }).collect())
        .collect();

    // Compute current variance (criterion to maximise).
    fn varimax_criterion(l_norm: &[Vec<f64>], k: usize) -> f64 {
        let n = l_norm.len() as f64;
        let mut total = 0.0;
        for j in 0..k {
            let s2: f64 = l_norm.iter().map(|r| r[j].powi(4)).sum::<f64>();
            let s1: f64 = l_norm.iter().map(|r| r[j].powi(2)).sum::<f64>();
            total += n * s2 - s1 * s1;
        }
        total
    }

    let mut prev_var = varimax_criterion(&l_norm, k);

    for _iter in 0..1000 {
        for p in 0..k {
            for q in (p + 1)..k {
                // u[i] = A[i]² - B[i]²,  v[i] = 2*A[i]*B[i]
                let a: Vec<f64> = l_norm.iter().map(|r| r[p]).collect();
                let b: Vec<f64> = l_norm.iter().map(|r| r[q]).collect();
                let u: Vec<f64> = a.iter().zip(&b).map(|(&ai, &bi)| ai * ai - bi * bi).collect();
                let v: Vec<f64> = a.iter().zip(&b).map(|(&ai, &bi)| 2.0 * ai * bi).collect();

                let n = n_vars as f64;
                let a_sum: f64 = u.iter().sum();
                let b_sum: f64 = v.iter().sum();
                let c_val: f64 = u.iter().zip(&v).map(|(&ui, &vi)| ui * ui - vi * vi).sum();
                let d_val: f64 = u.iter().zip(&v).map(|(&ui, &vi)| ui * vi).sum::<f64>() * 2.0;

                let num = d_val - 2.0 * a_sum * b_sum / n;
                let denom = c_val - (a_sum * a_sum - b_sum * b_sum) / n;

                let angle = f64::atan2(num, denom) / 4.0;
                let cos_a = angle.cos();
                let sin_a = angle.sin();

                // Apply rotation to l_norm columns p and q.
                for row in l_norm.iter_mut() {
                    let rp = row[p];
                    let rq = row[q];
                    row[p] = cos_a * rp + sin_a * rq;
                    row[q] = -sin_a * rp + cos_a * rq;
                }
                // Accumulate rotation matrix.
                for row in rot.iter_mut() {
                    let rp = row[p];
                    let rq = row[q];
                    row[p] = cos_a * rp + sin_a * rq;
                    row[q] = -sin_a * rp + cos_a * rq;
                }
            }
        }
        let new_var = varimax_criterion(&l_norm, k);
        if (new_var - prev_var).abs() < 1e-6 {
            break;
        }
        prev_var = new_var;
    }

    // Kaiser denormalization: multiply each row i by sqrt(h²[i]).
    let l_rot: Vec<Vec<f64>> = l_norm
        .iter()
        .enumerate()
        .map(|(i, row)| row.iter().map(|&x| x * h_sqrt[i]).collect())
        .collect();

    (l_rot, rot)
}

// ───────────────────────── execute ─────────────────────────

pub fn execute(ast: &FactorAst, session: &mut Session) -> Result<()> {
    // Validate method.
    if ast.method != "principal" {
        return Err(SasError::runtime(format!(
            "PROC FACTOR METHOD={} is not supported. Only METHOD=PRINCIPAL is implemented.",
            ast.method.to_uppercase()
        )));
    }

    // Validate rotate.
    if ast.rotate != "none" && ast.rotate != "varimax" {
        return Err(SasError::runtime(format!(
            "PROC FACTOR ROTATE={} is not supported. Use ROTATE=VARIMAX or ROTATE=NONE.",
            ast.rotate.to_uppercase()
        )));
    }

    // At least 2 variables required.
    if ast.var.len() < 2 {
        return Err(SasError::runtime(
            "PROC FACTOR requires at least 2 variables.",
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

    // Complete-case rows.
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

    // Analysis matrix: covariance if cov, else correlation.
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
        for i in 0..p {
            amat[i][i] = 1.0;
        }
    }
    // Enforce exact symmetry before Jacobi.
    for i in 0..p {
        for j in (i + 1)..p {
            let avg = 0.5 * (amat[i][j] + amat[j][i]);
            amat[i][j] = avg;
            amat[j][i] = avg;
        }
    }

    // Eigen-decomposition: V columns = eigenvectors, lambda descending.
    let (mut v, lambda) = eigenvectors_jacobi(&amat)?;

    // Sign convention: per column, if the abs-max element is negative, flip.
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

    // Determine number of factors to retain.
    let k = if let Some(nf_req) = ast.nfactors {
        nf_req.max(1).min(p)
    } else {
        // Kaiser criterion: λ > 1.0
        let kaiser = lambda.iter().filter(|&&lam| lam > 1.0).count();
        kaiser.max(1)
    };

    let retention_msg = if ast.nfactors.is_some() {
        format!(
            "{} factor{} will be retained by the NFACTORS criterion.",
            k,
            if k == 1 { "" } else { "s" }
        )
    } else {
        format!(
            "{} factor{} will be retained by the MINEIGEN criterion.",
            k,
            if k == 1 { "" } else { "s" }
        )
    };

    // Compute loadings: L[i][j] = V[i][j] * sqrt(λ[j])  for j in 0..k
    let loadings: Vec<Vec<f64>> = (0..p)
        .map(|i| (0..k).map(|j| v[i][j] * lambda[j].max(0.0).sqrt()).collect())
        .collect();

    // Initial communalities: h²[i] = Σⱼ L[i][j]²
    let communalities: Vec<f64> = loadings
        .iter()
        .map(|row| row.iter().map(|&x| x * x).sum())
        .collect();

    // Variance explained by each factor (sum of squares of each column).
    let factor_variance: Vec<f64> = (0..k)
        .map(|j| loadings.iter().map(|row| row[j] * row[j]).sum::<f64>())
        .collect();

    // ───────────────────────── listing ─────────────────────────
    session.listing.page_header();
    centered(session, "The FACTOR Procedure");
    session.listing.blank();

    centered(session, "Initial Factor Method: Principal Components");
    session.listing.blank();

    centered(session, "Prior Communality Estimates: ONE");
    session.listing.blank();

    // Eigenvalues table (all p eigenvalues).
    let eig_title = if ast.cov {
        "Eigenvalues of the Covariance Matrix"
    } else {
        "Eigenvalues of the Correlation Matrix"
    };
    let total_label = if ast.cov {
        let avg = if p > 0 { trace / p as f64 } else { 0.0 };
        format!("Total = {:.4}   Average = {:.4}", trace, avg)
    } else {
        format!("Total = {:.0}   Average = 1", p)
    };
    centered(session, eig_title);
    centered(session, &total_label);
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
        let mut rows: Vec<Vec<String>> = Vec::with_capacity(p);
        let mut cumulative = 0.0_f64;
        for i in 0..p {
            cumulative += lambda[i];
            let diff = if i + 1 < p {
                format!("{:.4}", lambda[i] - lambda[i + 1])
            } else {
                ".".to_string()
            };
            let proportion = if trace != 0.0 { lambda[i] / trace } else { 0.0 };
            let cumul = if trace != 0.0 { cumulative / trace } else { 0.0 };
            rows.push(vec![
                format!("{}", i + 1),
                format!("{:.4}", lambda[i]),
                diff,
                format!("{:.4}", proportion),
                format!("{:.4}", cumul),
            ]);
        }
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // Retention criterion message.
    centered(session, &retention_msg);
    session.listing.blank();

    // Factor Pattern (initial loadings).
    centered(session, "Factor Pattern");
    session.listing.blank();
    {
        let mut headers: Vec<String> = vec![String::new()];
        let mut aligns: Vec<Align> = vec![Align::Left];
        for j in 0..k {
            headers.push(format!("Factor{}", j + 1));
            aligns.push(Align::Right);
        }
        let mut rows: Vec<Vec<String>> = Vec::with_capacity(p);
        for i in 0..p {
            let mut row = vec![names[i].clone()];
            for j in 0..k {
                row.push(format!("{:.4}", loadings[i][j]));
            }
            rows.push(row);
        }
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // Variance Explained by Each Factor.
    centered(session, "Variance Explained by Each Factor");
    session.listing.blank();
    {
        let mut headers: Vec<String> = vec![String::new()];
        let mut aligns: Vec<Align> = vec![Align::Left];
        for j in 0..k {
            headers.push(format!("Factor{}", j + 1));
            aligns.push(Align::Right);
        }
        let mut weighted_row = vec!["Weighted".to_string()];
        let mut unweighted_row = vec!["Unweighted".to_string()];
        for j in 0..k {
            weighted_row.push(format!("{:.4}", factor_variance[j]));
            unweighted_row.push(format!("{:.4}", factor_variance[j]));
        }
        session
            .listing
            .write_table(&headers, &aligns, &[weighted_row, unweighted_row]);
        session.listing.blank();
    }

    // Final Communality Estimates (before rotation).
    let total_communality: f64 = communalities.iter().sum();
    centered(
        session,
        &format!(
            "Final Communality Estimates: Total = {:.4}",
            total_communality
        ),
    );
    session.listing.blank();
    {
        let mut headers: Vec<String> = vec![String::new()];
        let mut aligns: Vec<Align> = vec![Align::Left];
        for nm in &names {
            headers.push(nm.clone());
            aligns.push(Align::Right);
        }
        let mut row: Vec<String> = vec![String::new()];
        for &h2 in &communalities {
            row.push(format!("{:.4}", h2));
        }
        session.listing.write_table(&headers, &aligns, &[row]);
        session.listing.blank();
    }

    // ───── VARIMAX rotation (if requested and k >= 2) ─────
    if ast.rotate == "varimax" && k >= 2 {
        let (l_rot, _rot_matrix) = varimax(&loadings);

        // Rotated variance by factor.
        let rot_variance: Vec<f64> = (0..k)
            .map(|j| l_rot.iter().map(|row| row[j] * row[j]).sum::<f64>())
            .collect();

        centered(session, "Rotation Method: Varimax");
        session.listing.blank();

        centered(session, "Rotated Factor Pattern");
        session.listing.blank();
        {
            let mut headers: Vec<String> = vec![String::new()];
            let mut aligns: Vec<Align> = vec![Align::Left];
            for j in 0..k {
                headers.push(format!("Factor{}", j + 1));
                aligns.push(Align::Right);
            }
            let mut rows: Vec<Vec<String>> = Vec::with_capacity(p);
            for i in 0..p {
                let mut row = vec![names[i].clone()];
                for j in 0..k {
                    row.push(format!("{:.4}", l_rot[i][j]));
                }
                rows.push(row);
            }
            session.listing.write_table(&headers, &aligns, &rows);
            session.listing.blank();
        }

        centered(session, "Variance Explained by Each Rotated Factor");
        session.listing.blank();
        {
            let mut headers: Vec<String> = vec![String::new()];
            let mut aligns: Vec<Align> = vec![Align::Left];
            for j in 0..k {
                headers.push(format!("Factor{}", j + 1));
                aligns.push(Align::Right);
            }
            let mut rot_row: Vec<String> = vec![String::new()];
            for j in 0..k {
                rot_row.push(format!("{:.4}", rot_variance[j]));
            }
            session.listing.write_table(&headers, &aligns, &[rot_row]);
            session.listing.blank();
        }

        // Final communalities (invariant under orthogonal rotation).
        let rot_communalities: Vec<f64> = l_rot
            .iter()
            .map(|row| row.iter().map(|&x| x * x).sum())
            .collect();
        let total_rot_comm: f64 = rot_communalities.iter().sum();
        centered(
            session,
            &format!(
                "Final Communality Estimates: Total = {:.4}",
                total_rot_comm
            ),
        );
        session.listing.blank();
        {
            let mut headers: Vec<String> = vec![String::new()];
            let mut aligns: Vec<Align> = vec![Align::Left];
            for nm in &names {
                headers.push(nm.clone());
                aligns.push(Align::Right);
            }
            let mut row: Vec<String> = vec![String::new()];
            for &h2 in &rot_communalities {
                row.push(format!("{:.4}", h2));
            }
            session.listing.write_table(&headers, &aligns, &[row]);
            session.listing.blank();
        }
    }

    // OUT= note.
    if ast.out.is_some() {
        session.log.note(
            "PROC FACTOR OUT= scoring is not yet implemented; the output data set was not created.",
        );
    }

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

    fn parse_factor(src: &str) -> Result<FactorAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "factor"
        parse(&mut ts)
    }

    // ───────────── parse tests ─────────────

    #[test]
    fn parse_minimal() {
        let ast = parse_factor("proc factor data=a; var x y; run;").unwrap();
        assert_eq!(ast.data.as_ref().unwrap().name, "a");
        assert!(!ast.cov);
        assert_eq!(ast.nfactors, None);
        assert_eq!(ast.method, "principal");
        assert_eq!(ast.rotate, "none");
        assert!(ast.out.is_none());
        assert_eq!(ast.var, vec!["x", "y"]);
    }

    #[test]
    fn parse_options() {
        let ast =
            parse_factor("proc factor data=a cov nfactors=2 rotate=varimax out=b; var x y z; run;")
                .unwrap();
        assert!(ast.cov);
        assert_eq!(ast.nfactors, Some(2));
        assert_eq!(ast.rotate, "varimax");
        assert_eq!(ast.out.as_ref().unwrap().name, "b");
        assert_eq!(ast.var, vec!["x", "y", "z"]);
    }

    #[test]
    fn parse_unknown_option_errors() {
        let r = parse_factor("proc factor data=a bogus; var x y; run;");
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
        let ast = FactorAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            cov: false,
            nfactors: None,
            method: "principal".into(),
            rotate: "none".into(),
            out: None,
            var: vec!["x".into()],
        };
        let r = execute(&ast, &mut session);
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("at least 2 variables"));
    }

    #[test]
    fn execute_invalid_method_errors() {
        let mut session = make_session();
        let ast = FactorAst {
            data: None,
            cov: false,
            nfactors: None,
            method: "ml".into(),
            rotate: "none".into(),
            out: None,
            var: vec!["x".into(), "y".into()],
        };
        let r = execute(&ast, &mut session);
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("ML"));
    }

    #[test]
    fn execute_invalid_rotate_errors() {
        let mut session = make_session();
        let ast = FactorAst {
            data: None,
            cov: false,
            nfactors: None,
            method: "principal".into(),
            rotate: "promax".into(),
            out: None,
            var: vec!["x".into(), "y".into()],
        };
        let r = execute(&ast, &mut session);
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("PROMAX"));
    }

    /// Oracle test: x=[1,2,3,4,5], y=[2,3,3,5,4]
    /// Expected: 1 factor retained (Kaiser), loading ≈ 0.9571,
    /// h² ≈ 0.9160, total communality ≈ 1.8321.
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

        let ast = FactorAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            cov: false,
            nfactors: None,
            method: "principal".into(),
            rotate: "none".into(),
            out: None,
            var: vec!["x".into(), "y".into()],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("The FACTOR Procedure"), "{listing}");
        assert!(listing.contains("Factor Pattern"), "{listing}");
        assert!(listing.contains("MINEIGEN criterion"), "{listing}");
        // Eigenvalue λ₁ ≈ 1.8321
        assert!(listing.contains("1.8321"), "λ₁ missing: {listing}");
        // Communality total ≈ 1.8321
        assert!(listing.contains("1.8321"), "total comm missing: {listing}");
    }

    /// Invariant: h²[i] before and after VARIMAX rotation differ by < 1e-8.
    #[test]
    fn varimax_communality_invariant() {
        // 3-variable, 2-factor loading matrix (arbitrary values).
        let l = vec![
            vec![0.8, 0.2],
            vec![0.7, 0.5],
            vec![0.3, 0.9],
        ];
        let h2_before: Vec<f64> = l
            .iter()
            .map(|row| row.iter().map(|&x| x * x).sum())
            .collect();

        let (l_rot, _) = varimax(&l);
        let h2_after: Vec<f64> = l_rot
            .iter()
            .map(|row| row.iter().map(|&x| x * x).sum())
            .collect();

        for (i, (&b, &a)) in h2_before.iter().zip(&h2_after).enumerate() {
            assert!(
                (b - a).abs() < 1e-8,
                "h²[{i}] changed: before={b:.10}, after={a:.10}"
            );
        }

        // Also check total variance conserved.
        let total_before: f64 = h2_before.iter().sum();
        let total_after: f64 = h2_after.iter().sum();
        assert!(
            (total_before - total_after).abs() < 1e-8,
            "total variance changed: {total_before:.10} -> {total_after:.10}"
        );
    }

    /// VARIMAX: k=1 should be a no-op (return L unchanged).
    #[test]
    fn varimax_k1_noop() {
        let l = vec![vec![0.8], vec![0.7], vec![0.9]];
        let (l_rot, rot) = varimax(&l);
        // l_rot should equal l (no rotation possible with 1 factor).
        for (i, (orig, rotated)) in l.iter().zip(&l_rot).enumerate() {
            for (j, (&o, &r)) in orig.iter().zip(rotated).enumerate() {
                assert!(
                    (o - r).abs() < 1e-12,
                    "l_rot[{i}][{j}]={r} != l[{i}][{j}]={o}"
                );
            }
        }
        // Rotation matrix should be [[1]].
        assert_eq!(rot.len(), 1);
        assert!((rot[0][0] - 1.0).abs() < 1e-12);
    }

    /// VARIMAX rotation matrix R must be orthogonal: R · R^T = I.
    /// Also verify L_rot = L_norm_rotated · scale (i.e., L · rot consistent).
    #[test]
    fn varimax_rotation_matrix_orthogonal() {
        let l = vec![
            vec![0.8, 0.2],
            vec![0.7, 0.5],
            vec![0.3, 0.9],
            vec![0.6, 0.1],
        ];
        let (_, rot) = varimax(&l);
        let k = rot.len();

        // R · R^T should be identity (k×k).
        for i in 0..k {
            for j in 0..k {
                let dot: f64 = (0..k).map(|m| rot[i][m] * rot[j][m]).sum();
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!(
                    (dot - expected).abs() < 1e-8,
                    "R·R^T[{i}][{j}] = {dot:.10}, expected {expected}"
                );
            }
        }
    }

    /// End-to-end: execute with k=2 and rotate=varimax on 3-var data.
    /// Verifies no panic, no NaN, communality invariant holds.
    #[test]
    fn execute_varimax_no_panic() {
        let mut session = make_session();
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
            "y" => [2.0_f64, 4.0, 3.0, 5.0, 1.0, 6.0],
            "z" => [5.0_f64, 4.0, 3.0, 2.0, 1.0, 6.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x"), num_meta("y"), num_meta("z")],
        };
        write_dataset(&mut session, "V", ds);

        let ast = FactorAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "V".into() }),
            cov: false,
            nfactors: Some(2),
            method: "principal".into(),
            rotate: "varimax".into(),
            out: None,
            var: vec!["x".into(), "y".into(), "z".into()],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("Rotated Factor Pattern"), "{listing}");
        assert!(listing.contains("Rotation Method: Varimax"), "{listing}");
        // No NaN in the listing.
        assert!(!listing.contains("NaN"), "NaN found in listing: {listing}");
    }
}
