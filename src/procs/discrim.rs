//! PROC DISCRIM — Fisher's linear discriminant analysis (M27).
//!
//! Supports (pool=yes / METHOD=NORMAL):
//! - CLASS statement (group variable, char or numeric).
//! - VAR statement (numeric predictors).
//! - ID statement (label for the classification listing).
//! - PRIORS EQUAL (default) / PRIORS PROPORTIONAL.
//! - OUT= dataset with `_FROM_`, `_INTO_` and one `_<k>` posterior per class.
//!
//! Produces: header counts, Class Level Information, Within-Class Covariance
//! Matrix (per class), Pooled Within-Class Covariance Matrix, Pairwise Squared
//! Distances Between Groups, Linear Discriminant Function Coefficients,
//! Classification Results for Training Data, Error Count Estimates.
//!
//! Parse-accepted but not implemented (NOTE emitted): METHOD other than NORMAL,
//! POOL=NO/TEST (QDA deferred), OUTSTAT=, NOCLASSIFY, CROSSVALIDATE, SHORT.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column};
use crate::session::Session;
use crate::stat::invert_matrix;
use crate::token::TokenKind;
use crate::value::{format_best, Value};

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Priors {
    Equal,
    Proportional,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pool {
    Yes,
    No,
    Test,
}

#[derive(Debug, Clone)]
pub struct DiscrimAst {
    pub data: Option<DatasetRef>,
    pub out: Option<DatasetRef>,
    pub outstat: Option<DatasetRef>,
    pub method: Option<String>,
    pub pool: Pool,
    pub priors: Priors,
    pub noclassify: bool,
    pub crossvalidate: bool,
    pub short: bool,
    pub class_var: Option<String>,
    pub var_vars: Vec<String>,
    pub id_var: Option<String>,
}

// ───────────────────────── Parser helpers ─────────────────────────

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

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC DISCRIM. Called AFTER `proc discrim` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<DiscrimAst> {
    let mut data: Option<DatasetRef> = None;
    let mut out: Option<DatasetRef> = None;
    let mut outstat: Option<DatasetRef> = None;
    let mut method: Option<String> = None;
    let mut pool = Pool::Yes;
    let mut priors = Priors::Equal;
    let mut noclassify = false;
    let mut crossvalidate = false;
    let mut short = false;

    // PROC DISCRIM statement options, until `;`
    loop {
        let tk = ts.peek();
        if tk.kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if tk.kind == TokenKind::Eof {
            break;
        }
        if tk.is_kw("data") {
            data = Some(common::parse_dataset_opt(ts, "DATA")?);
        } else if tk.is_kw("out") {
            out = Some(common::parse_out_opt(ts)?);
        } else if tk.is_kw("outstat") {
            outstat = Some(common::parse_dataset_opt(ts, "OUTSTAT")?);
        } else if tk.is_kw("method") {
            ts.next();
            expect_eq(ts, "METHOD")?;
            method = ts.peek().ident().map(|s| s.to_ascii_uppercase());
            ts.next();
        } else if tk.is_kw("pool") {
            ts.next();
            expect_eq(ts, "POOL")?;
            let v = ts.peek().ident().map(|s| s.to_ascii_lowercase());
            pool = match v.as_deref() {
                Some("no") => Pool::No,
                Some("test") => Pool::Test,
                _ => Pool::Yes,
            };
            ts.next();
        } else if tk.is_kw("noclassify") {
            noclassify = true;
            ts.next();
        } else if tk.is_kw("crossvalidate") {
            crossvalidate = true;
            ts.next();
        } else if tk.is_kw("short") {
            short = true;
            ts.next();
        } else {
            // Skip unknown proc-level options.
            ts.next();
        }
    }

    // Sub-statements until run;/quit;
    let mut class_var: Option<String> = None;
    let mut var_vars: Vec<String> = Vec::new();
    let mut id_var: Option<String> = None;

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "class" => {
                ts.next();
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    class_var = Some(name);
                    ts.next();
                }
                ts.skip_to_semi();
                true
            }
            "var" => {
                ts.next();
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    if let Some(name) = ts.peek().ident().map(str::to_string) {
                        var_vars.push(name);
                        ts.next();
                    } else {
                        ts.next();
                    }
                }
                ts.expect_semi()?;
                true
            }
            "id" => {
                ts.next();
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    id_var = Some(name);
                    ts.next();
                }
                ts.skip_to_semi();
                true
            }
            "priors" => {
                ts.next();
                let v = ts.peek().ident().map(|s| s.to_ascii_lowercase());
                priors = match v.as_deref() {
                    Some("proportional") | Some("prop") => Priors::Proportional,
                    _ => Priors::Equal,
                };
                ts.skip_to_semi();
                true
            }
            _ => false,
        })
    })?;

    Ok(DiscrimAst {
        data,
        out,
        outstat,
        method,
        pool,
        priors,
        noclassify,
        crossvalidate,
        short,
        class_var,
        var_vars,
        id_var,
    })
}

// ───────────────────────── Formatting helpers ─────────────────────────

fn fmt4(v: f64) -> String {
    format!("{v:.4}")
}

fn fmt6(v: f64) -> String {
    format!("{v:.6}")
}

fn centered(session: &mut Session, text: &str) {
    let ls = session.listing.ls();
    let pad = ls.saturating_sub(text.len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), text));
}

/// Format a class-level Value for display.
fn value_label(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.trim_end().to_string(),
    }
}

// ───────────────────────── Linear algebra helpers ─────────────────────────

/// Multiply matrix (m×k) by vector (k) → vector (m).
fn mat_vec(mat: &[Vec<f64>], vec: &[f64]) -> Vec<f64> {
    mat.iter()
        .map(|row| row.iter().zip(vec.iter()).map(|(a, b)| a * b).sum())
        .collect()
}

/// Inner product of two vectors.
fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

// ───────────────────────── Core LDA computation ─────────────────────────

/// Result of fitting the LDA model. All vectors/matrices indexed by class
/// order in `classes`.
struct LdaModel {
    classes: Vec<Value>,
    class_labels: Vec<String>,
    counts: Vec<usize>,
    priors: Vec<f64>,
    means: Vec<Vec<f64>>,         // means[k] = centroid of class k (length p)
    within_cov: Vec<Vec<Vec<f64>>>, // within_cov[k] = S_k (p×p)
    pooled_inv: Vec<Vec<f64>>,   // Σ_pooled⁻¹ (p×p)
    pooled: Vec<Vec<f64>>,       // Σ_pooled (p×p)
    coefs: Vec<Vec<f64>>,        // coefs[k] = Σ⁻¹ μ_k (length p)
    constants: Vec<f64>,         // constants[k]
    n_total: usize,
    n_groups: usize,
    p: usize,
}

impl LdaModel {
    /// Discriminant score for class k at point x.
    fn score(&self, k: usize, x: &[f64]) -> f64 {
        dot(x, &self.coefs[k]) + self.constants[k]
    }

    /// Posterior probabilities (softmax over scores) for point x.
    fn posteriors(&self, x: &[f64]) -> Vec<f64> {
        let scores: Vec<f64> = (0..self.n_groups).map(|k| self.score(k, x)).collect();
        let max = scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let exps: Vec<f64> = scores.iter().map(|&s| (s - max).exp()).collect();
        let sum: f64 = exps.iter().sum();
        exps.iter().map(|&e| e / sum).collect()
    }

    /// argmax class index by score.
    fn classify(&self, x: &[f64]) -> usize {
        let mut best = 0usize;
        let mut best_score = f64::NEG_INFINITY;
        for k in 0..self.n_groups {
            let s = self.score(k, x);
            if s > best_score {
                best_score = s;
                best = k;
            }
        }
        best
    }

    /// Mahalanobis² distance between group i and group j centroids.
    fn group_distance(&self, i: usize, j: usize) -> f64 {
        let diff: Vec<f64> = (0..self.p)
            .map(|d| self.means[i][d] - self.means[j][d])
            .collect();
        let tmp = mat_vec(&self.pooled_inv, &diff);
        dot(&diff, &tmp)
    }
}

/// Sample covariance matrix (denominator n-1) for the rows in `data`.
fn sample_cov(data: &[Vec<f64>], mean: &[f64]) -> Vec<Vec<f64>> {
    let n = data.len();
    let p = mean.len();
    let mut cov = vec![vec![0.0; p]; p];
    if n < 2 {
        return cov;
    }
    for row in data {
        for a in 0..p {
            for b in 0..p {
                cov[a][b] += (row[a] - mean[a]) * (row[b] - mean[b]);
            }
        }
    }
    let denom = (n - 1) as f64;
    for a in 0..p {
        for b in 0..p {
            cov[a][b] /= denom;
        }
    }
    cov
}

/// Fit the LDA model from class-labeled observations.
/// `obs` : one (class_value, predictor-vector) per complete observation.
fn fit_lda(
    classes: Vec<Value>,
    class_obs: &[Vec<Vec<f64>>], // class_obs[k] = rows for class k
    priors_mode: &Priors,
    p: usize,
) -> Result<LdaModel> {
    let n_groups = classes.len();
    let counts: Vec<usize> = class_obs.iter().map(|c| c.len()).collect();
    let n_total: usize = counts.iter().sum();

    // Means per class.
    let means: Vec<Vec<f64>> = class_obs
        .iter()
        .map(|rows| {
            let n = rows.len() as f64;
            let mut m = vec![0.0; p];
            for row in rows {
                for d in 0..p {
                    m[d] += row[d];
                }
            }
            for d in 0..p {
                m[d] /= n;
            }
            m
        })
        .collect();

    // Within-class covariance per class (n_k - 1 denominator).
    let within_cov: Vec<Vec<Vec<f64>>> = class_obs
        .iter()
        .zip(means.iter())
        .map(|(rows, m)| sample_cov(rows, m))
        .collect();

    // Pooled covariance = Σ (n_k - 1) S_k / (N - G).
    let df_within = (n_total as i64 - n_groups as i64).max(1) as f64;
    let mut pooled = vec![vec![0.0; p]; p];
    for (k, sk) in within_cov.iter().enumerate() {
        let w = (counts[k] as i64 - 1).max(0) as f64;
        for a in 0..p {
            for b in 0..p {
                pooled[a][b] += w * sk[a][b];
            }
        }
    }
    for a in 0..p {
        for b in 0..p {
            pooled[a][b] /= df_within;
        }
    }

    let pooled_inv = invert_matrix(&pooled)?;

    // Priors.
    let priors: Vec<f64> = match priors_mode {
        Priors::Equal => vec![1.0 / n_groups as f64; n_groups],
        Priors::Proportional => counts
            .iter()
            .map(|&c| c as f64 / n_total as f64)
            .collect(),
    };

    // Coefficients and constants.
    let mut coefs: Vec<Vec<f64>> = Vec::with_capacity(n_groups);
    let mut constants: Vec<f64> = Vec::with_capacity(n_groups);
    for k in 0..n_groups {
        let coef_k = mat_vec(&pooled_inv, &means[k]);
        let const_k = -0.5 * dot(&means[k], &coef_k) + priors[k].ln();
        coefs.push(coef_k);
        constants.push(const_k);
    }

    let class_labels = classes.iter().map(value_label).collect();

    Ok(LdaModel {
        classes,
        class_labels,
        counts,
        priors,
        means,
        within_cov,
        pooled_inv,
        pooled,
        coefs,
        constants,
        n_total,
        n_groups,
        p,
    })
}

// ───────────────────────── Execute ─────────────────────────

pub fn execute(ast: &DiscrimAst, session: &mut Session) -> Result<()> {
    // ── 1. Guards ──────────────────────────────────────────────────────────
    let class_name = ast.class_var.as_ref().ok_or_else(|| {
        SasError::runtime("CLASS statement required in PROC DISCRIM")
    })?;

    if ast.var_vars.is_empty() {
        return Err(SasError::runtime(
            "VAR statement with at least one numeric variable required in PROC DISCRIM",
        ));
    }

    // Parse-accepted options that are not implemented → NOTE.
    if let Some(m) = &ast.method {
        if m != "NORMAL" {
            session.log.note(&format!(
                "METHOD={} is not implemented; using NORMAL (LDA).",
                m
            ));
        }
    }
    match ast.pool {
        Pool::No => session
            .log
            .note("POOL=NO (QDA) is not implemented; using pooled covariance (LDA)."),
        Pool::Test => session
            .log
            .note("POOL=TEST is not implemented; using pooled covariance (LDA)."),
        Pool::Yes => {}
    }
    if ast.outstat.is_some() {
        session
            .log
            .note("OUTSTAT= is parse-accepted but not implemented in PROC DISCRIM.");
    }
    if ast.noclassify {
        session
            .log
            .note("NOCLASSIFY is parse-accepted but not implemented in PROC DISCRIM.");
    }
    if ast.crossvalidate {
        session
            .log
            .note("CROSSVALIDATE is parse-accepted but not implemented in PROC DISCRIM.");
    }
    if ast.short {
        session
            .log
            .note("SHORT is parse-accepted but not implemented in PROC DISCRIM.");
    }

    // ── 2. Read dataset ────────────────────────────────────────────────────
    let in_ref = common::resolve_last_dataset(&ast.data, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }

    let n_read = ds.n_obs();
    session.log.note(&format!(
        "There were {} observations read from the data set {}.{}.",
        n_read, in_libref, in_table
    ));

    let p = ast.var_vars.len();

    // ── Find column indices ────────────────────────────────────────────────
    let find_col = |nm: &str| -> Result<usize> {
        ds.vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(nm))
            .ok_or_else(|| {
                SasError::runtime(format!("Variable {} not found.", nm.to_uppercase()))
            })
    };

    let class_idx = find_col(class_name)?;
    let mut var_idxs: Vec<usize> = Vec::with_capacity(p);
    for nm in &ast.var_vars {
        var_idxs.push(find_col(nm)?);
    }
    let id_idx: Option<usize> = match &ast.id_var {
        Some(nm) => Some(find_col(nm)?),
        None => None,
    };

    // ── Decode columns ─────────────────────────────────────────────────────
    let class_col = decode_column(&ds, class_idx)?;
    let mut var_cols: Vec<Vec<Value>> = Vec::with_capacity(p);
    for &idx in &var_idxs {
        var_cols.push(decode_column(&ds, idx)?);
    }
    let id_col: Option<Vec<Value>> = match id_idx {
        Some(i) => Some(decode_column(&ds, i)?),
        None => None,
    };

    // ── 3. Build complete observations grouped by class ────────────────────
    // Preserve first-seen order? SAS sorts classes by formatted value. Use
    // sas_cmp ordering for deterministic class order.
    let mut classes: Vec<Value> = Vec::new();
    // Keep, per kept observation: (row index in original, class index, x-vec)
    struct Obs {
        orig_row: usize,
        class: Value,
        x: Vec<f64>,
    }
    let mut kept: Vec<Obs> = Vec::new();

    for i in 0..n_read {
        if class_col[i].is_missing() {
            continue;
        }
        let mut x = Vec::with_capacity(p);
        let mut ok = true;
        for vc in &var_cols {
            match value_to_num(&vc[i]) {
                Some(v) if !v.is_nan() => x.push(v),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let cv = class_col[i].clone();
        if !classes
            .iter()
            .any(|c| c.sas_cmp(&cv) == std::cmp::Ordering::Equal)
        {
            classes.push(cv.clone());
        }
        kept.push(Obs {
            orig_row: i,
            class: cv,
            x,
        });
    }

    classes.sort_by(|a, b| a.sas_cmp(b));
    let n_groups = classes.len();

    if n_groups < 2 {
        return Err(SasError::runtime(
            "PROC DISCRIM requires at least 2 classes with complete observations.",
        ));
    }

    // Pre-fit class lookup (classes is moved into the model after fitting; use
    // model.classes via the closure below for post-fit work).
    let class_index_of = |cls: &[Value], v: &Value| -> usize {
        cls.iter()
            .position(|c| c.sas_cmp(v) == std::cmp::Ordering::Equal)
            .unwrap()
    };

    // Group rows per class.
    let mut class_obs: Vec<Vec<Vec<f64>>> = vec![Vec::new(); n_groups];
    for obs in &kept {
        class_obs[class_index_of(&classes, &obs.class)].push(obs.x.clone());
    }

    let n_used = kept.len();
    session
        .log
        .note(&format!("There were {} observations used.", n_used));

    // ── 4. Fit ─────────────────────────────────────────────────────────────
    let model = fit_lda(classes, &class_obs, &ast.priors, p)?;
    let class_index = |v: &Value| -> usize { class_index_of(&model.classes, v) };

    // ── 5. Listing ─────────────────────────────────────────────────────────
    let n = model.n_total;
    let g = model.n_groups;

    session.listing.page_header();
    centered(session, "The DISCRIMINANT Procedure");
    session.listing.blank();

    // Header counts table.
    {
        let headers: Vec<String> = vec![String::new(), String::new(), String::new(), String::new()];
        let aligns = vec![Align::Left, Align::Right, Align::Left, Align::Right];
        let rows: Vec<Vec<String>> = vec![
            vec![
                "Observations".into(),
                n.to_string(),
                "Variables".into(),
                p.to_string(),
            ],
            vec![
                "DF Total".into(),
                (n as i64 - 1).to_string(),
                "Classes".into(),
                g.to_string(),
            ],
            vec![
                "DF Within Classes".into(),
                (n as i64 - g as i64).to_string(),
                "DF Between Classes".into(),
                (g as i64 - 1).to_string(),
            ],
        ];
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // Class Level Information.
    centered(session, "Class Level Information");
    session.listing.blank();
    {
        let headers: Vec<String> = vec![
            class_name.clone(),
            "Variable".into(),
            "Frequency".into(),
            "Weight".into(),
            "Proportion".into(),
        ];
        let aligns = vec![
            Align::Left,
            Align::Left,
            Align::Right,
            Align::Right,
            Align::Right,
        ];
        let mut rows: Vec<Vec<String>> = Vec::with_capacity(g);
        for k in 0..g {
            let prop = model.counts[k] as f64 / n as f64;
            rows.push(vec![
                model.class_labels[k].clone(),
                make_class_var_name(&model.class_labels[k]),
                model.counts[k].to_string(),
                format!("{:.4}", model.counts[k] as f64),
                fmt6(prop),
            ]);
        }
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // Within-Class Covariance Matrix (per class).
    centered(session, "Within-Class Covariance Matrix");
    session.listing.blank();
    for k in 0..g {
        session.listing.write_line(&format!(
            "{}    DF = {}",
            model.class_labels[k],
            model.counts[k] as i64 - 1
        ));
        session.listing.blank();
        write_matrix(session, &ast.var_vars, &model.within_cov[k]);
        session.listing.blank();
    }

    // Pooled Within-Class Covariance Matrix.
    centered(session, "Pooled Within-Class Covariance Matrix");
    session.listing.blank();
    session
        .listing
        .write_line(&format!("DF = {}", n as i64 - g as i64));
    session.listing.blank();
    write_matrix(session, &ast.var_vars, &model.pooled);
    session.listing.blank();

    // Pairwise Squared Distances Between Groups.
    centered(session, "Pairwise Squared Distances Between Groups");
    session.listing.blank();
    {
        let mut headers: Vec<String> = vec![String::new()];
        let mut aligns: Vec<Align> = vec![Align::Left];
        for k in 0..g {
            headers.push(model.class_labels[k].clone());
            aligns.push(Align::Right);
        }
        let mut rows: Vec<Vec<String>> = Vec::with_capacity(g);
        for i in 0..g {
            let mut row = vec![model.class_labels[i].clone()];
            for j in 0..g {
                row.push(fmt4(model.group_distance(i, j)));
            }
            rows.push(row);
        }
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // Linear Discriminant Function Coefficients.
    centered(session, "Linear Discriminant Function Coefficients");
    session.listing.blank();
    {
        let mut headers: Vec<String> = vec!["Variable".into()];
        let mut aligns: Vec<Align> = vec![Align::Left];
        for k in 0..g {
            headers.push(model.class_labels[k].clone());
            aligns.push(Align::Right);
        }
        let mut rows: Vec<Vec<String>> = Vec::new();
        // Constant row.
        let mut crow = vec!["Constant".to_string()];
        for k in 0..g {
            crow.push(fmt4(model.constants[k]));
        }
        rows.push(crow);
        // One row per variable.
        for (d, vname) in ast.var_vars.iter().enumerate() {
            let mut vrow = vec![vname.clone()];
            for k in 0..g {
                vrow.push(fmt4(model.coefs[k][d]));
            }
            rows.push(vrow);
        }
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // ── 6. Classification ──────────────────────────────────────────────────
    // For each kept observation, compute classification + posteriors.
    let mut error_count: Vec<usize> = vec![0; g];

    centered(session, "Classification Results for Training Data");
    session.listing.blank();
    {
        let mut headers: Vec<String> = vec![
            if id_col.is_some() {
                ast.id_var.clone().unwrap()
            } else {
                "Obs".into()
            },
            "From CLASS".into(),
            "Classified Into CLASS".into(),
        ];
        let mut aligns: Vec<Align> = vec![Align::Right, Align::Left, Align::Left];
        for k in 0..g {
            headers.push(model.class_labels[k].clone());
            aligns.push(Align::Right);
        }
        let mut rows: Vec<Vec<String>> = Vec::with_capacity(n_used);
        for (n_obs_idx, obs) in kept.iter().enumerate() {
            let from = class_index(&obs.class);
            let into = model.classify(&obs.x);
            if from != into {
                error_count[from] += 1;
            }
            let post = model.posteriors(&obs.x);
            let label = if let Some(ic) = &id_col {
                value_label(&ic[obs.orig_row])
            } else {
                (n_obs_idx + 1).to_string()
            };
            let mut row = vec![
                label,
                model.class_labels[from].clone(),
                model.class_labels[into].clone(),
            ];
            for k in 0..g {
                row.push(fmt4(post[k]));
            }
            rows.push(row);
        }
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // ── 7. Error Count Estimates ───────────────────────────────────────────
    centered(session, "Error Count Estimates for Training Data");
    session.listing.blank();
    {
        let mut headers: Vec<String> = vec![String::new()];
        let mut aligns: Vec<Align> = vec![Align::Left];
        for k in 0..g {
            headers.push(model.class_labels[k].clone());
            aligns.push(Align::Right);
        }
        headers.push("Total".into());
        aligns.push(Align::Right);

        // Rate row.
        let mut rate_row = vec!["Rate".to_string()];
        let mut total_err = 0usize;
        for k in 0..g {
            let rate = if model.counts[k] > 0 {
                error_count[k] as f64 / model.counts[k] as f64
            } else {
                0.0
            };
            rate_row.push(fmt4(rate));
            total_err += error_count[k];
        }
        // Total rate = Σ priors_k * rate_k (SAS weights error rates by priors).
        let total_rate: f64 = (0..g)
            .map(|k| {
                let rate = if model.counts[k] > 0 {
                    error_count[k] as f64 / model.counts[k] as f64
                } else {
                    0.0
                };
                model.priors[k] * rate
            })
            .sum();
        let _ = total_err;
        rate_row.push(fmt4(total_rate));
        // Priors row.
        let mut priors_row = vec!["Priors".to_string()];
        for k in 0..g {
            priors_row.push(fmt4(model.priors[k]));
        }
        priors_row.push(String::new());

        session
            .listing
            .write_table(&headers, &aligns, &[rate_row, priors_row]);
        session.listing.blank();
    }

    // ── 8. OUT= dataset ────────────────────────────────────────────────────
    if let Some(out_ref) = &ast.out {
        write_out_dataset(ast, session, &ds, &model, &var_cols, &class_col, out_ref, n_read)?;
    }

    Ok(())
}

/// SAS shows the class level value as the "Variable" column in Class Level
/// Information (a valid SAS name derived from the formatted value).
fn make_class_var_name(label: &str) -> String {
    // SAS builds a name like `_A` for value "A". For numeric / messy values it
    // prefixes with an underscore. Keep it simple and SAS-like.
    let trimmed = label.trim();
    if trimmed.is_empty() {
        "_".to_string()
    } else if trimmed.chars().next().map(|c| c.is_alphabetic()).unwrap_or(false) {
        trimmed.to_string()
    } else {
        format!("_{trimmed}")
    }
}

/// Print a labeled square matrix with variable names on rows and columns.
fn write_matrix(session: &mut Session, var_names: &[String], mat: &[Vec<f64>]) {
    let p = var_names.len();
    let mut headers: Vec<String> = vec![String::new()];
    let mut aligns: Vec<Align> = vec![Align::Left];
    for nm in var_names {
        headers.push(nm.clone());
        aligns.push(Align::Right);
    }
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(p);
    for i in 0..p {
        let mut row = vec![var_names[i].clone()];
        for j in 0..p {
            row.push(fmt4(mat[i][j]));
        }
        rows.push(row);
    }
    session.listing.write_table(&headers, &aligns, &rows);
}

/// Build and write the OUT= dataset: input columns + `_FROM_`, `_INTO_`,
/// and one `_<k>` posterior column per class.
#[allow(clippy::too_many_arguments)]
fn write_out_dataset(
    _ast: &DiscrimAst,
    session: &mut Session,
    ds: &crate::dataset::SasDataset,
    model: &LdaModel,
    var_cols: &[Vec<Value>],
    class_col: &[Value],
    out_ref: &DatasetRef,
    n_read: usize,
) -> Result<()> {
    use crate::dataset::{SasDataset, VarMeta};
    use crate::value::VarType;
    use polars::prelude::*;

    let p = model.p;
    let g = model.n_groups;

    let mut from_vals: Vec<Option<String>> = Vec::with_capacity(n_read);
    let mut into_vals: Vec<Option<String>> = Vec::with_capacity(n_read);
    let mut post_cols: Vec<Vec<Option<f64>>> = vec![Vec::with_capacity(n_read); g];

    for i in 0..n_read {
        // Build x; if any var missing or class missing, row is not classified.
        let mut x = Vec::with_capacity(p);
        let mut ok = !class_col[i].is_missing();
        for vc in var_cols {
            match value_to_num(&vc[i]) {
                Some(v) if !v.is_nan() => x.push(v),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            let into = model.classify(&x);
            let post = model.posteriors(&x);
            from_vals.push(Some(value_label(&class_col[i])));
            into_vals.push(Some(model.class_labels[into].clone()));
            for k in 0..g {
                post_cols[k].push(Some(post[k]));
            }
        } else {
            from_vals.push(if class_col[i].is_missing() {
                None
            } else {
                Some(value_label(&class_col[i]))
            });
            into_vals.push(None);
            for k in 0..g {
                post_cols[k].push(None);
            }
        }
    }

    let mut out_df = ds.df.clone();
    out_df
        .with_column(Series::new("_FROM_".into(), from_vals))
        .and_then(|df| df.with_column(Series::new("_INTO_".into(), into_vals)))
        .map_err(|e| SasError::runtime(format!("DISCRIM OUT= build failed: {e}")))?;
    for k in 0..g {
        let col_name = format!("_{}", model.class_labels[k]);
        out_df
            .with_column(Series::new(col_name.into(), post_cols[k].clone()))
            .map_err(|e| SasError::runtime(format!("DISCRIM OUT= build failed: {e}")))?;
    }

    let mut vars = ds.vars.clone();
    vars.push(VarMeta {
        name: "_FROM_".into(),
        ty: VarType::Char,
        length: 32,
        format: None,
        label: None,
    });
    vars.push(VarMeta {
        name: "_INTO_".into(),
        ty: VarType::Char,
        length: 32,
        format: None,
        label: None,
    });
    for k in 0..g {
        vars.push(VarMeta {
            name: format!("_{}", model.class_labels[k]),
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

// ───────────────────────── Tests ─────────────────────────

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

    fn parse_discrim(src: &str) -> Result<DiscrimAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // proc
        ts.next(); // discrim
        parse(&mut ts)
    }

    fn make_oracle_session() -> (Session, DiscrimAst) {
        let session = make_session();
        let frame = df![
            "class" => ["A", "A", "A", "B", "B", "B"],
            "x" => [1.0_f64, 2.0, 3.0, 5.0, 6.0, 7.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df: frame,
            vars: vec![
                VarMeta {
                    name: "class".into(),
                    ty: VarType::Char,
                    length: 1,
                    format: None,
                    label: None,
                },
                VarMeta {
                    name: "x".into(),
                    ty: VarType::Num,
                    length: 8,
                    format: None,
                    label: None,
                },
            ],
        };
        session.libs.get("WORK").unwrap().write("LDA", &ds).unwrap();
        let ast = DiscrimAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "LDA".into(),
            }),
            out: None,
            outstat: None,
            method: None,
            pool: Pool::Yes,
            priors: Priors::Equal,
            noclassify: false,
            crossvalidate: false,
            short: false,
            class_var: Some("class".into()),
            var_vars: vec!["x".into()],
            id_var: None,
        };
        (session, ast)
    }

    fn fit_oracle() -> LdaModel {
        let classes = vec![Value::Char("A".into()), Value::Char("B".into())];
        let class_obs = vec![
            vec![vec![1.0], vec![2.0], vec![3.0]],
            vec![vec![5.0], vec![6.0], vec![7.0]],
        ];
        fit_lda(classes, &class_obs, &Priors::Equal, 1).unwrap()
    }

    // ── parse tests ──

    #[test]
    fn test_parse_basic() {
        let ast = parse_discrim("proc discrim; class g; var x y; run;").unwrap();
        assert_eq!(ast.class_var, Some("g".to_string()));
        assert_eq!(ast.var_vars, vec!["x", "y"]);
        assert_eq!(ast.priors, Priors::Equal);
        assert_eq!(ast.pool, Pool::Yes);
    }

    #[test]
    fn test_parse_options() {
        let ast = parse_discrim(
            "proc discrim data=a out=b method=normal pool=no noclassify short; class g; var x; id name; priors proportional; run;",
        )
        .unwrap();
        assert_eq!(ast.data.as_ref().unwrap().name, "a");
        assert_eq!(ast.out.as_ref().unwrap().name, "b");
        assert_eq!(ast.method.as_deref(), Some("NORMAL"));
        assert_eq!(ast.pool, Pool::No);
        assert_eq!(ast.priors, Priors::Proportional);
        assert!(ast.noclassify);
        assert!(ast.short);
        assert_eq!(ast.id_var, Some("name".to_string()));
    }

    // ── invariant tests ──

    #[test]
    fn test_pooled_cov_and_inverse() {
        let m = fit_oracle();
        // Σ_pooled = 1.0, inverse = 1.0
        assert!((m.pooled[0][0] - 1.0).abs() < 1e-12);
        assert!((m.pooled_inv[0][0] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn test_constants_bake_in_prior() {
        let m = fit_oracle();
        // Constant_A = -2.0 + ln(0.5) = -2.6931
        assert!((m.constants[0] - (-2.6931)).abs() < 1e-3, "got {}", m.constants[0]);
        // Constant_B = -18.0 + ln(0.5) = -18.6931
        assert!((m.constants[1] - (-18.6931)).abs() < 1e-3, "got {}", m.constants[1]);
        // coefficients
        assert!((m.coefs[0][0] - 2.0).abs() < 1e-12);
        assert!((m.coefs[1][0] - 6.0).abs() < 1e-12);
    }

    #[test]
    fn test_decision_boundary_at_4() {
        let m = fit_oracle();
        // Score_A(4) == Score_B(4) at the boundary.
        let sa = m.score(0, &[4.0]);
        let sb = m.score(1, &[4.0]);
        assert!((sa - sb).abs() < 1e-8, "sa={sa} sb={sb}");
    }

    #[test]
    fn test_group_distance() {
        let m = fit_oracle();
        // D²(A,B) = 16.0
        assert!((m.group_distance(0, 1) - 16.0).abs() < 1e-8);
        assert!((m.group_distance(1, 0) - 16.0).abs() < 1e-8);
        assert!(m.group_distance(0, 0).abs() < 1e-8);
    }

    #[test]
    fn test_posteriors_sum_to_one() {
        let m = fit_oracle();
        for x in [1.0, 2.0, 3.0, 5.0, 6.0, 7.0] {
            let post = m.posteriors(&[x]);
            let s: f64 = post.iter().sum();
            assert!((s - 1.0).abs() < 1e-8, "sum={s} for x={x}");
        }
    }

    #[test]
    fn test_classification_all_correct() {
        let m = fit_oracle();
        for x in [1.0, 2.0, 3.0] {
            assert_eq!(m.classify(&[x]), 0, "x={x} should be A");
        }
        for x in [5.0, 6.0, 7.0] {
            assert_eq!(m.classify(&[x]), 1, "x={x} should be B");
        }
    }

    #[test]
    fn test_posterior_x3() {
        let m = fit_oracle();
        let post = m.posteriors(&[3.0]);
        // P_A ≈ 0.9820
        assert!((post[0] - 0.9820).abs() < 1e-3, "P_A={}", post[0]);
        assert!((post[1] - 0.0180).abs() < 1e-3, "P_B={}", post[1]);
    }

    // ── execute / listing tests ──

    #[test]
    fn test_execute_oracle_listing() {
        let (mut session, ast) = make_oracle_session();
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("-2.6931"), "Constant_A missing: {listing}");
        assert!(listing.contains("-18.6931"), "Constant_B missing: {listing}");
        assert!(listing.contains("16.0000"), "D²(A,B) missing: {listing}");
        assert!(
            listing.contains("The DISCRIMINANT Procedure"),
            "title missing"
        );
    }

    #[test]
    fn test_out_dataset() {
        let (mut session, mut ast) = make_oracle_session();
        ast.out = Some(DatasetRef {
            libref: Some("WORK".into()),
            name: "RESULT".into(),
        });
        execute(&ast, &mut session).unwrap();
        let (out, _) = session.libs.get("WORK").unwrap().read("RESULT").unwrap();
        assert!(out.vars.iter().any(|v| v.name == "_FROM_"));
        assert!(out.vars.iter().any(|v| v.name == "_INTO_"));
        assert!(out.vars.iter().any(|v| v.name == "_A"));
        assert!(out.vars.iter().any(|v| v.name == "_B"));
        // All 6 rows classified correctly: _FROM_ == _INTO_.
        let from = out.df.column("_FROM_").unwrap().str().unwrap();
        let into = out.df.column("_INTO_").unwrap().str().unwrap();
        for i in 0..6 {
            assert_eq!(from.get(i), into.get(i), "row {i} misclassified");
        }
    }

    #[test]
    fn test_proportional_priors() {
        // Unequal group sizes change the prior term in the constant.
        let classes = vec![Value::Char("A".into()), Value::Char("B".into())];
        let class_obs = vec![
            vec![vec![1.0], vec![2.0], vec![3.0], vec![4.0]],
            vec![vec![6.0], vec![7.0]],
        ];
        let m = fit_lda(classes, &class_obs, &Priors::Proportional, 1).unwrap();
        // priors = 4/6, 2/6
        assert!((m.priors[0] - 4.0 / 6.0).abs() < 1e-12);
        assert!((m.priors[1] - 2.0 / 6.0).abs() < 1e-12);
    }
}
