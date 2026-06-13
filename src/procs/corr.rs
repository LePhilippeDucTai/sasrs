//! PROC CORR — Pearson product-moment correlations (v1).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc corr data=<ref> [nosimple] [noprob] [nocorr];
//!  var <name list>; [with <name list>;] run;`
//!
//! ## Périmètre v1 (fidèle à SAS 9.4 PROC CORR, Pearson uniquement)
//! - Options du statement PROC : `data=`, `nosimple`, `noprob`, `nocorr`.
//! - `var`  : variables analysées (défaut = toutes les numériques du dataset,
//!   dans l'ordre du dataset).
//! - `with` : facultatif. Présent → lignes de la matrice = variables WITH,
//!   colonnes = variables VAR. Absent → matrice carrée symétrique sur VAR.
//!
//! ## Sortie listing (titre "The CORR Procedure"), dans l'ordre SAS :
//! 1. Ligne récapitulative des variables analysées. SAS imprime
//!    `N Variables: ...` (sans WITH) ou bien `N With Variables:` / `M Variables:`
//!    (avec WITH). On reproduit ce style.
//! 2. Table « Simple Statistics » (sauf `nosimple`) : une ligne par variable
//!    analysée (union WITH ∪ VAR, dans l'ordre), colonnes Variable, N, Mean,
//!    Std Dev, Sum, Minimum, Maximum. `sample_std` (n-1), missing exclus via
//!    `partition_numeric`.
//! 3. Matrice « Pearson Correlation Coefficients » (sauf `nocorr`) : lignes =
//!    WITH (ou VAR), colonnes = VAR. Chaque cellule : r à 5 décimales ; sous
//!    r, la p-value `Prob > |r|` (test t bilatéral) sauf `noprob` ; et, quand
//!    les N par paire diffèrent, une 3e ligne avec N. Observations
//!    **pairwise-complete** (on retire toute ligne où l'une des deux variables
//!    est missing pour la paire). Variable constante (variance nulle) → r
//!    missing → SAS imprime `.`.
//!
//! ## Choix / simplifications documentés (pour l'orchestrateur)
//! - La p-value diagonale (r==1 exact, même variable) est imprimée par SAS
//!   sans valeur numérique : on suit SAS et imprimons une cellule vide pour
//!   la prob de la diagonale. (SAS laisse la ligne Prob vide sur la diagonale
//!   d'une matrice symétrique VAR×VAR ; pour WITH×VAR où une variable WITH est
//!   aussi VAR la même règle s'applique : r==1 exact, prob vide.)
//! - t-CDF : survie de la loi de Student via la fonction bêta incomplète
//!   régularisée I_x(a,b) (fraction continue de Lentz). Précision ~1e-10 sur
//!   la plage utile ; voir `student_t_sf` et `betai`. Formatage SAS : valeurs
//!   < 0.0001 → `<.0001`, sinon 4 décimales.
//! - OUT= : NON implémenté dans cet incrément (ajout futur). Le parser ne
//!   l'accepte pas silencieusement : `out=` provoque une erreur claire
//!   « not yet implemented » plutôt qu'un no-op.
//! - En-têtes de table : on s'appuie sur `ListingWriter::write_table` ; les
//!   cellules multi-lignes (r / prob / N) de la matrice sont rendues avec une
//!   ligne de tableau par composante (r, puis prob, puis N) pour rester dans
//!   le moule monospace existant.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::parser::StatementStream;
use crate::procs::common::{decode_column, partition_numeric, sample_std};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{format_best, Value, VarType};

pub struct CorrAst {
    pub data: Option<DatasetRef>,
    pub nosimple: bool,
    pub noprob: bool,
    pub nocorr: bool,
    /// Explicit VAR list (empty = default to all numeric variables).
    pub var: Vec<String>,
    /// Optional WITH list (empty = none).
    pub with: Vec<String>,
}

/// Parse `proc corr [data=a] [nosimple] [noprob] [nocorr];
/// [var ...;] [with ...;] ... run;`. Called AFTER "proc corr" was consumed.
/// Consumes through `run;` / `quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<CorrAst> {
    let mut data: Option<DatasetRef> = None;
    let mut nosimple = false;
    let mut noprob = false;
    let mut nocorr = false;

    // --- PROC CORR statement options, until `;` ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            ts.next();
            expect_eq(ts, "DATA")?;
            data = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("nosimple") {
            ts.next();
            nosimple = true;
        } else if ts.peek().is_kw("noprob") {
            ts.next();
            noprob = true;
        } else if ts.peek().is_kw("nocorr") {
            ts.next();
            nocorr = true;
        } else if ts.peek().is_kw("out") || ts.peek().is_kw("outp") {
            return Err(SasError::runtime(
                "The OUT=/OUTP= option of PROC CORR is not yet implemented.",
            ));
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!(
                    "Unexpected option '{}' on PROC CORR statement.",
                    name.to_uppercase()
                ),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC CORR statement.",
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; ---
    let mut var: Vec<String> = Vec::new();
    let mut with: Vec<String> = Vec::new();

    loop {
        while ts.peek().kind == TokenKind::Semi {
            ts.next();
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("run") || ts.peek().is_kw("quit") {
            ts.next();
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
        }

        if ts.peek().is_kw("var") {
            ts.next();
            var = ts.parse_name_list()?;
            ts.expect_semi()?;
        } else if ts.peek().is_kw("with") {
            ts.next();
            with = ts.parse_name_list()?;
            ts.expect_semi()?;
        } else {
            // Unknown sub-statement: skip it (recovery, like means/print).
            ts.skip_to_semi();
        }
    }

    Ok(CorrAst {
        data,
        nosimple,
        noprob,
        nocorr,
        var,
        with,
    })
}

fn expect_eq(ts: &mut StatementStream, opt: &str) -> Result<()> {
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(
            format!("expected '=' after {opt}"),
            ts.peek().span,
        ));
    }
    ts.next();
    Ok(())
}

/// Resolve `data=` or `_LAST_` into a concrete DatasetRef.
fn resolve_input(ast: &CorrAst, session: &Session) -> Result<DatasetRef> {
    match &ast.data {
        Some(r) => Ok(r.clone()),
        None => {
            let last = session.last_dataset.clone().ok_or_else(|| {
                SasError::runtime("There is no default input data set (_LAST_ is undefined).")
            })?;
            let parts: Vec<&str> = last.splitn(2, '.').collect();
            if parts.len() == 2 {
                Ok(DatasetRef {
                    libref: Some(parts[0].to_string()),
                    name: parts[1].to_string(),
                })
            } else {
                Ok(DatasetRef {
                    libref: None,
                    name: last,
                })
            }
        }
    }
}

// ───────────────────────── numeric core ─────────────────────────

/// Pearson r over pairwise-complete observations. Returns (r, n) where n is
/// the number of complete pairs. r is `None` when n < 2 or either variable
/// has zero variance (constant) over the pairwise-complete set.
fn pearson(xcol: &[Value], ycol: &[Value]) -> (Option<f64>, usize) {
    use crate::missing::value_to_num;
    let mut xs: Vec<f64> = Vec::new();
    let mut ys: Vec<f64> = Vec::new();
    let n_rows = xcol.len().min(ycol.len());
    for i in 0..n_rows {
        let xv = value_to_num(&xcol[i]);
        let yv = value_to_num(&ycol[i]);
        match (xv, yv) {
            (Some(x), Some(y)) if !x.is_nan() && !y.is_nan() => {
                xs.push(x);
                ys.push(y);
            }
            _ => {}
        }
    }
    let n = xs.len();
    if n < 2 {
        return (None, n);
    }
    let nf = n as f64;
    let mx = xs.iter().sum::<f64>() / nf;
    let my = ys.iter().sum::<f64>() / nf;
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    let mut syy = 0.0;
    for k in 0..n {
        let dx = xs[k] - mx;
        let dy = ys[k] - my;
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    if sxx <= 0.0 || syy <= 0.0 {
        // Constant variable → correlation undefined.
        return (None, n);
    }
    let r = sxy / (sxx.sqrt() * syy.sqrt());
    // Clamp tiny FP excursions outside [-1, 1].
    let r = r.clamp(-1.0, 1.0);
    (Some(r), n)
}

/// Two-sided p-value for Pearson r with n pairwise-complete observations:
/// t = r*sqrt((n-2)/(1-r^2)), p = P(|T_{n-2}| > |t|). Returns None when the
/// test is undefined (n < 3, or |r| == 1 exactly).
fn pearson_pvalue(r: f64, n: usize) -> Option<f64> {
    if n < 3 {
        return None;
    }
    let df = (n - 2) as f64;
    if r.abs() >= 1.0 {
        return Some(0.0);
    }
    let t = r * (df / (1.0 - r * r)).sqrt();
    Some(student_t_sf_two_sided(t.abs(), df))
}

/// Two-sided survival function of Student's t: P(|T_df| > t) for t >= 0.
/// Uses the identity P(|T| > t) = I_{df/(df+t^2)}(df/2, 1/2), where I is the
/// regularized incomplete beta function. Accurate to ~1e-10 over the usual
/// range of t and df encountered here.
fn student_t_sf_two_sided(t: f64, df: f64) -> f64 {
    if t <= 0.0 {
        return 1.0;
    }
    let x = df / (df + t * t);
    betai(df / 2.0, 0.5, x)
}

/// Regularized incomplete beta function I_x(a, b), x in [0,1].
/// Numerical Recipes-style continued-fraction evaluation (Lentz), with the
/// standard symmetry I_x(a,b) = 1 - I_{1-x}(b,a) for fast convergence.
fn betai(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let ln_beta = ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b);
    let front = (a * x.ln() + b * (1.0 - x).ln() + ln_beta).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        front * betacf(a, b, x) / a
    } else {
        1.0 - front * betacf(b, a, 1.0 - x) / b
    }
}

/// Continued fraction for the incomplete beta function (Lentz's algorithm).
fn betacf(a: f64, b: f64, x: f64) -> f64 {
    const MAXIT: usize = 300;
    const EPS: f64 = 3.0e-15;
    const FPMIN: f64 = 1.0e-300;

    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < FPMIN {
        d = FPMIN;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=MAXIT {
        let m = m as f64;
        let m2 = 2.0 * m;
        // Even step.
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        h *= d * c;
        // Odd step.
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            break;
        }
    }
    h
}

/// Lanczos approximation of ln Γ(x) for x > 0. Accuracy ~1e-13.
fn ln_gamma(x: f64) -> f64 {
    // Coefficients g=7, n=9 (Numerical Recipes).
    const COF: [f64; 6] = [
        76.18009172947146,
        -86.50532032941677,
        24.01409824083091,
        -1.231739572450155,
        0.1208650973866179e-2,
        -0.5395239384953e-5,
    ];
    let mut y = x;
    let tmp = x + 5.5 - (x + 0.5) * (x + 5.5).ln();
    let mut ser = 1.000000000190015;
    for c in COF.iter() {
        y += 1.0;
        ser += c / y;
    }
    -tmp + (2.5066282746310005 * ser / x).ln()
}

// ───────────────────────── formatting ─────────────────────────

/// Format a correlation r to 5 decimals, SAS-style. Missing → ".".
fn fmt_r(r: Option<f64>) -> String {
    match r {
        Some(v) => format!("{v:.5}"),
        None => ".".to_string(),
    }
}

/// Format a two-sided p-value SAS-style: `<.0001`, else 4 decimals. None
/// (undefined, e.g. on an exact-1 diagonal) → empty cell.
fn fmt_p(p: Option<f64>) -> String {
    match p {
        None => String::new(),
        Some(v) => {
            if v < 0.0001 {
                "<.0001".to_string()
            } else {
                format!("{v:.4}")
            }
        }
    }
}

// ───────────────────────── execute ─────────────────────────

pub fn execute(ast: &CorrAst, session: &mut Session) -> Result<()> {
    let in_ref = resolve_input(ast, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }

    let n_obs = ds.n_obs();

    // Resolve VAR list: explicit, else all numeric vars in dataset order.
    let resolve_names = |names: &[String]| -> Result<Vec<usize>> {
        let mut out = Vec::with_capacity(names.len());
        for nm in names {
            match ds.vars.iter().position(|m| m.name.eq_ignore_ascii_case(nm)) {
                Some(i) => {
                    if ds.vars[i].ty != VarType::Num {
                        return Err(SasError::runtime(format!(
                            "Variable {} in the VAR or WITH list is not numeric.",
                            nm.to_uppercase()
                        )));
                    }
                    out.push(i);
                }
                None => {
                    return Err(SasError::runtime(format!(
                        "Variable {} not found.",
                        nm.to_uppercase()
                    )));
                }
            }
        }
        Ok(out)
    };

    let var_cols: Vec<usize> = if !ast.var.is_empty() {
        resolve_names(&ast.var)?
    } else {
        (0..ds.vars.len())
            .filter(|&i| ds.vars[i].ty == VarType::Num)
            .collect()
    };

    if var_cols.is_empty() {
        return Err(SasError::runtime(
            "No numeric variables found for PROC CORR analysis.",
        ));
    }

    let with_cols: Vec<usize> = if !ast.with.is_empty() {
        resolve_names(&ast.with)?
    } else {
        Vec::new()
    };

    // Matrix rows = WITH (or VAR if no WITH); columns = VAR.
    let row_cols: Vec<usize> = if with_cols.is_empty() {
        var_cols.clone()
    } else {
        with_cols.clone()
    };
    let col_cols: Vec<usize> = var_cols.clone();

    // Analysis variables for Simple Statistics = union(rows, cols) in order:
    // WITH first (if any), then VAR, skipping duplicates by column index.
    let mut analysis_cols: Vec<usize> = Vec::new();
    for &c in row_cols.iter().chain(col_cols.iter()) {
        if !analysis_cols.contains(&c) {
            analysis_cols.push(c);
        }
    }

    // Decode each needed column exactly once.
    let mut decoded: std::collections::HashMap<usize, Vec<Value>> =
        std::collections::HashMap::new();
    for &c in &analysis_cols {
        decoded.insert(c, decode_column(&ds, c)?);
    }

    // --- listing ---
    session.listing.page_header();
    centered(session, "The CORR Procedure");
    session.listing.blank();

    // Variable summary line(s), SAS style.
    if with_cols.is_empty() {
        let names: Vec<String> = var_cols.iter().map(|&c| ds.vars[c].name.clone()).collect();
        session.listing.write_line(&format!(
            "{} Variables:  {}",
            var_cols.len(),
            names.join(" ")
        ));
    } else {
        let wnames: Vec<String> = with_cols.iter().map(|&c| ds.vars[c].name.clone()).collect();
        let vnames: Vec<String> = var_cols.iter().map(|&c| ds.vars[c].name.clone()).collect();
        session.listing.write_line(&format!(
            "{} With Variables:  {}",
            with_cols.len(),
            wnames.join(" ")
        ));
        session.listing.write_line(&format!(
            "{} Variables:  {}",
            var_cols.len(),
            vnames.join(" ")
        ));
    }
    session.listing.blank();

    // --- Simple Statistics ---
    if !ast.nosimple {
        emit_simple_statistics(session, &ds, &analysis_cols, &decoded, n_obs);
    }

    // --- Pearson Correlation Coefficients ---
    if !ast.nocorr {
        emit_correlations(
            session, &ds, &row_cols, &col_cols, &decoded, ast.noprob,
        );
    }

    Ok(())
}

/// Write a centered line within LINESIZE.
fn centered(session: &mut Session, text: &str) {
    let ls = session.listing.ls;
    let pad = ls.saturating_sub(text.len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), text));
}

fn emit_simple_statistics(
    session: &mut Session,
    ds: &crate::dataset::SasDataset,
    analysis_cols: &[usize],
    decoded: &std::collections::HashMap<usize, Vec<Value>>,
    n_obs: usize,
) {
    centered(session, "Simple Statistics");
    session.listing.blank();

    let headers: Vec<String> = vec![
        "Variable".into(),
        "N".into(),
        "Mean".into(),
        "Std Dev".into(),
        "Sum".into(),
        "Minimum".into(),
        "Maximum".into(),
    ];
    let aligns = vec![
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];

    let all_rows: Vec<usize> = (0..n_obs).collect();
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(analysis_cols.len());
    for &c in analysis_cols {
        let col = &decoded[&c];
        let (xs, _nmiss) = partition_numeric(col, &all_rows);
        let n = xs.len();
        let mean = if n > 0 {
            Some(xs.iter().sum::<f64>() / n as f64)
        } else {
            None
        };
        let sum = xs.iter().sum::<f64>();
        let std = sample_std(&xs);
        let min = xs.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

        let cell_opt = |v: Option<f64>| -> String {
            match v {
                Some(f) => format_best(f, 12),
                None => ".".to_string(),
            }
        };

        rows.push(vec![
            ds.vars[c].name.clone(),
            format!("{n}"),
            cell_opt(mean),
            cell_opt(std),
            format_best(sum, 12),
            cell_opt(if n > 0 { Some(min) } else { None }),
            cell_opt(if n > 0 { Some(max) } else { None }),
        ]);
    }

    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
}

fn emit_correlations(
    session: &mut Session,
    ds: &crate::dataset::SasDataset,
    row_cols: &[usize],
    col_cols: &[usize],
    decoded: &std::collections::HashMap<usize, Vec<Value>>,
    noprob: bool,
) {
    centered(session, "Pearson Correlation Coefficients");
    if !noprob {
        centered(session, "Prob > |r| under H0: Rho=0");
    }
    session.listing.blank();

    // Compute r, p, n for every (row, col) cell.
    let nr = row_cols.len();
    let nc = col_cols.len();
    let mut rmat = vec![vec![None::<f64>; nc]; nr];
    let mut pmat = vec![vec![None::<f64>; nc]; nr];
    let mut nmat = vec![vec![0usize; nc]; nr];

    for (i, &rc) in row_cols.iter().enumerate() {
        for (j, &cc) in col_cols.iter().enumerate() {
            if rc == cc {
                // Same variable: r is exactly 1, p undefined (SAS leaves it
                // blank on the diagonal). N = non-missing count of the var.
                let col = &decoded[&rc];
                let (xs, _) = partition_numeric(col, &(0..col.len()).collect::<Vec<_>>());
                rmat[i][j] = Some(1.0);
                pmat[i][j] = None;
                nmat[i][j] = xs.len();
            } else {
                let (r, n) = pearson(&decoded[&rc], &decoded[&cc]);
                rmat[i][j] = r;
                nmat[i][j] = n;
                pmat[i][j] = match r {
                    Some(rv) => pearson_pvalue(rv, n),
                    None => None,
                };
            }
        }
    }

    // Decide whether to print the per-cell N line: only when pairwise N
    // differs across the matrix (SAS prints N only when observations vary).
    let max_n = nmat.iter().flatten().copied().max().unwrap_or(0);
    let any_n_differs = nmat.iter().flatten().any(|&n| n != max_n);

    // Build the table. Column 0 is the row-variable label; each subsequent
    // column is one VAR. Each matrix row expands into up to 3 table rows:
    // r, prob (unless noprob), and N (only when any_n_differs).
    let mut headers: Vec<String> = Vec::with_capacity(nc + 1);
    headers.push(String::new());
    let mut aligns: Vec<Align> = Vec::with_capacity(nc + 1);
    aligns.push(Align::Left);
    for &cc in col_cols {
        headers.push(ds.vars[cc].name.clone());
        aligns.push(Align::Right);
    }

    let mut rows: Vec<Vec<String>> = Vec::new();
    for i in 0..nr {
        // r line, labelled with the row variable.
        let mut rline = vec![ds.vars[row_cols[i]].name.clone()];
        for j in 0..nc {
            rline.push(fmt_r(rmat[i][j]));
        }
        rows.push(rline);

        if !noprob {
            let mut pline = vec![String::new()];
            for j in 0..nc {
                pline.push(fmt_p(pmat[i][j]));
            }
            rows.push(pline);
        }

        if any_n_differs {
            let mut nline = vec![String::new()];
            for j in 0..nc {
                nline.push(format!("{}", nmat[i][j]));
            }
            rows.push(nline);
        }
    }

    session.listing.write_table(&headers, &aligns, &rows);
    session.listing.blank();
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

    fn char_meta(name: &str) -> VarMeta {
        VarMeta {
            name: name.to_string(),
            ty: VarType::Char,
            length: 4,
            format: None,
            label: None,
        }
    }

    fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
        session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
        session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
    }

    fn parse_corr(src: &str) -> Result<CorrAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "corr"
        parse(&mut ts)
    }

    // ───────────── parse tests ─────────────

    #[test]
    fn parse_minimal() {
        let ast = parse_corr("proc corr data=a; run;").unwrap();
        assert_eq!(ast.data.as_ref().unwrap().name, "a");
        assert!(!ast.nosimple && !ast.noprob && !ast.nocorr);
        assert!(ast.var.is_empty() && ast.with.is_empty());
    }

    #[test]
    fn parse_options_and_statements() {
        let ast =
            parse_corr("proc corr data=a nosimple noprob nocorr; var x y; with z; run;").unwrap();
        assert!(ast.nosimple && ast.noprob && ast.nocorr);
        assert_eq!(ast.var, vec!["x", "y"]);
        assert_eq!(ast.with, vec!["z"]);
    }

    #[test]
    fn parse_unknown_option_errors() {
        let r = parse_corr("proc corr data=a bogus; run;");
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("BOGUS"));
    }

    #[test]
    fn parse_out_not_implemented() {
        let r = parse_corr("proc corr data=a out=b; run;");
        assert!(r.is_err());
        assert!(r
            .err()
            .unwrap()
            .to_string()
            .contains("not yet implemented"));
    }

    // ───────────── numeric core tests ─────────────

    #[test]
    fn pearson_perfect_positive() {
        let x: Vec<Value> = [1.0, 2.0, 3.0, 4.0].iter().map(|v| Value::Num(*v)).collect();
        let y: Vec<Value> = [2.0, 4.0, 6.0, 8.0].iter().map(|v| Value::Num(*v)).collect();
        let (r, n) = pearson(&x, &y);
        assert_eq!(n, 4);
        assert!((r.unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn pearson_perfect_negative() {
        let x: Vec<Value> = [1.0, 2.0, 3.0].iter().map(|v| Value::Num(*v)).collect();
        let y: Vec<Value> = [3.0, 2.0, 1.0].iter().map(|v| Value::Num(*v)).collect();
        let (r, _) = pearson(&x, &y);
        assert!((r.unwrap() + 1.0).abs() < 1e-12);
    }

    #[test]
    fn pearson_hand_computed() {
        // x=[1,2,3,5], y=[2,1,4,3].
        let x: Vec<Value> = [1.0, 2.0, 3.0, 5.0].iter().map(|v| Value::Num(*v)).collect();
        let y: Vec<Value> = [2.0, 1.0, 4.0, 3.0].iter().map(|v| Value::Num(*v)).collect();
        let (r, n) = pearson(&x, &y);
        assert_eq!(n, 4);
        // mx=2.75, my=2.5; Sxy=3.5, Sxx=8.75, Syy=5 -> r = 3.5/sqrt(43.75) = 0.52915026
        assert!((r.unwrap() - 0.52915026).abs() < 1e-6, "r={:?}", r);
    }

    #[test]
    fn pearson_constant_is_missing() {
        let x: Vec<Value> = [5.0, 5.0, 5.0].iter().map(|v| Value::Num(*v)).collect();
        let y: Vec<Value> = [1.0, 2.0, 3.0].iter().map(|v| Value::Num(*v)).collect();
        let (r, n) = pearson(&x, &y);
        assert_eq!(n, 3);
        assert!(r.is_none());
    }

    #[test]
    fn pearson_pairwise_complete_n() {
        // Drop rows where either is missing.
        let x = vec![
            Value::Num(1.0),
            Value::Num(2.0),
            Value::missing(),
            Value::Num(4.0),
        ];
        let y = vec![
            Value::Num(2.0),
            Value::missing(),
            Value::Num(3.0),
            Value::Num(8.0),
        ];
        // Complete pairs: rows 0 and 3 → n=2.
        let (_r, n) = pearson(&x, &y);
        assert_eq!(n, 2);
    }

    #[test]
    fn pvalue_approx_matches_known() {
        // r=0, any n → p = 1.0.
        assert!((pearson_pvalue(0.0, 10).unwrap() - 1.0).abs() < 1e-9);
        // n=3 df=1: r small → p near 1.
        let p = pearson_pvalue(0.5, 12).unwrap();
        // For r=0.5, n=12, df=10: t=0.5*sqrt(10/0.75)=1.8257; p≈0.0978.
        assert!((p - 0.0978).abs() < 1e-3, "p={p}");
    }

    #[test]
    fn betai_symmetry_and_bounds() {
        // I_0 = 0, I_1 = 1.
        assert!(betai(2.0, 3.0, 0.0).abs() < 1e-12);
        assert!((betai(2.0, 3.0, 1.0) - 1.0).abs() < 1e-12);
        // I_x(a,b) + I_{1-x}(b,a) = 1.
        let s = betai(2.5, 4.0, 0.3) + betai(4.0, 2.5, 0.7);
        assert!((s - 1.0).abs() < 1e-9, "sum={s}");
    }

    #[test]
    fn fmt_helpers() {
        assert_eq!(fmt_r(Some(1.0)), "1.00000");
        assert_eq!(fmt_r(Some(0.9583)), "0.95830");
        assert_eq!(fmt_r(None), ".");
        assert_eq!(fmt_p(Some(0.00001)), "<.0001");
        assert_eq!(fmt_p(Some(0.1234)), "0.1234");
        assert_eq!(fmt_p(None), "");
    }

    // ───────────── execute tests ─────────────

    #[test]
    fn execute_perfect_correlation_listing() {
        let mut session = make_session();
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0, 4.0],
            "y" => [2.0_f64, 4.0, 6.0, 8.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x"), num_meta("y")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = CorrAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            nosimple: false,
            noprob: false,
            nocorr: false,
            var: vec![],
            with: vec![],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("The CORR Procedure"), "{listing}");
        assert!(listing.contains("Simple Statistics"), "{listing}");
        assert!(listing.contains("Pearson Correlation Coefficients"), "{listing}");
        // Diagonal 1.00000 and off-diagonal 1.00000 (perfectly correlated).
        assert!(listing.contains("1.00000"), "{listing}");
        // Variable summary line.
        assert!(listing.contains("2 Variables:"), "{listing}");
    }

    #[test]
    fn execute_nosimple_noprob_toggles() {
        let mut session = make_session();
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0, 4.0],
            "y" => [1.0_f64, 3.0, 2.0, 5.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x"), num_meta("y")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = CorrAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            nosimple: true,
            noprob: true,
            nocorr: false,
            var: vec!["x".into(), "y".into()],
            with: vec![],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(!listing.contains("Simple Statistics"), "nosimple: {listing}");
        assert!(!listing.contains("Prob > |r|"), "noprob: {listing}");
        assert!(listing.contains("Pearson Correlation Coefficients"), "{listing}");
    }

    #[test]
    fn execute_missing_pairwise_n_line() {
        let mut session = make_session();
        // x and y share 4 complete rows; x and z share only 3 (one missing),
        // so pairwise N differs and the N line should appear.
        let df = df![
            "x" => [Some(1.0_f64), Some(2.0), Some(3.0), Some(4.0)],
            "y" => [Some(2.0_f64), Some(1.0), Some(4.0), Some(3.0)],
            "z" => [Some(1.0_f64), None, Some(2.0), Some(5.0)]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x"), num_meta("y"), num_meta("z")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = CorrAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            nosimple: true,
            noprob: true,
            nocorr: false,
            var: vec!["x".into(), "y".into(), "z".into()],
            with: vec![],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // N line should show a "3" somewhere in the matrix region.
        assert!(listing.contains(" 3"), "expected N line with 3: {listing}");
        assert!(listing.contains(" 4"), "expected N line with 4: {listing}");
    }

    #[test]
    fn execute_constant_variable_missing_r() {
        let mut session = make_session();
        let df = df![
            "x" => [5.0_f64, 5.0, 5.0, 5.0],
            "y" => [1.0_f64, 2.0, 3.0, 4.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x"), num_meta("y")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = CorrAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            nosimple: true,
            noprob: true,
            nocorr: false,
            var: vec!["x".into(), "y".into()],
            with: vec![],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // Off-diagonal r between constant x and y is missing → ".".
        assert!(listing.contains(" ."), "expected missing r '.': {listing}");
    }

    #[test]
    fn execute_with_statement_shapes_matrix() {
        let mut session = make_session();
        let df = df![
            "a" => [1.0_f64, 2.0, 3.0, 4.0],
            "b" => [4.0_f64, 3.0, 2.0, 1.0],
            "w" => [1.0_f64, 2.0, 3.0, 4.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("a"), num_meta("b"), num_meta("w")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = CorrAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            nosimple: true,
            noprob: true,
            nocorr: false,
            var: vec!["a".into(), "b".into()],
            with: vec!["w".into()],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("1 With Variables:"), "{listing}");
        assert!(listing.contains("2 Variables:"), "{listing}");
        // w perfectly correlates with a (1.00000) and anti with b (-1.00000).
        assert!(listing.contains("1.00000"), "{listing}");
        assert!(listing.contains("-1.00000"), "{listing}");
    }

    #[test]
    fn execute_default_var_all_numeric() {
        let mut session = make_session();
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0],
            "g" => ["a", "b", "c"],
            "y" => [3.0_f64, 2.0, 1.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x"), char_meta("g"), num_meta("y")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = CorrAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            nosimple: false,
            noprob: true,
            nocorr: false,
            var: vec![],
            with: vec![],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // Only x and y (numeric) are analyzed; char g excluded.
        assert!(listing.contains("2 Variables:"), "{listing}");
        assert!(listing.contains("x y"), "{listing}");
    }
}
