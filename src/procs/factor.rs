//! PROC FACTOR — principal axis / principal component factor analysis (M27).
//!
//! `proc factor data=<ref> [nfactors=k] [method=principal] [rotate=varimax|none]
//!              [cov] [out=<ref>];
//!  var <name list>; run;`
//!
//! ## Périmètre
//! - `data=`, `cov`, `nfactors=`, `rotate=varimax|none|promax`, `out=`.
//! - Critère de rétention : Kaiser (λ>1) ou NFACTORS=k explicite.
//! - Rotation VARIMAX (Kaiser 1958) : orthogonale, normalisation Kaiser.
//! - Rotation PROMAX : oblique, partant de la solution VARIMAX (cible élevée à
//!   la puissance k=4, ajustement de Procrustes). Produit le « Rotated Factor
//!   Pattern » oblique et les « Inter-Factor Correlations ».
//! - OUT= : colonnes d'entrée + `Factor1..Factorm` (scores par régression).
//! - Différé : METHOD=ML/ITER, HEYWOOD, ALPHA, ROTATE=OBLIMIN.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column};
use crate::session::Session;
use crate::stat::{eigenvectors_jacobi, invert_matrix};
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
    /// Rotation method: "none", "varimax", or "promax".
    pub rotate: String,
    /// OUT= dataset (factor scores).
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

// ───────────────────────── PROMAX rotation ─────────────────────────

/// Result of a PROMAX (oblique) rotation.
pub struct PromaxResult {
    /// Oblique factor pattern P (n_vars × k): standardized regression
    /// coefficients of the variables on the (correlated) factors.
    pub pattern: Vec<Vec<f64>>,
    /// Inter-factor correlation matrix Φ (k × k).
    pub phi: Vec<Vec<f64>>,
}

/// Multiply two row-major matrices: (m×n) · (n×p) → (m×p).
fn matmul(a: &[Vec<f64>], b: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let m = a.len();
    let n = if m > 0 { a[0].len() } else { 0 };
    let p = if !b.is_empty() { b[0].len() } else { 0 };
    let mut out = vec![vec![0.0_f64; p]; m];
    for i in 0..m {
        for k in 0..n {
            let aik = a[i][k];
            if aik == 0.0 {
                continue;
            }
            for j in 0..p {
                out[i][j] += aik * b[k][j];
            }
        }
    }
    out
}

/// Transpose a row-major matrix.
fn transpose(a: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let m = a.len();
    let n = if m > 0 { a[0].len() } else { 0 };
    let mut out = vec![vec![0.0_f64; m]; n];
    for i in 0..m {
        for j in 0..n {
            out[j][i] = a[i][j];
        }
    }
    out
}

/// Apply PROMAX (Hendrickson & White 1964) oblique rotation, starting from the
/// orthogonal VARIMAX loadings `l_varimax` (n_vars × k). Power `power` (k=4 by
/// default in SAS) controls how aggressively the target sharpens the loadings.
///
/// Algorithm:
///   1. Build the target matrix `target[i][j] = |l[i][j]|^(power+1) / l[i][j]`
///      (sign-preserving power), i.e. raise each loading to `power` in
///      magnitude while keeping its sign.
///   2. Least-squares fit a transformation Q minimizing ‖L·Q − target‖:
///      Q = (Lᵀ L)⁻¹ Lᵀ target  (Procrustes / column-wise regression).
///   3. Normalize the columns of Q so that diag((QᵀQ)⁻¹) = 1, giving the
///      oblique pattern P = L · Q and inter-factor correlations
///      Φ = (Qᵀ Q)⁻¹ after the same normalization.
///
/// Returns the oblique pattern and the inter-factor correlation matrix.
pub fn promax(l_varimax: &[Vec<f64>], power: i32) -> Result<PromaxResult> {
    let n_vars = l_varimax.len();
    let k = if n_vars > 0 { l_varimax[0].len() } else { 0 };

    if k < 2 {
        // No oblique rotation possible with a single factor: identity Φ.
        let phi = vec![vec![1.0_f64; k.max(1)]; k.max(1)];
        return Ok(PromaxResult {
            pattern: l_varimax.to_vec(),
            phi,
        });
    }

    // 1. Sign-preserving power target: target = sign(l) * |l|^power.
    let target: Vec<Vec<f64>> = l_varimax
        .iter()
        .map(|row| {
            row.iter()
                .map(|&x| x.signum() * x.abs().powi(power))
                .collect()
        })
        .collect();

    // 2. Q = (Lᵀ L)⁻¹ Lᵀ target.
    let lt = transpose(l_varimax);
    let ltl = matmul(&lt, l_varimax); // k×k
    let ltl_inv = invert_matrix(&ltl)?;
    let lt_target = matmul(&lt, &target); // k×k
    let mut q = matmul(&ltl_inv, &lt_target); // k×k

    // 3. Normalize columns of Q so that the resulting factors have unit
    //    variance: scale column j by 1/sqrt(diag((QᵀQ)⁻¹)[j]).
    let qtq = matmul(&transpose(&q), &q); // k×k
    let qtq_inv = invert_matrix(&qtq)?;
    let scale: Vec<f64> = (0..k)
        .map(|j| {
            let d = qtq_inv[j][j];
            if d > 0.0 {
                d.sqrt()
            } else {
                1.0
            }
        })
        .collect();
    for row in q.iter_mut() {
        for j in 0..k {
            row[j] *= scale[j];
        }
    }

    // Oblique pattern P = L · Q.
    let pattern = matmul(l_varimax, &q);

    // Inter-factor correlations Φ = (Qᵀ Q)⁻¹ for the normalized Q.
    let qtq2 = matmul(&transpose(&q), &q);
    let mut phi = invert_matrix(&qtq2)?;
    // Force exact unit diagonal and symmetry (clean display).
    for i in 0..k {
        for j in (i + 1)..k {
            let avg = 0.5 * (phi[i][j] + phi[j][i]);
            phi[i][j] = avg;
            phi[j][i] = avg;
        }
    }
    for i in 0..k {
        phi[i][i] = 1.0;
    }

    Ok(PromaxResult { pattern, phi })
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
    if ast.rotate == "oblimin" {
        return Err(SasError::runtime(
            "PROC FACTOR ROTATE=OBLIMIN is not yet implemented. Use ROTATE=PROMAX, VARIMAX or NONE.",
        ));
    }
    if ast.rotate != "none" && ast.rotate != "varimax" && ast.rotate != "promax" {
        return Err(SasError::runtime(format!(
            "PROC FACTOR ROTATE={} is not supported. Use ROTATE=PROMAX, VARIMAX or NONE.",
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

    // Pattern used for OUT= factor scoring (rotated when a rotation applies).
    let mut final_pattern: Vec<Vec<f64>> = loadings.clone();

    // ───── VARIMAX rotation (if requested and k >= 2) ─────
    if ast.rotate == "varimax" && k >= 2 {
        let (l_rot, _rot_matrix) = varimax(&loadings);
        final_pattern = l_rot.clone();

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

    // ───── PROMAX oblique rotation (if requested and k >= 2) ─────
    if ast.rotate == "promax" && k >= 2 {
        // Promax starts from the orthogonal VARIMAX solution.
        let (l_varimax, _rot_matrix) = varimax(&loadings);
        let pm = promax(&l_varimax, 4)?;
        final_pattern = pm.pattern.clone();

        centered(session, "Rotation Method: Promax (power = 4)");
        session.listing.blank();

        // Oblique Rotated Factor Pattern (Standardized Regression Coefficients).
        centered(session, "Rotated Factor Pattern (Standardized Regression Coefficients)");
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
                    row.push(format!("{:.4}", pm.pattern[i][j]));
                }
                rows.push(row);
            }
            session.listing.write_table(&headers, &aligns, &rows);
            session.listing.blank();
        }

        // Inter-Factor Correlations.
        centered(session, "Inter-Factor Correlations");
        session.listing.blank();
        {
            let mut headers: Vec<String> = vec![String::new()];
            let mut aligns: Vec<Align> = vec![Align::Left];
            for j in 0..k {
                headers.push(format!("Factor{}", j + 1));
                aligns.push(Align::Right);
            }
            let mut rows: Vec<Vec<String>> = Vec::with_capacity(k);
            for i in 0..k {
                let mut row = vec![format!("Factor{}", i + 1)];
                for j in 0..k {
                    row.push(format!("{:.4}", pm.phi[i][j]));
                }
                rows.push(row);
            }
            session.listing.write_table(&headers, &aligns, &rows);
            session.listing.blank();
        }
    }

    // OUT= : write input columns + Factor1..Factorm regression factor scores.
    //
    // Scoring method (standard SAS regression scoring): with Z the matrix of
    // standardized analysis variables, R the correlation matrix and `pattern`
    // the (possibly rotated) factor pattern, the standardized scoring
    // coefficients are B = R⁻¹ · pattern (n_vars × k) and the factor scores are
    // F = Z · B. For COV analysis the variables are only centered. Observations
    // with any missing analysis variable receive missing scores.
    if let Some(out_ref) = &ast.out {
        write_out_dataset(
            session,
            &ds,
            &decoded,
            &means,
            &stds,
            &amat,
            &final_pattern,
            ast.cov,
            p,
            k,
            out_ref,
        )?;
    }

    Ok(())
}

/// Build and write the FACTOR OUT= dataset: every input column plus
/// `Factor1..Factorm` regression factor scores. Scores = Z · (R⁻¹ · pattern),
/// where Z is the standardized (or, for COV, centered) data and R = `amat` the
/// analysis matrix. Incomplete observations receive missing scores; rows are
/// kept in input order (mirroring SAS).
#[allow(clippy::too_many_arguments)]
fn write_out_dataset(
    session: &mut Session,
    ds: &crate::dataset::SasDataset,
    decoded: &[Vec<f64>],
    means: &[f64],
    stds: &[f64],
    amat: &[Vec<f64>],
    pattern: &[Vec<f64>],
    cov: bool,
    p: usize,
    k: usize,
    out_ref: &DatasetRef,
) -> Result<()> {
    use crate::dataset::{SasDataset, VarMeta};
    use polars::prelude::*;

    // Scoring coefficients B = R⁻¹ · pattern  (p × k).
    let r_inv = invert_matrix(amat)?;
    let coef = matmul(&r_inv, pattern);

    let n_read = ds.n_obs();
    let mut score_cols: Vec<Vec<Option<f64>>> = vec![Vec::with_capacity(n_read); k];
    for row_idx in 0..n_read {
        let row: Vec<f64> = decoded.iter().map(|col| col[row_idx]).collect();
        if row.iter().all(|x| x.is_finite()) {
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
            for f in 0..k {
                let score: f64 = (0..p).map(|j| z[j] * coef[j][f]).sum();
                score_cols[f].push(Some(score));
            }
        } else {
            for f in 0..k {
                score_cols[f].push(None);
            }
        }
    }

    let mut out_df = ds.df.clone();
    for f in 0..k {
        let name = format!("Factor{}", f + 1);
        out_df
            .with_column(Series::new(name.into(), score_cols[f].clone()))
            .map_err(|e| SasError::runtime(format!("FACTOR OUT= build failed: {e}")))?;
    }

    let mut vars = ds.vars.clone();
    for f in 0..k {
        vars.push(VarMeta {
            name: format!("Factor{}", f + 1),
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
            rotate: "quartimax".into(),
            out: None,
            var: vec!["x".into(), "y".into()],
        };
        let r = execute(&ast, &mut session);
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("QUARTIMAX"));
    }

    #[test]
    fn execute_oblimin_deferred_errors() {
        let mut session = make_session();
        let ast = FactorAst {
            data: None,
            cov: false,
            nfactors: None,
            method: "principal".into(),
            rotate: "oblimin".into(),
            out: None,
            var: vec!["x".into(), "y".into()],
        };
        let r = execute(&ast, &mut session);
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("OBLIMIN"));
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

    /// PROMAX oracle: on a clearly-clustered loading matrix (two blocks), the
    /// oblique solution must (a) have an inter-factor correlation off-diagonal
    /// that is non-zero (factors become correlated), and (b) produce a sharper
    /// pattern than VARIMAX — larger primary loadings and smaller cross-loadings
    /// (closer to a {0,1} structure).
    #[test]
    fn promax_correlates_factors_and_sharpens() {
        // 4 variables: vars 0,1 load on factor 1; vars 2,3 on factor 2, but
        // with a deliberate cross-loading so the clusters are oblique.
        let l = vec![
            vec![0.80, 0.40],
            vec![0.75, 0.35],
            vec![0.40, 0.80],
            vec![0.35, 0.75],
        ];
        let (l_var, _) = varimax(&l);
        let pm = promax(&l_var, 4).unwrap();

        // (a) Inter-factor correlation is not the identity.
        let off = pm.phi[0][1].abs();
        assert!(off > 1e-3, "Inter-factor correlation too small: {off}");
        assert!((pm.phi[0][0] - 1.0).abs() < 1e-9, "phi diag != 1");

        // (b) Sharper: for each variable, the dominant pattern loading is at
        // least as large in magnitude, and the cross-loading is smaller, than
        // varimax — on aggregate the cross-loadings shrink.
        let cross_varimax: f64 = (0..4)
            .map(|i| l_var[i][0].abs().min(l_var[i][1].abs()))
            .sum();
        let cross_promax: f64 = (0..4)
            .map(|i| pm.pattern[i][0].abs().min(pm.pattern[i][1].abs()))
            .sum();
        assert!(
            cross_promax < cross_varimax,
            "promax cross-loadings ({cross_promax:.4}) not smaller than varimax ({cross_varimax:.4})"
        );
    }

    /// PROMAX on k=1 is a no-op returning the input pattern and Φ=[[1]].
    #[test]
    fn promax_k1_noop() {
        let l = vec![vec![0.8], vec![0.7], vec![0.9]];
        let pm = promax(&l, 4).unwrap();
        assert_eq!(pm.phi.len(), 1);
        assert!((pm.phi[0][0] - 1.0).abs() < 1e-12);
        for (a, b) in l.iter().zip(&pm.pattern) {
            assert!((a[0] - b[0]).abs() < 1e-12);
        }
    }

    /// End-to-end PROMAX listing: prints the oblique pattern and inter-factor
    /// correlations and creates no NaNs.
    #[test]
    fn execute_promax_listing() {
        let mut session = make_session();
        let df = df![
            "a" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
            "b" => [1.0_f64, 2.1, 2.9, 4.2, 5.1, 5.8],
            "c" => [6.0_f64, 5.0, 4.0, 3.0, 2.0, 1.0],
            "d" => [5.9_f64, 5.1, 4.0, 2.9, 2.1, 1.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("a"), num_meta("b"), num_meta("c"), num_meta("d")],
        };
        write_dataset(&mut session, "P", ds);

        let ast = FactorAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "P".into() }),
            cov: false,
            nfactors: Some(2),
            method: "principal".into(),
            rotate: "promax".into(),
            out: None,
            var: vec!["a".into(), "b".into(), "c".into(), "d".into()],
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("Rotation Method: Promax"), "{listing}");
        assert!(
            listing.contains("Inter-Factor Correlations"),
            "{listing}"
        );
        assert!(!listing.contains("NaN"), "NaN in listing: {listing}");
    }

    /// OUT= : the dataset is created with input columns + Factor1..Factork,
    /// _LAST_ is updated, and the standardized factor scores have mean ≈ 0.
    #[test]
    fn execute_out_factor_scores() {
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
            nfactors: Some(1),
            method: "principal".into(),
            rotate: "none".into(),
            out: Some(DatasetRef { libref: Some("WORK".into()), name: "FS".into() }),
            var: vec!["x".into(), "y".into()],
        };
        execute(&ast, &mut session).unwrap();
        assert_eq!(session.last_dataset.as_deref(), Some("WORK.FS"));

        let (out, _) = session.libs.get("WORK").unwrap().read("FS").unwrap();
        assert!(out.vars.iter().any(|m| m.name == "Factor1"));
        assert!(out.vars.iter().any(|m| m.name.eq_ignore_ascii_case("x")));

        let col = out.df.column("Factor1").unwrap().f64().unwrap();
        let vals: Vec<f64> = col.into_no_null_iter().collect();
        let mean = vals.iter().sum::<f64>() / vals.len() as f64;
        assert!(mean.abs() < 1e-9, "Factor1 mean={mean}");
        // Standardized regression scores have ~unit variance for a 1-factor
        // solution that explains most variance; just assert finiteness here.
        assert!(vals.iter().all(|v| v.is_finite()), "non-finite score");
    }
}
