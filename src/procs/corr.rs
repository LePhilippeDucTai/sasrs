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
//! ## M21.5 — extensions (Spearman / Kendall / OUT= / WEIGHT)
//! - `spearman` : corrélation de rang de Spearman = Pearson sur les **rangs**
//!   (rangs moyens pour les ex æquo) de chaque paire appariée-complète.
//!   `Prob > |r|` via la même approximation t (ddl = n−2). Bloc « Spearman
//!   Correlation Coefficients ».
//! - `kendall` : tau-b de Kendall, τ_b = (n_c − n_d)/√((n0−n1)(n0−n2)),
//!   p-value par approximation normale z = 3·τ·√(n(n−1))/√(2(2n+5)) (sans
//!   correction de ties en v1, documenté). Bloc « Kendall Tau b Coefficients ».
//! - `pearson` : sélectionne explicitement Pearson. Par défaut (aucune option
//!   de méthode), seul Pearson est produit, byte-identique à l'incrément v1.
//! - `weight var` : Pearson **pondéré** (moyennes/(co)variances pondérées par
//!   w ; obs exclue si w manquant ou ≤ 0 — voir `partition_weighted`). Ne
//!   s'applique PAS à Spearman/Kendall en v1 (documenté).
//! - `out=`/`outp=`/`outs=`/`outk=` : dataset TYPE=CORR. Variables `_TYPE_`
//!   (MEAN/STD/N/CORR), `_NAME_` (nom de variable des lignes CORR), puis une
//!   colonne par variable analysée. OUTP=/OUT= = Pearson, OUTS= = Spearman,
//!   OUTK= = Kendall. NOTE de création, types SAS, `last_dataset` mis à jour.
//!   Le bloc CORR du dataset est carré (analysis × analysis), indépendamment
//!   de WITH (qui ne modifie que la mise en page du listing).
//! - En-têtes de table : on s'appuie sur `ListingWriter::write_table` ; les
//!   cellules multi-lignes (r / prob / N) de la matrice sont rendues avec une
//!   ligne de tableau par composante (r, puis prob, puis N) pour rester dans
//!   le moule monospace existant.

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column, partition_numeric, partition_weighted, sample_std};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{format_best, Value, VarType};
use polars::prelude::{Column, DataFrame, NamedFrom, Series};

pub struct CorrAst {
    pub data: Option<DatasetRef>,
    pub nosimple: bool,
    pub noprob: bool,
    pub nocorr: bool,
    /// Request Pearson coefficients explicitly. When any of pearson/spearman/
    /// kendall is set, only the requested methods are produced; otherwise
    /// Pearson is the default.
    pub pearson: bool,
    /// Request Spearman rank correlation coefficients.
    pub spearman: bool,
    /// Request Kendall tau-b coefficients.
    pub kendall: bool,
    /// Explicit VAR list (empty = default to all numeric variables).
    pub var: Vec<String>,
    /// Optional WITH list (empty = none).
    pub with: Vec<String>,
    /// Optional WEIGHT variable (Pearson only). None = unweighted.
    pub weight: Option<String>,
    /// OUTP= / OUT= : Pearson output dataset (TYPE=CORR).
    pub outp: Option<DatasetRef>,
    /// OUTS= : Spearman output dataset (TYPE=CORR).
    pub outs: Option<DatasetRef>,
    /// OUTK= : Kendall output dataset (TYPE=CORR).
    pub outk: Option<DatasetRef>,
}

impl CorrAst {
    /// Whether the Pearson method should be computed/displayed. Pearson is the
    /// default when no method option was given.
    fn want_pearson(&self) -> bool {
        self.pearson || !(self.spearman || self.kendall)
    }
}

/// Parse `proc corr [data=a] [nosimple] [noprob] [nocorr];
/// [var ...;] [with ...;] ... run;`. Called AFTER "proc corr" was consumed.
/// Consumes through `run;` / `quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<CorrAst> {
    let mut data: Option<DatasetRef> = None;
    let mut nosimple = false;
    let mut noprob = false;
    let mut nocorr = false;
    let mut pearson = false;
    let mut spearman = false;
    let mut kendall = false;
    let mut outp: Option<DatasetRef> = None;
    let mut outs: Option<DatasetRef> = None;
    let mut outk: Option<DatasetRef> = None;

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
            common::expect_eq(ts, "DATA")?;
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
        } else if ts.peek().is_kw("pearson") {
            ts.next();
            pearson = true;
        } else if ts.peek().is_kw("spearman") {
            ts.next();
            spearman = true;
        } else if ts.peek().is_kw("kendall") {
            ts.next();
            kendall = true;
        } else if ts.peek().is_kw("out") || ts.peek().is_kw("outp") {
            common::expect_eq(ts, "OUT")?;
            outp = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("outs") {
            common::expect_eq(ts, "OUTS")?;
            outs = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("outk") {
            common::expect_eq(ts, "OUTK")?;
            outk = Some(ts.parse_dataset_ref()?);
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
    let mut weight: Option<String> = None;

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "var" => {
                ts.next();
                var = common::parse_var_list(ts)?;
                true
            }
            "with" => {
                ts.next();
                with = common::parse_var_list(ts)?;
                true
            }
            "weight" => {
                ts.next();
                let names = ts.parse_name_list()?;
                ts.expect_semi()?;
                // SAS allows a single weight variable.
                if names.len() != 1 {
                    return Err(SasError::runtime(
                        "The WEIGHT statement of PROC CORR accepts exactly one variable.",
                    ));
                }
                weight = Some(names.into_iter().next().unwrap());
                true
            }
            _ => false,
        })
    })?;

    Ok(CorrAst {
        data,
        nosimple,
        noprob,
        nocorr,
        pearson,
        spearman,
        kendall,
        var,
        with,
        weight,
        outp,
        outs,
        outk,
    })
}

// ───────────────────────── numeric core ─────────────────────────

/// Pearson r over pairwise-complete observations. Returns (r, n) where n is
/// the number of complete pairs. r is `None` when n < 2 or either variable
/// has zero variance (constant) over the pairwise-complete set.
fn pearson(xcol: &[Value], ycol: &[Value]) -> (Option<f64>, usize) {
    let (xs, ys) = paired_complete(xcol, ycol);
    let n = xs.len();
    if n < 2 {
        return (None, n);
    }
    (pearson_xy(&xs, &ys), n)
}

/// Collect the pairwise-complete numeric observations of two columns as
/// parallel `(xs, ys)` vectors (rows where either value is missing/NaN are
/// dropped). Shared by Pearson, Spearman and Kendall.
fn paired_complete(xcol: &[Value], ycol: &[Value]) -> (Vec<f64>, Vec<f64>) {
    let mut xs: Vec<f64> = Vec::new();
    let mut ys: Vec<f64> = Vec::new();
    let n_rows = xcol.len().min(ycol.len());
    for i in 0..n_rows {
        match (value_to_num(&xcol[i]), value_to_num(&ycol[i])) {
            (Some(x), Some(y)) if !x.is_nan() && !y.is_nan() => {
                xs.push(x);
                ys.push(y);
            }
            _ => {}
        }
    }
    (xs, ys)
}

/// Pearson r over two already paired-complete numeric vectors. Returns None
/// when n < 2 or either side is constant (zero variance).
fn pearson_xy(xs: &[f64], ys: &[f64]) -> Option<f64> {
    let n = xs.len();
    if n < 2 {
        return None;
    }
    let nf = n as f64;
    let mx = xs.iter().sum::<f64>() / nf;
    let my = ys.iter().sum::<f64>() / nf;
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for k in 0..n {
        let dx = xs[k] - mx;
        let dy = ys[k] - my;
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    if sxx <= 0.0 || syy <= 0.0 {
        return None;
    }
    Some((sxy / (sxx.sqrt() * syy.sqrt())).clamp(-1.0, 1.0))
}

/// Weighted Pearson r over pairwise-complete observations. An observation is
/// usable only when x, y AND w are non-missing and w > 0 (SAS WEIGHT rule).
/// Weighted moments: mean_w = Σw·x/Σw, cov_w = Σw(x−mx)(y−my)/Σw, etc.
/// Returns (r, n) where n counts the usable triples. r is None when n < 2 or
/// either weighted variance is zero.
fn pearson_weighted(xcol: &[Value], ycol: &[Value], wcol: &[Value]) -> (Option<f64>, usize) {
    let n_rows = xcol.len().min(ycol.len()).min(wcol.len());
    let mut xs: Vec<f64> = Vec::new();
    let mut ys: Vec<f64> = Vec::new();
    let mut ws: Vec<f64> = Vec::new();
    for i in 0..n_rows {
        match (
            value_to_num(&xcol[i]),
            value_to_num(&ycol[i]),
            value_to_num(&wcol[i]),
        ) {
            (Some(x), Some(y), Some(w))
                if !x.is_nan() && !y.is_nan() && !w.is_nan() && w > 0.0 =>
            {
                xs.push(x);
                ys.push(y);
                ws.push(w);
            }
            _ => {}
        }
    }
    let n = xs.len();
    if n < 2 {
        return (None, n);
    }
    let sw: f64 = ws.iter().sum();
    if sw <= 0.0 {
        return (None, n);
    }
    let mx: f64 = xs.iter().zip(&ws).map(|(x, w)| w * x).sum::<f64>() / sw;
    let my: f64 = ys.iter().zip(&ws).map(|(y, w)| w * y).sum::<f64>() / sw;
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for k in 0..n {
        let dx = xs[k] - mx;
        let dy = ys[k] - my;
        sxy += ws[k] * dx * dy;
        sxx += ws[k] * dx * dx;
        syy += ws[k] * dy * dy;
    }
    if sxx <= 0.0 || syy <= 0.0 {
        return (None, n);
    }
    let r = (sxy / (sxx.sqrt() * syy.sqrt())).clamp(-1.0, 1.0);
    (Some(r), n)
}

/// Mean (midrank) ranks of a slice: ties receive the average of the ranks they
/// would otherwise occupy (1-based). Used by Spearman.
fn mean_ranks(xs: &[f64]) -> Vec<f64> {
    let n = xs.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| xs[a].partial_cmp(&xs[b]).unwrap_or(std::cmp::Ordering::Equal));
    let mut ranks = vec![0.0_f64; n];
    let mut i = 0;
    while i < n {
        let mut j = i + 1;
        while j < n && xs[idx[j]] == xs[idx[i]] {
            j += 1;
        }
        // Ranks i+1 .. j (1-based) averaged.
        let avg = ((i + 1 + j) as f64) / 2.0; // sum (i+1..=j) / count
        for &k in &idx[i..j] {
            ranks[k] = avg;
        }
        i = j;
    }
    ranks
}

/// Spearman rank correlation over pairwise-complete observations: Pearson on
/// the midranks of x and y. Returns (r, n). None when n < 2 or a rank vector
/// is constant (all values tied).
fn spearman(xcol: &[Value], ycol: &[Value]) -> (Option<f64>, usize) {
    let (xs, ys) = paired_complete(xcol, ycol);
    let n = xs.len();
    if n < 2 {
        return (None, n);
    }
    let rx = mean_ranks(&xs);
    let ry = mean_ranks(&ys);
    (pearson_xy(&rx, &ry), n)
}

/// Kendall tau-b over pairwise-complete observations.
/// τ_b = (n_c − n_d)/√((n0 − n1)(n0 − n2)), with n0 = n(n−1)/2,
/// n1 = Σ t_i(t_i−1)/2 (ties in x), n2 = Σ u_j(u_j−1)/2 (ties in y).
/// Returns (tau, n). None when n < 2 or a denominator factor is zero (i.e. all
/// x tied or all y tied).
fn kendall_tau_b(xcol: &[Value], ycol: &[Value]) -> (Option<f64>, usize) {
    let (xs, ys) = paired_complete(xcol, ycol);
    let n = xs.len();
    if n < 2 {
        return (None, n);
    }
    let mut concordant: i64 = 0;
    let mut discordant: i64 = 0;
    for i in 0..n {
        for j in (i + 1)..n {
            let dx = xs[i] - xs[j];
            let dy = ys[i] - ys[j];
            let s = dx.signum() * dy.signum();
            if dx != 0.0 && dy != 0.0 {
                if s > 0.0 {
                    concordant += 1;
                } else {
                    discordant += 1;
                }
            }
        }
    }
    let n0 = (n as f64) * (n as f64 - 1.0) / 2.0;
    let n1 = tie_term(&xs);
    let n2 = tie_term(&ys);
    let denom = (n0 - n1) * (n0 - n2);
    if denom <= 0.0 {
        return (None, n);
    }
    let tau = (concordant - discordant) as f64 / denom.sqrt();
    (Some(tau.clamp(-1.0, 1.0)), n)
}

/// Σ t(t−1)/2 over groups of equal values (tie correction term for Kendall).
fn tie_term(xs: &[f64]) -> f64 {
    let mut v: Vec<f64> = xs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut total = 0.0;
    let mut i = 0;
    let n = v.len();
    while i < n {
        let mut j = i + 1;
        while j < n && v[j] == v[i] {
            j += 1;
        }
        let t = (j - i) as f64;
        total += t * (t - 1.0) / 2.0;
        i = j;
    }
    total
}

/// Two-sided p-value for Kendall tau-b via the normal approximation
/// z = 3·τ·√(n(n−1)) / √(2(2n+5)); p = 2·(1 − Φ(|z|)). v1 ignores the
/// ties correction in the variance (documented). None when n < 2.
fn kendall_pvalue(tau: f64, n: usize) -> Option<f64> {
    if n < 2 {
        return None;
    }
    let nf = n as f64;
    let z = 3.0 * tau * (nf * (nf - 1.0)).sqrt() / (2.0 * (2.0 * nf + 5.0)).sqrt();
    Some(2.0 * normal_sf(z.abs()))
}

/// Upper-tail standard-normal survival function 1 − Φ(z) for z >= 0, via the
/// complementary error function relation Φ(z) = ½ erfc(−z/√2). Accuracy ~1e-7,
/// ample for a documented normal approximation.
fn normal_sf(z: f64) -> f64 {
    0.5 * erfc(z / std::f64::consts::SQRT_2)
}

/// erfc(x) — Numerical Recipes rational (Chebyshev) approximation, |error| < 1.2e-7.
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let ans = t
        * (-z * z - 1.26551223
            + t * (1.00002368
                + t * (0.37409196
                    + t * (0.09678418
                        + t * (-0.18628806
                            + t * (0.27886807
                                + t * (-1.13520398
                                    + t * (1.48851587
                                        + t * (-0.82215223 + t * 0.17087277)))))))))
        .exp();
    if x >= 0.0 {
        ans
    } else {
        2.0 - ans
    }
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
    let in_ref = common::resolve_last_dataset(&ast.data, session)?;
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

    // Resolve the WEIGHT variable (single numeric column). Applies to Pearson.
    let weight_col: Option<usize> = match &ast.weight {
        Some(nm) => Some(resolve_names(std::slice::from_ref(nm))?[0]),
        None => None,
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

    // Decode each needed column exactly once (analysis vars + weight).
    let mut decoded: std::collections::HashMap<usize, Vec<Value>> =
        std::collections::HashMap::new();
    for &c in &analysis_cols {
        decoded.insert(c, decode_column(&ds, c)?);
    }
    if let Some(wc) = weight_col {
        decoded
            .entry(wc)
            .or_insert_with(|| decode_column(&ds, wc).unwrap_or_default());
    }
    let weight_vals: Option<&[Value]> = weight_col.map(|wc| decoded[&wc].as_slice());

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

    // Which methods are requested (Pearson default when none specified).
    let methods: Vec<Method> = {
        let mut m = Vec::new();
        if ast.want_pearson() {
            m.push(Method::Pearson);
        }
        if ast.spearman {
            m.push(Method::Spearman);
        }
        if ast.kendall {
            m.push(Method::Kendall);
        }
        m
    };

    // --- Correlation Coefficients (one block per requested method) ---
    if !ast.nocorr {
        for &method in &methods {
            let cells = compute_matrix(method, &row_cols, &col_cols, &decoded, weight_vals);
            emit_correlations(session, &ds, &row_cols, &col_cols, method, &cells, ast.noprob);
        }
    }

    // --- OUT= / OUTP= / OUTS= / OUTK= : TYPE=CORR datasets ---
    // The CORR block of the output dataset is square (analysis × analysis),
    // independent of WITH.
    let out_targets: [(Method, &Option<DatasetRef>); 3] = [
        (Method::Pearson, &ast.outp),
        (Method::Spearman, &ast.outs),
        (Method::Kendall, &ast.outk),
    ];
    for (method, target) in out_targets {
        if let Some(target) = target {
            let out_ds = build_out_dataset(
                method,
                &ds,
                &analysis_cols,
                &decoded,
                weight_vals,
                n_obs,
            )?;
            write_out_dataset(session, target, out_ds)?;
        }
    }

    Ok(())
}

/// Correlation method requested by PROC CORR.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    Pearson,
    Spearman,
    Kendall,
}

impl Method {
    /// Heading line for the listing block.
    fn heading(self) -> &'static str {
        match self {
            Method::Pearson => "Pearson Correlation Coefficients",
            Method::Spearman => "Spearman Correlation Coefficients",
            Method::Kendall => "Kendall Tau b Coefficients",
        }
    }

}

/// `_TYPE_` value written in the TYPE=CORR output dataset's CORR rows. SAS uses
/// the literal "CORR" for Pearson, Spearman and Kendall datasets alike.
const CORR_TYPE: &str = "CORR";

/// One computed (r, p, n) cell.
#[derive(Clone, Copy)]
struct Cell {
    r: Option<f64>,
    p: Option<f64>,
    n: usize,
}

/// Compute r/p/n for every (row_col, col_col) pair under `method`. WEIGHT (if
/// any) is applied to Pearson only.
fn compute_matrix(
    method: Method,
    row_cols: &[usize],
    col_cols: &[usize],
    decoded: &std::collections::HashMap<usize, Vec<Value>>,
    weight: Option<&[Value]>,
) -> Vec<Vec<Cell>> {
    let mut out = vec![vec![Cell { r: None, p: None, n: 0 }; col_cols.len()]; row_cols.len()];
    for (i, &rc) in row_cols.iter().enumerate() {
        for (j, &cc) in col_cols.iter().enumerate() {
            out[i][j] = compute_cell(method, &decoded[&rc], &decoded[&cc], rc == cc, weight);
        }
    }
    out
}

/// Compute a single cell. `same_var` marks the diagonal where r is exactly 1
/// and the p-value is left blank (SAS convention).
fn compute_cell(
    method: Method,
    xcol: &[Value],
    ycol: &[Value],
    same_var: bool,
    weight: Option<&[Value]>,
) -> Cell {
    if same_var {
        // N = non-missing count of the variable (weighted: usable count).
        let n = match (method, weight) {
            (Method::Pearson, Some(w)) => {
                let (pairs, _) = partition_weighted(xcol, w, &(0..xcol.len()).collect::<Vec<_>>());
                pairs.len()
            }
            _ => {
                let (xs, _) = partition_numeric(xcol, &(0..xcol.len()).collect::<Vec<_>>());
                xs.len()
            }
        };
        return Cell { r: Some(1.0), p: None, n };
    }
    match method {
        Method::Pearson => {
            let (r, n) = match weight {
                Some(w) => pearson_weighted(xcol, ycol, w),
                None => pearson(xcol, ycol),
            };
            let p = r.and_then(|rv| pearson_pvalue(rv, n));
            Cell { r, p, n }
        }
        Method::Spearman => {
            let (r, n) = spearman(xcol, ycol);
            let p = r.and_then(|rv| pearson_pvalue(rv, n));
            Cell { r, p, n }
        }
        Method::Kendall => {
            let (r, n) = kendall_tau_b(xcol, ycol);
            let p = r.and_then(|rv| kendall_pvalue(rv, n));
            Cell { r, p, n }
        }
    }
}

/// Write a centered line within LINESIZE.
fn centered(session: &mut Session, text: &str) {
    let ls = session.listing.ls();
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
    method: Method,
    cells: &[Vec<Cell>],
    noprob: bool,
) {
    centered(session, method.heading());
    if !noprob {
        // Symbol matches the coefficient: r for Pearson/Spearman, tau for
        // Kendall.
        let prob_line = match method {
            Method::Kendall => "Prob > |tau| under H0: Tau=0",
            _ => "Prob > |r| under H0: Rho=0",
        };
        centered(session, prob_line);
    }
    session.listing.blank();

    let nr = row_cols.len();
    let nc = col_cols.len();
    let rmat: Vec<Vec<Option<f64>>> =
        cells.iter().map(|row| row.iter().map(|c| c.r).collect()).collect();
    let pmat: Vec<Vec<Option<f64>>> =
        cells.iter().map(|row| row.iter().map(|c| c.p).collect()).collect();
    let nmat: Vec<Vec<usize>> =
        cells.iter().map(|row| row.iter().map(|c| c.n).collect()).collect();

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

// ───────────────────────── OUT= (TYPE=CORR) ─────────────────────────

/// Build a TYPE=CORR output dataset for `method` over the square analysis ×
/// analysis correlation matrix. Layout (SAS):
///   _TYPE_  _NAME_   <var1> <var2> ...
///   MEAN             m1     m2     ...
///   STD              s1     s2     ...
///   N                n1     n2     ...
///   CORR    var1     r11    r12    ...
///   CORR    var2     r21    r22    ...
/// MEAN/STD/N rows carry an empty `_NAME_`. The CORR block uses the same
/// pairwise-complete r computed for the listing. WEIGHT applies to Pearson.
fn build_out_dataset(
    method: Method,
    ds: &SasDataset,
    analysis_cols: &[usize],
    decoded: &std::collections::HashMap<usize, Vec<Value>>,
    weight: Option<&[Value]>,
    n_obs: usize,
) -> Result<SasDataset> {
    let k = analysis_cols.len();
    let all_rows: Vec<usize> = (0..n_obs).collect();

    // Per-variable simple stats (unweighted MEAN/STD/N, matching SAS TYPE=CORR
    // simple-statistics rows; WEIGHT does not alter these rows in v1).
    let mut means = Vec::with_capacity(k);
    let mut stds = Vec::with_capacity(k);
    let mut ns = Vec::with_capacity(k);
    for &c in analysis_cols {
        let (xs, _) = partition_numeric(&decoded[&c], &all_rows);
        let n = xs.len();
        means.push(if n > 0 {
            Some(xs.iter().sum::<f64>() / n as f64)
        } else {
            None
        });
        stds.push(sample_std(&xs));
        ns.push(n as f64);
    }

    // CORR block: square matrix over analysis_cols.
    let cells = compute_matrix(method, analysis_cols, analysis_cols, decoded, weight);

    // Assemble row-major then transpose into columns.
    // Row order: MEAN, STD, N, then one CORR row per analysis variable.
    let n_rows = 3 + k;
    let mut type_col: Vec<Option<String>> = Vec::with_capacity(n_rows);
    let mut name_col: Vec<Option<String>> = Vec::with_capacity(n_rows);
    // One value column per analysis variable.
    let mut value_cols: Vec<Vec<Option<f64>>> = vec![Vec::with_capacity(n_rows); k];

    type_col.push(Some("MEAN".into()));
    name_col.push(None);
    for j in 0..k {
        value_cols[j].push(means[j]);
    }
    type_col.push(Some("STD".into()));
    name_col.push(None);
    for j in 0..k {
        value_cols[j].push(stds[j]);
    }
    type_col.push(Some("N".into()));
    name_col.push(None);
    for j in 0..k {
        value_cols[j].push(Some(ns[j]));
    }
    for i in 0..k {
        type_col.push(Some(CORR_TYPE.into()));
        name_col.push(Some(ds.vars[analysis_cols[i]].name.clone()));
        for j in 0..k {
            value_cols[j].push(cells[i][j].r);
        }
    }

    // Build columns: _TYPE_ (char), _NAME_ (char), then one numeric column per
    // analysis variable (original variable name preserved).
    let mut columns: Vec<Column> = Vec::with_capacity(k + 2);
    let mut vars: Vec<VarMeta> = Vec::with_capacity(k + 2);

    columns.push(Series::new("_TYPE_".into(), type_col).into());
    vars.push(char_var_meta("_TYPE_", 8));
    columns.push(Series::new("_NAME_".into(), name_col).into());
    vars.push(char_var_meta("_NAME_", 32));

    for (j, &c) in analysis_cols.iter().enumerate() {
        let name = ds.vars[c].name.clone();
        columns.push(Series::new(name.as_str().into(), std::mem::take(&mut value_cols[j])).into());
        vars.push(num_var_meta(&name));
    }

    let df = DataFrame::new(columns)?;
    Ok(SasDataset { df, vars })
}

/// Persist a built TYPE=CORR dataset to `target`, update `_LAST_`, and emit the
/// SAS creation NOTE.
fn write_out_dataset(
    session: &mut Session,
    target: &DatasetRef,
    out_ds: SasDataset,
) -> Result<()> {
    let out_libref = target.libref_or_work();
    let out_table = target.name.to_uppercase();
    let display = format!("{out_libref}.{out_table}");
    let n_rows = out_ds.n_obs();
    let n_vars = out_ds.vars.len();

    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(display.clone());
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_rows, n_vars
    ));
    Ok(())
}

fn num_var_meta(name: &str) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}

fn char_var_meta(name: &str, length: usize) -> VarMeta {
    VarMeta {
        name: name.to_string(),
        ty: VarType::Char,
        length,
        format: None,
        label: None,
    }
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
    fn parse_out_options() {
        let ast = parse_corr(
            "proc corr data=a out=p outs=s outk=k spearman kendall pearson; run;",
        )
        .unwrap();
        assert_eq!(ast.outp.as_ref().unwrap().name, "p");
        assert_eq!(ast.outs.as_ref().unwrap().name, "s");
        assert_eq!(ast.outk.as_ref().unwrap().name, "k");
        assert!(ast.pearson && ast.spearman && ast.kendall);
    }

    #[test]
    fn parse_outp_and_weight() {
        let ast = parse_corr("proc corr data=a outp=b; var x y; weight wt; run;").unwrap();
        assert_eq!(ast.outp.as_ref().unwrap().name, "b");
        assert_eq!(ast.weight.as_deref(), Some("wt"));
        // Default method selection: pearson wanted, spearman/kendall not.
        assert!(ast.want_pearson() && !ast.spearman && !ast.kendall);
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
            pearson: false,
            spearman: false,
            kendall: false,
            weight: None,
            outp: None,
            outs: None,
            outk: None,
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
            pearson: false,
            spearman: false,
            kendall: false,
            weight: None,
            outp: None,
            outs: None,
            outk: None,
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
            pearson: false,
            spearman: false,
            kendall: false,
            weight: None,
            outp: None,
            outs: None,
            outk: None,
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
            pearson: false,
            spearman: false,
            kendall: false,
            weight: None,
            outp: None,
            outs: None,
            outk: None,
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
            pearson: false,
            spearman: false,
            kendall: false,
            weight: None,
            outp: None,
            outs: None,
            outk: None,
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
            pearson: false,
            spearman: false,
            kendall: false,
            weight: None,
            outp: None,
            outs: None,
            outk: None,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // Only x and y (numeric) are analyzed; char g excluded.
        assert!(listing.contains("2 Variables:"), "{listing}");
        assert!(listing.contains("x y"), "{listing}");
    }

    // ───────────── M21.5: Spearman / Kendall / WEIGHT / OUT= ─────────────

    fn base_ast(table: &str) -> CorrAst {
        CorrAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: table.into() }),
            nosimple: true,
            noprob: false,
            nocorr: false,
            pearson: false,
            spearman: false,
            kendall: false,
            var: vec![],
            with: vec![],
            weight: None,
            outp: None,
            outs: None,
            outk: None,
        }
    }

    fn vnum(vals: &[f64]) -> Vec<Value> {
        vals.iter().map(|v| Value::Num(*v)).collect()
    }

    // --- numeric core: Spearman ---

    #[test]
    fn spearman_perfect_monotone() {
        // Monotone but non-linear: ranks are perfectly correlated → r_s = 1.
        let x = vnum(&[1.0, 2.0, 3.0, 4.0]);
        let y = vnum(&[1.0, 4.0, 9.0, 16.0]);
        let (r, n) = spearman(&x, &y);
        assert_eq!(n, 4);
        assert!((r.unwrap() - 1.0).abs() < 1e-12, "r={r:?}");
    }

    #[test]
    fn spearman_hand_example() {
        // [(1,1),(2,3),(3,2),(4,4)]: ranks x=[1,2,3,4], y=[1,3,2,4],
        // Pearson on ranks → r_s = 0.8.
        let x = vnum(&[1.0, 2.0, 3.0, 4.0]);
        let y = vnum(&[1.0, 3.0, 2.0, 4.0]);
        let (r, n) = spearman(&x, &y);
        assert_eq!(n, 4);
        assert!((r.unwrap() - 0.8).abs() < 1e-12, "r={r:?}");
    }

    #[test]
    fn spearman_with_ties_uses_midranks() {
        // x=[1,1,2,3] → ranks [1.5,1.5,3,4]; y=[10,20,20,30] → [1,2.5,2.5,4].
        let x = vnum(&[1.0, 1.0, 2.0, 3.0]);
        let y = vnum(&[10.0, 20.0, 20.0, 30.0]);
        let rx = mean_ranks(&[1.0, 1.0, 2.0, 3.0]);
        assert_eq!(rx, vec![1.5, 1.5, 3.0, 4.0]);
        let (r, n) = spearman(&x, &y);
        assert_eq!(n, 4);
        // Pearson on [1.5,1.5,3,4] & [1,2.5,2.5,4]: hand → 0.9...
        assert!(r.unwrap() > 0.8 && r.unwrap() <= 1.0, "r={r:?}");
    }

    #[test]
    fn spearman_constant_is_missing() {
        let x = vnum(&[5.0, 5.0, 5.0]);
        let y = vnum(&[1.0, 2.0, 3.0]);
        let (r, _) = spearman(&x, &y);
        assert!(r.is_none());
    }

    // --- numeric core: Kendall ---

    #[test]
    fn kendall_hand_example() {
        // [(1,1),(2,3),(3,2),(4,4)]: C=5, D=1, no ties → tau_b = (5-1)/6.
        let x = vnum(&[1.0, 2.0, 3.0, 4.0]);
        let y = vnum(&[1.0, 3.0, 2.0, 4.0]);
        let (t, n) = kendall_tau_b(&x, &y);
        assert_eq!(n, 4);
        assert!((t.unwrap() - (4.0 / 6.0)).abs() < 1e-12, "t={t:?}");
    }

    #[test]
    fn kendall_perfect_and_anti() {
        let x = vnum(&[1.0, 2.0, 3.0, 4.0]);
        let yup = vnum(&[10.0, 20.0, 30.0, 40.0]);
        let ydn = vnum(&[40.0, 30.0, 20.0, 10.0]);
        assert!((kendall_tau_b(&x, &yup).0.unwrap() - 1.0).abs() < 1e-12);
        assert!((kendall_tau_b(&x, &ydn).0.unwrap() + 1.0).abs() < 1e-12);
    }

    #[test]
    fn kendall_tie_b_correction() {
        // x has a tie: x=[1,1,2,3], y=[1,2,3,4]. n0=6, n1=1 (one x-tie pair),
        // n2=0. Pairs excluding tie: (concordant). C=5, D=0.
        // tau_b = (5-0)/sqrt((6-1)(6-0)) = 5/sqrt(30) ≈ 0.9128709.
        let x = vnum(&[1.0, 1.0, 2.0, 3.0]);
        let y = vnum(&[1.0, 2.0, 3.0, 4.0]);
        let (t, n) = kendall_tau_b(&x, &y);
        assert_eq!(n, 4);
        assert!((t.unwrap() - 5.0 / 30f64.sqrt()).abs() < 1e-9, "t={t:?}");
    }

    #[test]
    fn kendall_all_tied_is_missing() {
        let x = vnum(&[2.0, 2.0, 2.0]);
        let y = vnum(&[1.0, 2.0, 3.0]);
        assert!(kendall_tau_b(&x, &y).0.is_none());
    }

    // --- weighted Pearson ---

    #[test]
    fn weighted_equals_unweighted_when_w1() {
        let x = vnum(&[1.0, 2.0, 3.0, 5.0]);
        let y = vnum(&[2.0, 1.0, 4.0, 3.0]);
        let w = vnum(&[1.0, 1.0, 1.0, 1.0]);
        let (ru, _) = pearson(&x, &y);
        let (rw, nw) = pearson_weighted(&x, &y, &w);
        assert_eq!(nw, 4);
        assert!((ru.unwrap() - rw.unwrap()).abs() < 1e-12, "{:?} {:?}", ru, rw);
    }

    #[test]
    fn weighted_excludes_nonpositive_and_missing() {
        // Row with w=0 and row with missing w are dropped → n=2.
        let x = vnum(&[1.0, 2.0, 3.0, 4.0]);
        let y = vnum(&[2.0, 4.0, 6.0, 8.0]);
        let w = vec![Value::Num(2.0), Value::Num(0.0), Value::missing(), Value::Num(3.0)];
        let (r, n) = pearson_weighted(&x, &y, &w);
        assert_eq!(n, 2);
        // Remaining pairs perfectly correlated → r = 1.
        assert!((r.unwrap() - 1.0).abs() < 1e-12, "r={r:?}");
    }

    #[test]
    fn weighted_changes_result() {
        // Up-weighting the well-aligned third point pulls the weighted r toward
        // 1, above the unweighted value. Hand-computed (weighted means
        // mx=2.75, my=7.75): sxy=19.25, sxx=4.25, syy=94.25 → r≈0.96183,
        // vs unweighted r=8/√76≈0.91766.
        let x = vnum(&[1.0, 2.0, 3.0]);
        let y = vnum(&[1.0, 2.0, 9.0]);
        let w = vnum(&[1.0, 1.0, 10.0]);
        let (ru, _) = pearson(&x, &y);
        let (rw, _) = pearson_weighted(&x, &y, &w);
        assert!((ru.unwrap() - 0.917663).abs() < 1e-5, "ru={ru:?}");
        assert!((rw.unwrap() - 0.961826).abs() < 1e-5, "rw={rw:?}");
        // Weighting materially changes the result (here, raises it).
        assert!(rw.unwrap() > ru.unwrap(), "ru={ru:?} rw={rw:?}");
    }

    // --- listing blocks ---

    #[test]
    fn execute_spearman_block() {
        let mut session = make_session();
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0, 4.0],
            "y" => [1.0_f64, 3.0, 2.0, 4.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("y")] };
        write_dataset(&mut session, "T", ds);

        let mut ast = base_ast("T");
        ast.spearman = true;
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("Spearman Correlation Coefficients"), "{listing}");
        // No Pearson block when only spearman requested.
        assert!(!listing.contains("Pearson Correlation Coefficients"), "{listing}");
        // r_s off-diagonal = 0.80000.
        assert!(listing.contains("0.80000"), "{listing}");
    }

    #[test]
    fn execute_kendall_block() {
        let mut session = make_session();
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0, 4.0],
            "y" => [1.0_f64, 3.0, 2.0, 4.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("y")] };
        write_dataset(&mut session, "T", ds);

        let mut ast = base_ast("T");
        ast.kendall = true;
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("Kendall Tau b Coefficients"), "{listing}");
        assert!(listing.contains("Prob > |tau|"), "{listing}");
        // tau_b = 0.66667.
        assert!(listing.contains("0.66667"), "{listing}");
    }

    #[test]
    fn execute_all_three_methods() {
        let mut session = make_session();
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0, 4.0],
            "y" => [2.0_f64, 1.0, 4.0, 3.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("y")] };
        write_dataset(&mut session, "T", ds);

        let mut ast = base_ast("T");
        ast.pearson = true;
        ast.spearman = true;
        ast.kendall = true;
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("Pearson Correlation Coefficients"), "{listing}");
        assert!(listing.contains("Spearman Correlation Coefficients"), "{listing}");
        assert!(listing.contains("Kendall Tau b Coefficients"), "{listing}");
    }

    // --- OUT= datasets ---

    #[test]
    fn execute_outp_dataset_structure() {
        let mut session = make_session();
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0, 4.0],
            "y" => [2.0_f64, 4.0, 6.0, 8.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("y")] };
        write_dataset(&mut session, "T", ds);

        let mut ast = base_ast("T");
        ast.outp = Some(DatasetRef { libref: Some("WORK".into()), name: "C".into() });
        execute(&ast, &mut session).unwrap();

        // Read back the produced TYPE=CORR dataset.
        let (out, _) = session.libs.get("WORK").unwrap().read("C").unwrap();
        // Columns: _TYPE_, _NAME_, x, y.
        let names: Vec<String> = out.vars.iter().map(|v| v.name.clone()).collect();
        assert_eq!(names, vec!["_TYPE_", "_NAME_", "x", "y"]);
        // 3 stats rows + 2 corr rows = 5.
        assert_eq!(out.n_obs(), 5);

        let type_col = decode_column(&out, 0).unwrap();
        let types: Vec<String> = type_col
            .iter()
            .map(|v| match v {
                Value::Char(s) => s.clone(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(types, vec!["MEAN", "STD", "N", "CORR", "CORR"]);

        // _NAME_ on CORR rows = x then y; empty on stats rows.
        let name_col = decode_column(&out, 1).unwrap();
        match &name_col[3] {
            Value::Char(s) => assert_eq!(s, "x"),
            other => panic!("expected x, got {other:?}"),
        }

        // CORR row for x: r(x,x)=1, r(x,y)=1 (perfect). Column "x" idx 2.
        let xcorr = decode_column(&out, 2).unwrap();
        assert!((value_to_num(&xcorr[3]).unwrap() - 1.0).abs() < 1e-12);
        // N row value = 4.
        assert!((value_to_num(&xcorr[2]).unwrap() - 4.0).abs() < 1e-12);
        // MEAN of x = 2.5.
        assert!((value_to_num(&xcorr[0]).unwrap() - 2.5).abs() < 1e-12);

        // _LAST_ updated.
        assert_eq!(session.last_dataset.as_deref(), Some("WORK.C"));
    }

    #[test]
    fn execute_outs_outk_methods() {
        let mut session = make_session();
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0, 4.0],
            "y" => [1.0_f64, 3.0, 2.0, 4.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("y")] };
        write_dataset(&mut session, "T", ds);

        let mut ast = base_ast("T");
        ast.spearman = true;
        ast.kendall = true;
        ast.outs = Some(DatasetRef { libref: Some("WORK".into()), name: "S".into() });
        ast.outk = Some(DatasetRef { libref: Some("WORK".into()), name: "K".into() });
        execute(&ast, &mut session).unwrap();

        // Spearman OUTS: corr(x,y) row for x = 0.8.
        let (s, _) = session.libs.get("WORK").unwrap().read("S").unwrap();
        let sx = decode_column(&s, 2).unwrap(); // column x
        // row 4 (index 4) is CORR y; row 3 is CORR x → off-diag at col y.
        let sy = decode_column(&s, 3).unwrap(); // column y, CORR x row
        assert!((value_to_num(&sy[3]).unwrap() - 0.8).abs() < 1e-9, "{:?}", sy[3]);
        assert!((value_to_num(&sx[3]).unwrap() - 1.0).abs() < 1e-9);

        // Kendall OUTK: corr(x,y) = 0.6667.
        let (kd, _) = session.libs.get("WORK").unwrap().read("K").unwrap();
        let ky = decode_column(&kd, 3).unwrap();
        assert!((value_to_num(&ky[3]).unwrap() - 4.0 / 6.0).abs() < 1e-9, "{:?}", ky[3]);
    }

    #[test]
    fn execute_weighted_listing_runs() {
        let mut session = make_session();
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0, 4.0],
            "y" => [2.0_f64, 4.0, 6.0, 8.0],
            "wt" => [1.0_f64, 1.0, 1.0, 1.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x"), num_meta("y"), num_meta("wt")],
        };
        write_dataset(&mut session, "T", ds);

        let mut ast = base_ast("T");
        ast.var = vec!["x".into(), "y".into()];
        ast.weight = Some("wt".into());
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // With w=1 the weighted r equals the unweighted perfect correlation.
        assert!(listing.contains("1.00000"), "{listing}");
        assert!(listing.contains("Pearson Correlation Coefficients"), "{listing}");
    }
}
