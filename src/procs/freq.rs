//! PROC FREQ (jalon M5).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc freq data=a ; tables v1 v2 v1*v2 [/ missing nopercent norow
//! nocol nofreq out=b] ; run ;`
//!
//! - Table 1 voie : valeurs triées (ordre sas_cmp), colonnes Frequency,
//!   Percent, Cumulative Frequency, Cumulative Percent. Par défaut les
//!   missings sont EXCLUS du tableau et comptés sous une ligne
//!   "Frequency Missing = N" ; option MISSING les réintègre (et ils
//!   entrent alors dans les pourcentages).
//! - Table 2 voies (v1*v2) : crosstab avec Frequency / Percent / Row Pct
//!   / Col Pct par cellule + marges. Implémenter par group_by Polars sur
//!   les paires puis mise en forme manuelle (le rendu SAS en blocs de 4
//!   lignes par cellule).
//! - out= (1 voie) : colonnes <var>, COUNT, PERCENT.
//!
//! ## Statistiques avancées (M21.2)
//! Options après `/` sur le statement TABLES :
//! - **CHISQ** : en deux voies → Pearson + Likelihood Ratio (M10) ; en UNE
//!   voie → test d'ajustement à l'équiprobabilité ("Chi-Square Test for Equal
//!   Proportions", DF = k-1).
//! - **FISHER** / **EXACT** : test exact de Fisher pour les tables 2×2
//!   (probabilités hypergéométriques exactes : F, left/right one-sided, P,
//!   two-sided). r×c (> 2×2) → note de non-support (différé, sans panic).
//! - **MEASURES** / **RELRISK** : odds ratio + risques relatifs cohorte
//!   (Col1/Col2) avec IC 95 % Wald sur l'échelle log, pour les tables 2×2.
//!   Cellules nulles → estimation manquante ("."), jamais de division par 0.
//! - **AGREE** : kappa simple de Cohen pour une table CARRÉE (Po, Pe, ASE,
//!   IC 95 %). Table non carrée → note propre.
//! - **TREND** : test de tendance de Cochran-Armitage pour une table 2×c ou
//!   r×2 (scores 1..k, statistique Z, p uni/bilatérale via `probnorm`).
//! Les blocs ne s'impriment que si leur option est demandée : la sortie par
//! défaut (et le CHISQ deux voies) restent byte-identiques.
//!
//! ## Choix de rendu (documenté pour l'orchestrateur)
//! - Titre centré "The FREQ Procedure" via `page_header()` puis une ligne
//!   centrée.
//! - Une voie : table à 5 colonnes (`<Var>`, Frequency, Percent,
//!   Cumulative Frequency, Cumulative Percent). Sans MISSING et avec des
//!   missings présents, une ligne "Frequency Missing = N" suit la table.
//! - Crosstab v1*v2 : une table dont la colonne de tête liste les valeurs
//!   de `v1` (plus une ligne "Total"), et qui porte une colonne par valeur
//!   de `v2` (plus "Total"). Chaque cellule (croisement) est rendue sur
//!   QUATRE lignes empilées dans la même colonne : Frequency, Percent
//!   (du total général), Row Pct (du total de la ligne), Col Pct (du total
//!   de la colonne). Les cellules de marge "Total" ne portent que
//!   Frequency et Percent (les deux dernières lignes restent vides). On
//!   construit ces lignes empilées à la main puis on les passe à
//!   `write_table` (une "ligne logique" = 4 lignes physiques).

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, chisq_sf, decode_column, ln_choose, probnorm};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{format_best, Value, VarType};
use polars::prelude::*;
use std::cmp::Ordering;

pub struct FreqAst {
    pub data: Option<DatasetRef>,
    pub tables: Vec<TableRequest>,
    /// WEIGHT statement variable (cell frequencies become the sum of weights).
    pub weight: Option<String>,
    /// BY statement variables (one independent analysis per BY group).
    pub by: Vec<(String, bool)>,
}

pub struct TableRequest {
    /// 1 nom = une voie ; 2 noms = crosstab v1*v2.
    pub vars: Vec<String>,
    pub missing: bool,
    pub out: Option<DatasetRef>,
    /// Display-suppression options (parsed AND honored).
    pub nofreq: bool,
    pub nopercent: bool,
    pub norow: bool,
    pub nocol: bool,
    pub nocum: bool,
    /// CHISQ statistics request (one-way goodness-of-fit OR two-way).
    pub chisq: bool,
    /// Fisher exact test (two-way 2x2).
    pub fisher: bool,
    /// AGREE (Cohen's simple kappa, square two-way table).
    pub agree: bool,
    /// MEASURES / RELRISK (odds ratio + relative risks, 2x2).
    pub measures: bool,
    /// TREND (Cochran-Armitage trend test, 2xc or rx2).
    pub trend: bool,
    /// LIST layout (one row per non-empty cell instead of the grid).
    pub list: bool,
}

/// Parse `proc freq [data=a] ; [tables ...;]... run;`. Called AFTER
/// "proc freq" has been consumed. Consumes through `run;`/`quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<FreqAst> {
    let mut data: Option<DatasetRef> = None;

    // --- PROC FREQ statement options, until `;` ---
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
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!(
                    "Unexpected option '{}' on PROC FREQ statement.",
                    name.to_uppercase()
                ),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC FREQ statement.",
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; ---
    let mut tables: Vec<TableRequest> = Vec::new();
    let mut weight: Option<String> = None;
    let mut by: Vec<(String, bool)> = Vec::new();

    // Sous-statements jusqu'à `run;`/`quit;` (combinateur partagé M31).
    common::parse_proc_body(ts, |ts, kw| {
        Ok(match kw {
            "tables" | "table" => {
                ts.next();
                let reqs = parse_tables(ts)?;
                tables.extend(reqs);
                true
            }
            "weight" => {
                ts.next();
                weight = Some(common::parse_weight(ts)?);
                true
            }
            "by" => {
                ts.next();
                by = common::parse_by(ts)?;
                true
            }
            _ => false,
        })
    })?;

    Ok(FreqAst {
        data,
        tables,
        weight,
        by,
    })
}

/// Parse one TABLES statement body (after "tables" consumed), through its
/// terminating `;`. Returns one TableRequest per spec.
fn parse_tables(ts: &mut StatementStream) -> Result<Vec<TableRequest>> {
    let mut specs: Vec<Vec<String>> = Vec::new();

    // Specs until `/` (options) or `;`.
    loop {
        match &ts.peek().kind {
            TokenKind::Semi | TokenKind::Slash | TokenKind::Eof => break,
            _ => {}
        }
        // One spec: v or v1*v2.
        let first_tok = ts.peek().clone();
        let Some(first) = first_tok.ident().map(str::to_string) else {
            return Err(SasError::parse(
                "expected a variable name in the TABLES statement",
                first_tok.span,
            ));
        };
        ts.next();
        let mut vars = vec![first];
        // Allow an arbitrary chain v1*v2*v3*… (n-way crosstab).
        while ts.peek().kind == TokenKind::Star {
            ts.next();
            let snd_tok = ts.peek().clone();
            let Some(snd) = snd_tok.ident().map(str::to_string) else {
                return Err(SasError::parse(
                    "expected a variable name after '*' in the TABLES statement",
                    snd_tok.span,
                ));
            };
            ts.next();
            vars.push(snd);
        }
        specs.push(vars);
    }

    // Options after `/`.
    let mut missing = false;
    let mut out: Option<DatasetRef> = None;
    let mut nofreq = false;
    let mut nopercent = false;
    let mut norow = false;
    let mut nocol = false;
    let mut nocum = false;
    let mut chisq = false;
    let mut fisher = false;
    let mut agree = false;
    let mut measures = false;
    let mut trend = false;
    let mut list = false;
    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        loop {
            match &ts.peek().kind {
                TokenKind::Semi | TokenKind::Eof => break,
                _ => {}
            }
            if ts.peek().is_kw("missing") {
                ts.next();
                missing = true;
            } else if ts.peek().is_kw("out") {
                common::expect_eq(ts, "OUT")?;
                out = Some(ts.parse_dataset_ref()?);
            } else if ts.peek().is_kw("nopercent") {
                ts.next();
                nopercent = true;
            } else if ts.peek().is_kw("norow") {
                ts.next();
                norow = true;
            } else if ts.peek().is_kw("nocol") {
                ts.next();
                nocol = true;
            } else if ts.peek().is_kw("nofreq") {
                ts.next();
                nofreq = true;
            } else if ts.peek().is_kw("nocum") {
                ts.next();
                nocum = true;
            } else if ts.peek().is_kw("chisq") {
                ts.next();
                chisq = true;
            } else if ts.peek().is_kw("fisher") || ts.peek().is_kw("exact") {
                ts.next();
                fisher = true;
            } else if ts.peek().is_kw("agree") {
                ts.next();
                agree = true;
            } else if ts.peek().is_kw("measures") || ts.peek().is_kw("relrisk") {
                ts.next();
                measures = true;
            } else if ts.peek().is_kw("trend") {
                ts.next();
                trend = true;
            } else if ts.peek().is_kw("list") {
                ts.next();
                list = true;
            } else if let Some(name) = ts.peek().ident().map(str::to_string) {
                // Unknown option: ignore leniently (skip the token, and any
                // `=value` that follows).
                ts.next();
                if ts.peek().kind == TokenKind::Eq {
                    ts.next();
                    // skip a single value token (ident/num)
                    if !matches!(ts.peek().kind, TokenKind::Semi | TokenKind::Eof) {
                        ts.next();
                    }
                }
            } else {
                // Unexpected token among options: stop (let expect_semi catch
                // the terminator).
                break;
            }
        }
    }

    ts.expect_semi()?;

    // OUT= requires exactly one table spec on the TABLES statement (SAS rule).
    if out.is_some() && specs.len() != 1 {
        return Err(SasError::runtime(
            "The OUT= option in PROC FREQ requires a single table request on the TABLES statement.",
        ));
    }

    let n = specs.len();
    Ok(specs
        .into_iter()
        .enumerate()
        .map(|(i, vars)| TableRequest {
            vars,
            missing,
            // OUT= only applies (and is only valid) for a single spec.
            out: if i == 0 && n == 1 { out.clone() } else { None },
            nofreq,
            nopercent,
            norow,
            nocol,
            nocum,
            chisq,
            fisher,
            agree,
            measures,
            trend,
            list,
        })
        .collect())
}

/// Find a variable column index by name (case-insensitive), or error.
fn find_var(ds: &SasDataset, name: &str) -> Result<usize> {
    ds.vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", name.to_uppercase())))
}

/// Render a category value for the listing (numeric via format_best, char as
/// the string, missing via its MissingKind display).
fn category_label(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Char(s) => s.trim_end().to_string(),
        Value::Missing(k) => k.display(),
    }
}

/// Format a percentage to two decimals.
fn fmt_pct(p: f64) -> String {
    format!("{p:.2}")
}

/// Format a (possibly weighted) frequency. Integral values print as plain
/// integers (so the unweighted default path stays byte-identical and integer
/// weights still look like counts); fractional weighted frequencies print with
/// the SAS default of two decimals.
fn fmt_freq(f: f64) -> String {
    if (f - f.round()).abs() < 1e-9 {
        format!("{}", f.round() as i64)
    } else {
        format!("{f:.2}")
    }
}

/// A distinct category with its observed (possibly weighted) frequency, in
/// sas_cmp order. With no WEIGHT the frequency is an integer count stored as
/// f64; with WEIGHT it is the sum of the category's weights.
struct Category {
    value: Value,
    freq: f64,
}

/// Tally the distinct values of `col` (restricted to `rows`) into categories
/// ordered by sas_cmp. When `include_missing` is false, missing values are
/// excluded (their frequency is returned separately as `n_missing`).
///
/// When `weights` is `Some`, each observation contributes its weight instead
/// of 1, applying SAS WEIGHT exclusion rules (an observation with a missing or
/// non-positive weight is dropped and counted in `n_weight_excluded`). The
/// "Frequency Missing" tally (`n_missing`) accumulates the WEIGHT of the
/// excluded observations so the weighted accounting stays consistent.
fn tally(
    col: &[Value],
    rows: &[usize],
    include_missing: bool,
    weights: Option<&[Value]>,
) -> (Vec<Category>, f64) {
    let mut cats: Vec<Category> = Vec::new();
    let mut n_missing = 0.0_f64;
    for &i in rows {
        let v = &col[i];
        // Resolve this observation's weight (1.0 when no WEIGHT statement).
        let w = match weights {
            None => 1.0,
            Some(wc) => match value_to_num(&wc[i]) {
                Some(wf) if !wf.is_nan() && wf > 0.0 => wf,
                // Missing or non-positive weight: SAS drops the observation
                // entirely (it contributes neither to a cell nor to the
                // frequency-missing tally for the analysis variable here — but
                // a missing analysis value is still counted below).
                _ => continue,
            },
        };
        if v.is_missing() {
            n_missing += w;
            if !include_missing {
                continue;
            }
        }
        match cats
            .iter_mut()
            .find(|c| c.value.sas_cmp(v) == Ordering::Equal)
        {
            Some(c) => c.freq += w,
            None => cats.push(Category {
                value: v.clone(),
                freq: w,
            }),
        }
    }
    cats.sort_by(|a, b| a.value.sas_cmp(&b.value));
    (cats, n_missing)
}

/// Execute PROC FREQ. Called by `procs::execute_proc`.
pub fn execute(ast: &FreqAst, session: &mut Session) -> Result<()> {
    let in_ref = common::resolve_last_dataset(&ast.data, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }

    let n_obs = ds.n_obs();

    // --- WEIGHT statement: decode the weight column once (or None). ---
    let weight_values: Option<Vec<Value>> = match &ast.weight {
        Some(wname) => {
            let widx = find_var(&ds, wname)?;
            Some(decode_column(&ds, widx)?)
        }
        None => None,
    };

    // --- BY processing: resolve, verify sortedness, partition into groups. ---
    // No BY → a single group spanning all rows (output byte-identical).
    let by_cols = common::resolve_by_cols(&ds, &ast.by)?;
    let by_values: Vec<Vec<Value>> = by_cols
        .iter()
        .map(|c| decode_column(&ds, c.col_idx))
        .collect::<Result<_>>()?;
    let by_names: Vec<String> = by_cols.iter().map(|c| c.name.clone()).collect();
    let by_groups_list: Vec<(Vec<Value>, Vec<usize>)> = if by_cols.is_empty() {
        vec![(Vec::new(), (0..n_obs).collect())]
    } else {
        let descending: Vec<bool> = by_cols.iter().map(|c| c.descending).collect();
        let in_display = format!("{in_libref}.{in_table}");
        common::by_groups(&by_values, &descending, n_obs, &by_names, &in_display)?
    };

    session.listing.page_header();
    // Centered procedure title line.
    let title = "The FREQ Procedure";
    let ls = session.listing.ls();
    let pad = ls.saturating_sub(title.len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), title));
    session.listing.blank();

    for (by_key, grp_rows) in &by_groups_list {
        if !by_names.is_empty() {
            emit_by_heading(session, &by_names, by_key);
        }
        for req in &ast.tables {
            match req.vars.len() {
                1 => one_way(session, &ds, req, grp_rows, weight_values.as_deref())?,
                2 => two_way(session, &ds, req, grp_rows, weight_values.as_deref())?,
                _ => n_way(session, &ds, req, grp_rows, weight_values.as_deref())?,
            }
        }
    }

    Ok(())
}

/// Emit the standard BY-group heading line (`var=val var2=val2`), matching the
/// MEANS/UNIVARIATE rendering.
fn emit_by_heading(session: &mut Session, by_names: &[String], by_key: &[Value]) {
    let parts: Vec<String> = by_names
        .iter()
        .zip(by_key)
        .map(|(name, v)| format!("{}={}", name, category_label(v)))
        .collect();
    session.listing.write_line(&parts.join(" "));
    session.listing.blank();
}

/// One-way frequency table for a single variable, over `rows` (one BY group
/// or all rows). `weights` carries the WEIGHT column when present.
fn one_way(
    session: &mut Session,
    ds: &SasDataset,
    req: &TableRequest,
    rows: &[usize],
    weights: Option<&[Value]>,
) -> Result<()> {
    let col_idx = find_var(ds, &req.vars[0])?;
    let col = decode_column(ds, col_idx)?;
    let var_name = ds.vars[col_idx].name.clone();

    let (cats, n_missing) = tally(&col, rows, req.missing, weights);

    // Denominator: the sum of the category frequencies already gives the
    // right value — with MISSING the missing categories are included in
    // `cats`, otherwise they are excluded (so denom = non-missing count).
    let denom: f64 = cats.iter().map(|c| c.freq).sum();

    // Listing table. Display options suppress whole columns:
    //   NOFREQ    -> drop Frequency
    //   NOPERCENT -> drop Percent (and Cumulative Percent)
    //   NOCUM     -> drop Cumulative Frequency and Cumulative Percent
    // The default (no options) keeps all five columns exactly as before.
    let show_freq = !req.nofreq;
    let show_pct = !req.nopercent;
    let show_cum_freq = !req.nocum;
    let show_cum_pct = !req.nocum && !req.nopercent;

    let mut headers = vec![var_name.clone()];
    let mut aligns = vec![if ds.vars[col_idx].ty == VarType::Num {
        Align::Right
    } else {
        Align::Left
    }];
    if show_freq {
        headers.push("Frequency".to_string());
        aligns.push(Align::Right);
    }
    if show_pct {
        headers.push("Percent".to_string());
        aligns.push(Align::Right);
    }
    if show_cum_freq {
        headers.push("Cumulative Frequency".to_string());
        aligns.push(Align::Right);
    }
    if show_cum_pct {
        headers.push("Cumulative Percent".to_string());
        aligns.push(Align::Right);
    }

    let mut out_rows: Vec<Vec<String>> = Vec::with_capacity(cats.len());
    let mut cum_freq = 0.0_f64;
    for c in &cats {
        cum_freq += c.freq;
        let pct = if denom > 0.0 {
            100.0 * c.freq / denom
        } else {
            0.0
        };
        let cum_pct = if denom > 0.0 {
            100.0 * cum_freq / denom
        } else {
            0.0
        };
        let mut row = vec![category_label(&c.value)];
        if show_freq {
            row.push(fmt_freq(c.freq));
        }
        if show_pct {
            row.push(fmt_pct(pct));
        }
        if show_cum_freq {
            row.push(fmt_freq(cum_freq));
        }
        if show_cum_pct {
            row.push(fmt_pct(cum_pct));
        }
        out_rows.push(row);
    }

    session.listing.write_table(&headers, &aligns, &out_rows);

    // Frequency Missing line (only when missings are excluded).
    if !req.missing && n_missing > 0.0 {
        session.listing.blank();
        session
            .listing
            .write_line(&format!("Frequency Missing = {}", fmt_freq(n_missing)));
    }

    // CHISQ one-way: goodness-of-fit against equal proportions.
    if req.chisq {
        chisq_one_way_block(session, &cats);
    }

    // OUT= dataset (one-way only).
    if let Some(out) = &req.out {
        write_one_way_out(session, ds, col_idx, &cats, denom, out)?;
    }

    Ok(())
}

/// One-way goodness-of-fit chi-square test against equal proportions
/// (TESTP= defaulting to 1/k per category). Statistic Σ(obs-exp)²/exp with
/// exp = N/k, DF = k-1. Degenerate cases (k < 2 or N = 0) are skipped with a
/// graceful note.
fn chisq_one_way_block(session: &mut Session, cats: &[Category]) {
    let k = cats.len();
    let n: f64 = cats.iter().map(|c| c.freq).sum();

    session.listing.blank();
    if k < 2 || n <= 0.0 {
        session
            .listing
            .write_line("Chi-Square Test for Equal Proportions is not computable for this table.");
        return;
    }

    let exp = n / k as f64;
    let mut chisq = 0.0_f64;
    for c in cats {
        let d = c.freq - exp;
        chisq += d * d / exp;
    }
    let df = (k - 1) as f64;
    let p = chisq_sf(chisq, df);

    session
        .listing
        .write_line("Chi-Square Test for Equal Proportions");
    session.listing.blank();
    let headers = vec!["Statistic".to_string(), "Value".to_string()];
    let aligns = vec![Align::Left, Align::Right];
    let rows = vec![
        vec!["Chi-Square".to_string(), format!("{chisq:.4}")],
        vec!["DF".to_string(), format!("{}", k - 1)],
        vec!["Pr > ChiSq".to_string(), fmt_chisq_p(p)],
    ];
    session.listing.write_table(&headers, &aligns, &rows);
}

/// Build and write the OUT= dataset for a one-way table: columns <var>,
/// COUNT, PERCENT.
fn write_one_way_out(
    session: &mut Session,
    ds: &SasDataset,
    col_idx: usize,
    cats: &[Category],
    denom: f64,
    out: &DatasetRef,
) -> Result<()> {
    let meta = &ds.vars[col_idx];
    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();

    // Category column (same type/meta as the input variable).
    let cat_series = match meta.ty {
        VarType::Num => {
            let vals: Vec<Option<f64>> =
                cats.iter().map(|c| value_to_num(&c.value)).collect();
            Series::new(meta.name.as_str().into(), vals)
        }
        VarType::Char => {
            let vals: Vec<Option<String>> = cats
                .iter()
                .map(|c| match &c.value {
                    Value::Char(s) if s.trim_end().is_empty() => None,
                    Value::Char(s) => Some(s.trim_end().to_string()),
                    _ => None,
                })
                .collect();
            Series::new(meta.name.as_str().into(), vals)
        }
    };
    columns.push(cat_series.into());
    vars.push(meta.clone());

    // COUNT.
    let count_vals: Vec<Option<f64>> = cats.iter().map(|c| Some(c.freq)).collect();
    columns.push(Series::new("COUNT".into(), count_vals).into());
    vars.push(num_var_meta("COUNT"));

    // PERCENT.
    let pct_vals: Vec<Option<f64>> = cats
        .iter()
        .map(|c| {
            Some(if denom > 0.0 {
                100.0 * c.freq / denom
            } else {
                0.0
            })
        })
        .collect();
    columns.push(Series::new("PERCENT".into(), pct_vals).into());
    vars.push(num_var_meta("PERCENT"));

    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars };

    let out_libref = out.libref_or_work();
    let out_table = out.name.to_uppercase();
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

/// Resolve this observation's weight (1.0 when no WEIGHT), applying the SAS
/// exclusion rules (missing/non-positive → None, i.e. drop the observation).
fn obs_weight(weights: Option<&[Value]>, i: usize) -> Option<f64> {
    match weights {
        None => Some(1.0),
        Some(wc) => match value_to_num(&wc[i]) {
            Some(wf) if !wf.is_nan() && wf > 0.0 => Some(wf),
            _ => None,
        },
    }
}

/// Distinct sas_cmp-ordered values of `col` over `rows`, keeping missings only
/// when `include_missing` is set, and only for observations with a usable
/// weight.
fn distinct_axis(
    col: &[Value],
    rows: &[usize],
    include_missing: bool,
    weights: Option<&[Value]>,
) -> Vec<Value> {
    let mut vals: Vec<Value> = Vec::new();
    for &i in rows {
        if obs_weight(weights, i).is_none() {
            continue;
        }
        let v = &col[i];
        if (include_missing || !v.is_missing())
            && !vals.iter().any(|x| x.sas_cmp(v) == Ordering::Equal)
        {
            vals.push(v.clone());
        }
    }
    vals.sort_by(|a, b| a.sas_cmp(b));
    vals
}

/// Round a weighted frequency matrix to integer counts for the integer-only
/// statistics blocks (Fisher/MEASURES/AGREE/TREND). These tests are defined on
/// counts; with integer weights the rounding is exact, with fractional weights
/// it is a documented approximation (SAS itself only supports these on
/// frequency counts). CHISQ uses the exact weighted values directly.
fn round_matrix(freq: &[Vec<f64>]) -> Vec<Vec<usize>> {
    freq.iter()
        .map(|row| row.iter().map(|&f| f.round().max(0.0) as usize).collect())
        .collect()
}

/// Two-way crosstab for v1*v2, over `rows` with optional weights.
fn two_way(
    session: &mut Session,
    ds: &SasDataset,
    req: &TableRequest,
    rows: &[usize],
    weights: Option<&[Value]>,
) -> Result<()> {
    let row_idx = find_var(ds, &req.vars[0])?;
    let col_idx = find_var(ds, &req.vars[1])?;
    let row_col = decode_column(ds, row_idx)?;
    let col_col = decode_column(ds, col_idx)?;
    let row_name = ds.vars[row_idx].name.clone();
    let col_name = ds.vars[col_idx].name.clone();

    let keep = |v: &Value| req.missing || !v.is_missing();
    let row_vals = distinct_axis(&row_col, rows, req.missing, weights);
    let col_vals = distinct_axis(&col_col, rows, req.missing, weights);

    let nr = row_vals.len();
    let nc = col_vals.len();

    // freq[r][c] = sum of weights (or counts) for the cell.
    let mut freq = vec![vec![0.0_f64; nc]; nr];
    for &i in rows {
        let Some(w) = obs_weight(weights, i) else {
            continue;
        };
        let rv = &row_col[i];
        let cv = &col_col[i];
        if !keep(rv) || !keep(cv) {
            continue;
        }
        let r = row_vals.iter().position(|x| x.sas_cmp(rv) == Ordering::Equal);
        let c = col_vals.iter().position(|x| x.sas_cmp(cv) == Ordering::Equal);
        if let (Some(r), Some(c)) = (r, c) {
            freq[r][c] += w;
        }
    }

    render_two_way(session, req, &row_name, &col_name, &row_vals, &col_vals, &freq);
    Ok(())
}

/// Render a two-way crosstab from a computed weighted frequency matrix:
/// grid layout (default) or LIST layout (`/LIST`), followed by any requested
/// statistic blocks. Shared by `two_way` and the n-way stratified renderer.
fn render_two_way(
    session: &mut Session,
    req: &TableRequest,
    row_name: &str,
    col_name: &str,
    row_vals: &[Value],
    col_vals: &[Value],
    freq: &[Vec<f64>],
) {
    let nr = row_vals.len();
    let nc = col_vals.len();

    let row_tot: Vec<f64> = (0..nr).map(|r| freq[r].iter().sum()).collect();
    let col_tot: Vec<f64> = (0..nc).map(|c| (0..nr).map(|r| freq[r][c]).sum()).collect();
    let grand: f64 = row_tot.iter().sum();

    // LIST layout: one row per non-empty cell, suppressing the grid and the
    // row/col percentages (SAS LIST shows Frequency / Percent / Cumulative).
    if req.list {
        render_two_way_list(
            session, req, row_name, col_name, row_vals, col_vals, freq, grand,
        );
        emit_two_way_stats(session, req, row_name, col_name, freq, &row_tot, &col_tot, grand);
        return;
    }

    // Which stacked per-cell lines to show. Display options drop a line:
    //   NOFREQ    -> Frequency, NOPERCENT -> Percent,
    //   NOROW     -> Row Pct,   NOCOL     -> Col Pct.
    // Default (no options) keeps all four, exactly as before.
    let show_freq = !req.nofreq;
    let show_pct = !req.nopercent;
    let show_rowp = !req.norow;
    let show_colp = !req.nocol;

    // Legend reflecting the lines actually shown.
    let mut legend_parts: Vec<&str> = Vec::new();
    if show_freq {
        legend_parts.push("Frequency");
    }
    if show_pct {
        legend_parts.push("Percent");
    }
    if show_rowp {
        legend_parts.push("Row Pct");
    }
    if show_colp {
        legend_parts.push("Col Pct");
    }

    session
        .listing
        .write_line(&format!("Table of {row_name} by {col_name}"));
    session.listing.blank();
    if !legend_parts.is_empty() {
        session
            .listing
            .write_line(&format!("Cell contents: {}", legend_parts.join(" / ")));
        session.listing.blank();
    }

    // Header: row-var name, one column per col value, then Total.
    let mut headers = vec![row_name.to_string()];
    for cv in col_vals {
        headers.push(category_label(cv));
    }
    headers.push("Total".to_string());
    let mut aligns = vec![Align::Left];
    for _ in 0..nc {
        aligns.push(Align::Right);
    }
    aligns.push(Align::Right);

    // Each logical row -> 4 physical rows (Frequency, Percent, Row Pct,
    // Col Pct). The first physical row carries the row-value label.
    let mut rows: Vec<Vec<String>> = Vec::new();
    let pct_of = |num: f64, den: f64| -> String {
        if den > 0.0 {
            fmt_pct(100.0 * num / den)
        } else {
            fmt_pct(0.0)
        }
    };

    // The row-value label rides on the first physical line that is shown.
    let label_on_freq = show_freq;
    let label_on_pct = !show_freq && show_pct;
    let label_on_rowp = !show_freq && !show_pct && show_rowp;
    let label_on_colp = !show_freq && !show_pct && !show_rowp && show_colp;

    for r in 0..nr {
        let mut line_freq = vec![if label_on_freq {
            category_label(&row_vals[r])
        } else {
            String::new()
        }];
        let mut line_pct = vec![if label_on_pct {
            category_label(&row_vals[r])
        } else {
            String::new()
        }];
        let mut line_rowp = vec![if label_on_rowp {
            category_label(&row_vals[r])
        } else {
            String::new()
        }];
        let mut line_colp = vec![if label_on_colp {
            category_label(&row_vals[r])
        } else {
            String::new()
        }];
        for c in 0..nc {
            let f = freq[r][c];
            line_freq.push(fmt_freq(f));
            line_pct.push(pct_of(f, grand));
            line_rowp.push(pct_of(f, row_tot[r]));
            line_colp.push(pct_of(f, col_tot[c]));
        }
        // Row total margin: Frequency + Percent only.
        line_freq.push(fmt_freq(row_tot[r]));
        line_pct.push(pct_of(row_tot[r], grand));
        line_rowp.push(String::new());
        line_colp.push(String::new());
        if show_freq {
            rows.push(line_freq);
        }
        if show_pct {
            rows.push(line_pct);
        }
        if show_rowp {
            rows.push(line_rowp);
        }
        if show_colp {
            rows.push(line_colp);
        }
    }

    // Total row (column totals + grand total): Frequency + Percent only.
    let mut tot_freq = vec!["Total".to_string()];
    let mut tot_pct = vec![String::new()];
    for c in 0..nc {
        tot_freq.push(fmt_freq(col_tot[c]));
        tot_pct.push(pct_of(col_tot[c], grand));
    }
    tot_freq.push(fmt_freq(grand));
    tot_pct.push(pct_of(grand, grand));
    if show_freq {
        rows.push(tot_freq);
    }
    if show_pct {
        // When the Frequency line is suppressed the "Total" label needs to
        // land on the percent line so the margin row stays identifiable.
        if !show_freq {
            tot_pct[0] = "Total".to_string();
        }
        rows.push(tot_pct);
    }

    session.listing.write_table(&headers, &aligns, &rows);

    emit_two_way_stats(session, req, row_name, col_name, freq, &row_tot, &col_tot, grand);
}

/// Print all requested statistic blocks for a two-way table. CHISQ uses the
/// exact (possibly weighted) frequencies; the integer-count tests
/// (Fisher/MEASURES/AGREE/TREND) operate on a rounded copy.
#[allow(clippy::too_many_arguments)]
fn emit_two_way_stats(
    session: &mut Session,
    req: &TableRequest,
    row_name: &str,
    col_name: &str,
    freq: &[Vec<f64>],
    row_tot: &[f64],
    col_tot: &[f64],
    grand: f64,
) {
    if req.chisq {
        chisq_block(session, row_name, col_name, freq, row_tot, col_tot, grand);
    }
    if req.fisher || req.trend || req.measures || req.agree {
        let ifreq = round_matrix(freq);
        let irow: Vec<usize> = ifreq.iter().map(|r| r.iter().sum()).collect();
        let icol: Vec<usize> = (0..col_tot.len())
            .map(|c| (0..ifreq.len()).map(|r| ifreq[r][c]).sum())
            .collect();
        let igrand: usize = irow.iter().sum();
        if req.fisher {
            fisher_block(session, &ifreq, &irow, &icol, igrand);
        }
        if req.trend {
            trend_block(session, &ifreq, &irow, &icol, igrand);
        }
        if req.measures {
            measures_block(session, &ifreq);
        }
        if req.agree {
            agree_block(session, &ifreq, &irow, &icol, igrand);
        }
    }
}

/// Render a two-way table in LIST layout: one row per non-empty cell, with
/// columns (row var, col var, Frequency, Percent, Cumulative Frequency,
/// Cumulative Percent). Cells are walked in sas_cmp order (row-major). LIST
/// suppresses the grid and the Row/Col percentages.
#[allow(clippy::too_many_arguments)]
fn render_two_way_list(
    session: &mut Session,
    req: &TableRequest,
    row_name: &str,
    col_name: &str,
    row_vals: &[Value],
    col_vals: &[Value],
    freq: &[Vec<f64>],
    grand: f64,
) {
    session
        .listing
        .write_line(&format!("Table of {row_name} by {col_name}"));
    session.listing.blank();

    let headers = vec![
        row_name.to_string(),
        col_name.to_string(),
        "Frequency".to_string(),
        "Percent".to_string(),
        "Cumulative Frequency".to_string(),
        "Cumulative Percent".to_string(),
    ];
    let aligns = vec![
        Align::Left,
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut cum = 0.0_f64;
    for r in 0..row_vals.len() {
        for c in 0..col_vals.len() {
            let f = freq[r][c];
            if f == 0.0 {
                continue; // LIST prints only non-empty cells.
            }
            cum += f;
            let pct = if grand > 0.0 { 100.0 * f / grand } else { 0.0 };
            let cum_pct = if grand > 0.0 { 100.0 * cum / grand } else { 0.0 };
            rows.push(vec![
                category_label(&row_vals[r]),
                category_label(&col_vals[c]),
                fmt_freq(f),
                fmt_pct(pct),
                fmt_freq(cum),
                fmt_pct(cum_pct),
            ]);
        }
    }

    session.listing.write_table(&headers, &aligns, &rows);
}

/// n-way (≥3 variables) crosstab. SAS prints this as a series of two-way
/// tables of the LAST two variables, stratified by the distinct combinations
/// of the leading variable(s). Each stratum is preceded by a header line
/// naming the controlling values, then rendered with the existing two-way
/// layout (grid or LIST) and statistics.
fn n_way(
    session: &mut Session,
    ds: &SasDataset,
    req: &TableRequest,
    rows: &[usize],
    weights: Option<&[Value]>,
) -> Result<()> {
    let k = req.vars.len();
    // Resolve all columns once.
    let cols: Vec<Vec<Value>> = req
        .vars
        .iter()
        .map(|v| find_var(ds, v).and_then(|i| decode_column(ds, i)))
        .collect::<Result<_>>()?;
    let names: Vec<String> = req
        .vars
        .iter()
        .map(|v| {
            find_var(ds, v)
                .map(|i| ds.vars[i].name.clone())
                .unwrap_or_else(|_| v.clone())
        })
        .collect();

    // The leading vars (all but the last two) define the strata.
    let lead = k - 2;
    let row_col = &cols[k - 2];
    let col_col = &cols[k - 1];
    let row_name = &names[k - 2];
    let col_name = &names[k - 1];

    let keep = |v: &Value| req.missing || !v.is_missing();

    // Distinct stratum keys (tuple of leading values) in sas_cmp order, only
    // over rows that pass the keep filter on the stratum vars and have a usable
    // weight.
    let lead_cols: Vec<&Vec<Value>> = (0..lead).map(|j| &cols[j]).collect();
    let mut stratum_rows: Vec<usize> = Vec::new();
    for &i in rows {
        if obs_weight(weights, i).is_none() {
            continue;
        }
        if (0..lead).all(|j| keep(&cols[j][i])) {
            stratum_rows.push(i);
        }
    }
    let strata = common::group_by_keys(&lead_cols, ds.n_obs());
    // group_by_keys walks all rows; restrict each stratum to our `rows` subset.
    let row_set: std::collections::HashSet<usize> = stratum_rows.iter().copied().collect();

    for (key, all_grp_rows) in &strata {
        let grp_rows: Vec<usize> = all_grp_rows
            .iter()
            .copied()
            .filter(|i| row_set.contains(i))
            .collect();
        if grp_rows.is_empty() {
            continue;
        }
        // Stratum header: lead1=val1 lead2=val2 ...
        let header: Vec<String> = (0..lead)
            .map(|j| format!("{}={}", names[j], category_label(&key[j])))
            .collect();
        session
            .listing
            .write_line(&format!("Controlling for {}", header.join(" ")));
        session.listing.blank();

        // Build the two-way frequency matrix for this stratum.
        let row_vals = distinct_axis(row_col, &grp_rows, req.missing, weights);
        let col_vals = distinct_axis(col_col, &grp_rows, req.missing, weights);
        let nr = row_vals.len();
        let nc = col_vals.len();
        let mut freq = vec![vec![0.0_f64; nc]; nr];
        for &i in &grp_rows {
            let Some(w) = obs_weight(weights, i) else {
                continue;
            };
            let rv = &row_col[i];
            let cv = &col_col[i];
            if !keep(rv) || !keep(cv) {
                continue;
            }
            let r = row_vals.iter().position(|x| x.sas_cmp(rv) == Ordering::Equal);
            let c = col_vals.iter().position(|x| x.sas_cmp(cv) == Ordering::Equal);
            if let (Some(r), Some(c)) = (r, c) {
                freq[r][c] += w;
            }
        }

        render_two_way(session, req, row_name, col_name, &row_vals, &col_vals, &freq);
        session.listing.blank();
    }

    Ok(())
}

/// Fisher's exact test. Full exact two-sided p-value for 2x2 tables (sum of
/// hypergeometric probabilities ≤ that of the observed table), plus the
/// left/right one-sided tails and the observed table probability. Tables
/// larger than 2x2 are deferred with a graceful note (no panic).
fn fisher_block(
    session: &mut Session,
    freq: &[Vec<usize>],
    row_tot: &[usize],
    col_tot: &[usize],
    grand: usize,
) {
    let nr = row_tot.len();
    let nc = col_tot.len();
    session.listing.blank();
    session.listing.write_line("Fisher's Exact Test");
    session.listing.blank();

    if nr != 2 || nc != 2 {
        session
            .listing
            .write_line("Fisher's exact test for tables larger than 2x2 is not supported.");
        return;
    }
    if grand == 0 {
        session
            .listing
            .write_line("Fisher's Exact Test is not computable for this table.");
        return;
    }

    // Margins are fixed. With r1 = row_tot[0], c1 = col_tot[0], n = grand, the
    // count a = freq[0][0] determines the whole table. a ranges over
    // [max(0, r1+c1-n), min(r1, c1)]. The hypergeometric probability of a is
    // C(r1,a)·C(r2,c1-a)/C(n,c1).
    let r1 = row_tot[0] as i64;
    let r2 = row_tot[1] as i64;
    let c1 = col_tot[0] as i64;
    let n = grand as i64;
    let a_obs = freq[0][0] as i64;

    let ln_p = |a: i64| -> f64 {
        let b = c1 - a; // freq[1][0]
        ln_choose(r1 as u64, a as u64) + ln_choose(r2 as u64, b as u64)
            - ln_choose(n as u64, c1 as u64)
    };

    let lo = 0.max(r1 + c1 - n);
    let hi = r1.min(c1);
    let p_obs = ln_p(a_obs).exp();

    let mut p_left = 0.0_f64; // P(A <= a_obs)
    let mut p_right = 0.0_f64; // P(A >= a_obs)
    let mut p_two = 0.0_f64; // sum of probs <= p_obs (with tolerance)
    let tol = 1e-7;
    for a in lo..=hi {
        let p = ln_p(a).exp();
        if a <= a_obs {
            p_left += p;
        }
        if a >= a_obs {
            p_right += p;
        }
        if p <= p_obs * (1.0 + tol) {
            p_two += p;
        }
    }
    let clamp = |p: f64| p.clamp(0.0, 1.0);

    let headers = vec!["Statistic".to_string(), "Value".to_string()];
    let aligns = vec![Align::Left, Align::Right];
    let rows = vec![
        vec!["Cell (1,1) Frequency (F)".to_string(), format!("{a_obs}")],
        vec![
            "Left-sided Pr <= F".to_string(),
            fmt_chisq_p(clamp(p_left)),
        ],
        vec![
            "Right-sided Pr >= F".to_string(),
            fmt_chisq_p(clamp(p_right)),
        ],
        vec!["Table Probability (P)".to_string(), fmt_chisq_p(clamp(p_obs))],
        vec!["Two-sided Pr <= P".to_string(), fmt_chisq_p(clamp(p_two))],
    ];
    session.listing.write_table(&headers, &aligns, &rows);
}

/// Cochran-Armitage trend test. Requires a 2-row (or 2-column) table; the
/// non-binary dimension supplies ordinal scores 1..k. Reports the Z statistic
/// with one- and two-sided normal-approximation p-values. Other shapes are
/// deferred with a graceful note.
fn trend_block(
    session: &mut Session,
    freq: &[Vec<usize>],
    row_tot: &[usize],
    col_tot: &[usize],
    grand: usize,
) {
    let nr = row_tot.len();
    let nc = col_tot.len();
    session.listing.blank();
    session.listing.write_line("Cochran-Armitage Trend Test");
    session.listing.blank();

    if grand == 0 || (nr != 2 && nc != 2) || nr < 2 || nc < 2 {
        session
            .listing
            .write_line("The Cochran-Armitage Trend Test requires a 2xC or Rx2 table.");
        return;
    }

    // Orient so that there are 2 rows and `k` ordinal columns. If the table is
    // Rx2 instead, transpose roles (scores along rows).
    // We compute using the first row's counts (n_{1i}) against column totals.
    // T = Σ s_i (n_{1i} - r1 * c_i / N).
    // Var(T) = (r1*r2/N) * [ Σ c_i s_i² - (Σ c_i s_i)² / N ].
    let (cells_row1, marg): (Vec<f64>, Vec<f64>);
    let r1f: f64;
    let r2f: f64;
    if nr == 2 {
        cells_row1 = (0..nc).map(|c| freq[0][c] as f64).collect();
        marg = col_tot.iter().map(|&c| c as f64).collect();
        r1f = row_tot[0] as f64;
        r2f = row_tot[1] as f64;
    } else {
        // Rx2: treat columns as the binary dimension, rows as ordinal scores.
        cells_row1 = (0..nr).map(|r| freq[r][0] as f64).collect();
        marg = row_tot.iter().map(|&r| r as f64).collect();
        r1f = col_tot[0] as f64;
        r2f = col_tot[1] as f64;
    }
    let k = cells_row1.len();
    let scores: Vec<f64> = (1..=k).map(|i| i as f64).collect();
    let nf = grand as f64;

    let mut t = 0.0_f64;
    let mut sum_cs = 0.0_f64; // Σ c_i s_i
    let mut sum_cs2 = 0.0_f64; // Σ c_i s_i²
    for i in 0..k {
        t += scores[i] * (cells_row1[i] - r1f * marg[i] / nf);
        sum_cs += marg[i] * scores[i];
        sum_cs2 += marg[i] * scores[i] * scores[i];
    }
    let var = (r1f * r2f / nf) * (sum_cs2 - sum_cs * sum_cs / nf);

    if var <= 0.0 {
        session
            .listing
            .write_line("The Cochran-Armitage Trend Test is not computable for this table.");
        return;
    }
    let z = t / var.sqrt();
    // One-sided p toward the observed direction; two-sided = 2*one-sided.
    let p_one = 1.0 - probnorm(z.abs());
    let p_two = (2.0 * p_one).min(1.0);

    let headers = vec!["Statistic".to_string(), "Value".to_string()];
    let aligns = vec![Align::Left, Align::Right];
    let rows = vec![
        vec!["Statistic (Z)".to_string(), format!("{z:.4}")],
        vec!["One-sided Pr".to_string(), fmt_chisq_p(p_one.clamp(0.0, 1.0))],
        vec!["Two-sided Pr".to_string(), fmt_chisq_p(p_two.clamp(0.0, 1.0))],
    ];
    session.listing.write_table(&headers, &aligns, &rows);
}

/// MEASURES / RELRISK: odds ratio and the two cohort relative risks for a 2x2
/// table, each with a 95% confidence interval (Wald, on the log scale). Cells
/// containing zeros yield missing estimates rather than dividing by zero.
fn measures_block(session: &mut Session, freq: &[Vec<usize>]) {
    session.listing.blank();
    session
        .listing
        .write_line("Estimates of the Relative Risk (Row1/Row2)");
    session.listing.blank();

    if freq.len() != 2 || freq[0].len() != 2 || freq[1].len() != 2 {
        session
            .listing
            .write_line("Relative risk estimates require a 2x2 table.");
        return;
    }

    let a = freq[0][0] as f64;
    let b = freq[0][1] as f64;
    let c = freq[1][0] as f64;
    let d = freq[1][1] as f64;

    let headers = vec![
        "Type of Study".to_string(),
        "Value".to_string(),
        "95% Confidence Limits".to_string(),
    ];
    let aligns = vec![Align::Left, Align::Right, Align::Left];
    let mut rows: Vec<Vec<String>> = Vec::new();

    // Helper rendering "lo   hi" or "." when an estimate is undefined.
    let limits = |lo: f64, hi: f64, ok: bool| -> String {
        if ok {
            format!("{lo:.4}   {hi:.4}")
        } else {
            ".".to_string()
        }
    };

    // Odds ratio = ad/bc; SE(ln OR) = sqrt(1/a+1/b+1/c+1/d).
    if a > 0.0 && b > 0.0 && c > 0.0 && d > 0.0 {
        let or = (a * d) / (b * c);
        let se = (1.0 / a + 1.0 / b + 1.0 / c + 1.0 / d).sqrt();
        let (lo, hi) = (
            (or.ln() - 1.96 * se).exp(),
            (or.ln() + 1.96 * se).exp(),
        );
        rows.push(vec![
            "Case-Control (Odds Ratio)".to_string(),
            format!("{or:.4}"),
            limits(lo, hi, true),
        ]);
    } else {
        rows.push(vec![
            "Case-Control (Odds Ratio)".to_string(),
            ".".to_string(),
            ".".to_string(),
        ]);
    }

    // Cohort (Col1 Risk): RR = [a/(a+b)] / [c/(c+d)].
    let r1 = a + b;
    let r2 = c + d;
    if r1 > 0.0 && r2 > 0.0 && a > 0.0 && c > 0.0 {
        let rr = (a / r1) / (c / r2);
        let se = (b / (a * r1) + d / (c * r2)).sqrt();
        let (lo, hi) = ((rr.ln() - 1.96 * se).exp(), (rr.ln() + 1.96 * se).exp());
        rows.push(vec![
            "Cohort (Col1 Risk)".to_string(),
            format!("{rr:.4}"),
            limits(lo, hi, true),
        ]);
    } else {
        rows.push(vec![
            "Cohort (Col1 Risk)".to_string(),
            ".".to_string(),
            ".".to_string(),
        ]);
    }

    // Cohort (Col2 Risk): RR = [b/(a+b)] / [d/(c+d)].
    if r1 > 0.0 && r2 > 0.0 && b > 0.0 && d > 0.0 {
        let rr = (b / r1) / (d / r2);
        let se = (a / (b * r1) + c / (d * r2)).sqrt();
        let (lo, hi) = ((rr.ln() - 1.96 * se).exp(), (rr.ln() + 1.96 * se).exp());
        rows.push(vec![
            "Cohort (Col2 Risk)".to_string(),
            format!("{rr:.4}"),
            limits(lo, hi, true),
        ]);
    } else {
        rows.push(vec![
            "Cohort (Col2 Risk)".to_string(),
            ".".to_string(),
            ".".to_string(),
        ]);
    }

    session.listing.write_table(&headers, &aligns, &rows);
}

/// AGREE: Cohen's simple kappa coefficient for a square table, with its
/// asymptotic standard error and a 95% confidence interval. Non-square tables
/// are rejected with a graceful note.
fn agree_block(
    session: &mut Session,
    freq: &[Vec<usize>],
    row_tot: &[usize],
    col_tot: &[usize],
    grand: usize,
) {
    let nr = row_tot.len();
    let nc = col_tot.len();
    session.listing.blank();
    session.listing.write_line("Simple Kappa Coefficient");
    session.listing.blank();

    if nr != nc {
        session
            .listing
            .write_line("AGREE requires a square table.");
        return;
    }
    if grand == 0 {
        session
            .listing
            .write_line("Simple Kappa Coefficient is not computable for this table.");
        return;
    }

    let n = grand as f64;
    // Observed agreement Po = Σ p_ii ; expected Pe = Σ p_i+ · p_+i.
    let mut po = 0.0_f64;
    let mut pe = 0.0_f64;
    for i in 0..nr {
        po += freq[i][i] as f64 / n;
        pe += (row_tot[i] as f64 / n) * (col_tot[i] as f64 / n);
    }

    if (1.0 - pe).abs() < 1e-12 {
        session
            .listing
            .write_line("Simple Kappa Coefficient is not computable (perfect expected agreement).");
        return;
    }
    let kappa = (po - pe) / (1.0 - pe);

    // Asymptotic standard error under H1 (Fleiss et al.), the SAS ASE.
    // ASE = sqrt( [ A + B - C ] / [ (1-Pe)² · n ] ) with
    //   A = Σ p_ii [1 - (p_i+ + p_+i)(1 - kappa)]²
    //   B = (1-kappa)² Σ_{i≠j} p_ij (p_+i + p_j+)²
    //   C = (kappa - Pe(1-kappa))²
    let p = |i: usize, j: usize| freq[i][j] as f64 / n;
    let pr = |i: usize| row_tot[i] as f64 / n; // p_i+ (row marginal)
    let pc = |j: usize| col_tot[j] as f64 / n; // p_+j (col marginal)

    let mut term_a = 0.0_f64;
    for i in 0..nr {
        let s = 1.0 - (pr(i) + pc(i)) * (1.0 - kappa);
        term_a += p(i, i) * s * s;
    }
    let mut term_b = 0.0_f64;
    for i in 0..nr {
        for j in 0..nc {
            if i != j {
                let s = pc(i) + pr(j);
                term_b += p(i, j) * s * s;
            }
        }
    }
    term_b *= (1.0 - kappa) * (1.0 - kappa);
    let term_c = (kappa - pe * (1.0 - kappa)).powi(2);

    let var = (term_a + term_b - term_c) / ((1.0 - pe).powi(2) * n);
    let ase = if var > 0.0 { var.sqrt() } else { 0.0 };
    let lower = kappa - 1.96 * ase;
    let upper = kappa + 1.96 * ase;

    let headers = vec!["Statistic".to_string(), "Value".to_string()];
    let aligns = vec![Align::Left, Align::Right];
    let rows = vec![
        vec!["Kappa".to_string(), format!("{kappa:.4}")],
        vec!["ASE".to_string(), format!("{ase:.4}")],
        vec!["95% Lower Conf Limit".to_string(), format!("{lower:.4}")],
        vec!["95% Upper Conf Limit".to_string(), format!("{upper:.4}")],
    ];
    session.listing.write_table(&headers, &aligns, &rows);
}

/// Format a p-value SAS-style: `<.0001`, else 4 decimals (mirrors corr.rs).
fn fmt_chisq_p(p: f64) -> String {
    if p < 0.0001 {
        "<.0001".to_string()
    } else {
        format!("{p:.4}")
    }
}

/// Print the "Statistics for Table of <row> by <col>" CHISQ block for a
/// two-way table: Pearson chi-square and the likelihood-ratio chi-square,
/// each with DF and an upper-tail p-value. Degenerate tables (grand total 0,
/// any zero margin, or DF <= 0) are skipped gracefully with a note.
fn chisq_block(
    session: &mut Session,
    row_name: &str,
    col_name: &str,
    freq: &[Vec<f64>],
    row_tot: &[f64],
    col_tot: &[f64],
    grand: f64,
) {
    session.listing.blank();
    session
        .listing
        .write_line(&format!("Statistics for Table of {row_name} by {col_name}"));
    session.listing.blank();

    let nr = row_tot.len();
    let nc = col_tot.len();
    let df = (nr.saturating_sub(1)) * (nc.saturating_sub(1));

    // Guard against degenerate tables: no expected counts are defined.
    if grand <= 0.0
        || df == 0
        || row_tot.iter().any(|&t| t <= 0.0)
        || col_tot.iter().any(|&t| t <= 0.0)
    {
        session
            .listing
            .write_line("Chi-Square statistics are not computable for this table.");
        return;
    }

    let g = grand;
    let mut pearson = 0.0_f64;
    let mut lratio = 0.0_f64;
    for r in 0..nr {
        for c in 0..nc {
            let e = row_tot[r] * col_tot[c] / g;
            let n = freq[r][c];
            if e > 0.0 {
                let d = n - e;
                pearson += d * d / e;
            }
            if n > 0.0 && e > 0.0 {
                lratio += n * (n / e).ln();
            }
        }
    }
    lratio *= 2.0;

    let df_f = df as f64;
    let p_pearson = chisq_sf(pearson, df_f);
    let p_lratio = chisq_sf(lratio, df_f);

    let headers = vec![
        "Statistic".to_string(),
        "DF".to_string(),
        "Value".to_string(),
        "Prob".to_string(),
    ];
    let aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right];
    let rows = vec![
        vec![
            "Chi-Square".to_string(),
            format!("{df}"),
            format!("{pearson:.4}"),
            fmt_chisq_p(p_pearson),
        ],
        vec![
            "Likelihood Ratio Chi-Square".to_string(),
            format!("{df}"),
            format!("{lratio:.4}"),
            fmt_chisq_p(p_lratio),
        ],
    ];
    session.listing.write_table(&headers, &aligns, &rows);
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

    fn parse_freq(src: &str) -> Result<FreqAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "freq"
        parse(&mut ts)
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

    /// Build a TableRequest with all display/chisq options off (defaults).
    fn tr(vars: &[&str], missing: bool, out: Option<DatasetRef>) -> TableRequest {
        TableRequest {
            vars: vars.iter().map(|s| s.to_string()).collect(),
            missing,
            out,
            nofreq: false,
            nopercent: false,
            norow: false,
            nocol: false,
            nocum: false,
            chisq: false,
            fisher: false,
            agree: false,
            measures: false,
            trend: false,
            list: false,
        }
    }

    /// Build a FreqAst with no WEIGHT/BY (test convenience).
    fn fast(data: DatasetRef, tables: Vec<TableRequest>) -> FreqAst {
        FreqAst {
            data: Some(data),
            tables,
            weight: None,
            by: Vec::new(),
        }
    }

    fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
        session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
        session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
    }

    fn read_col(session: &Session, table: &str, col: &str) -> Vec<Value> {
        let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
        let idx = ds.vars.iter().position(|m| m.name == col).unwrap();
        decode_column(&ds, idx).unwrap()
    }

    // ───────────────────────────── parse tests ─────────────────────────────

    #[test]
    fn parse_one_way() {
        let ast = parse_freq("proc freq data=a; tables x; run;").unwrap();
        assert_eq!(ast.data.as_ref().unwrap().name, "a");
        assert_eq!(ast.tables.len(), 1);
        assert_eq!(ast.tables[0].vars, vec!["x"]);
        assert!(!ast.tables[0].missing);
        assert!(ast.tables[0].out.is_none());
    }

    #[test]
    fn parse_multiple_specs_and_crosstab() {
        let ast = parse_freq("proc freq data=a; tables a b a*b; run;").unwrap();
        assert_eq!(ast.tables.len(), 3);
        assert_eq!(ast.tables[0].vars, vec!["a"]);
        assert_eq!(ast.tables[1].vars, vec!["b"]);
        assert_eq!(ast.tables[2].vars, vec!["a", "b"]);
    }

    #[test]
    fn parse_missing_and_out() {
        let ast = parse_freq("proc freq data=a; tables a / missing out=work.o; run;").unwrap();
        assert_eq!(ast.tables.len(), 1);
        assert!(ast.tables[0].missing);
        let out = ast.tables[0].out.as_ref().unwrap();
        assert_eq!(out.libref.as_deref(), Some("work"));
        assert_eq!(out.name, "o");
    }

    #[test]
    fn parse_out_requires_single_spec() {
        let r = parse_freq("proc freq data=a; tables a b / out=work.o; run;");
        assert!(r.is_err());
        let msg = r.err().unwrap().to_string();
        assert!(msg.contains("OUT="), "msg: {msg}");
    }

    #[test]
    fn parse_ignores_display_options() {
        let ast =
            parse_freq("proc freq data=a; tables x / nopercent norow nocol nofreq nocum; run;")
                .unwrap();
        assert_eq!(ast.tables.len(), 1);
        assert_eq!(ast.tables[0].vars, vec!["x"]);
    }

    #[test]
    fn parse_accepts_table_spelling() {
        let ast = parse_freq("proc freq data=a; table x; run;").unwrap();
        assert_eq!(ast.tables.len(), 1);
        assert_eq!(ast.tables[0].vars, vec!["x"]);
    }

    #[test]
    fn parse_multiple_tables_statements_accumulate() {
        let ast = parse_freq("proc freq data=a; tables x; tables y*z; run;").unwrap();
        assert_eq!(ast.tables.len(), 2);
        assert_eq!(ast.tables[0].vars, vec!["x"]);
        assert_eq!(ast.tables[1].vars, vec!["y", "z"]);
    }

    // ───────────────────────────── tally tests ─────────────────────────────

    #[test]
    fn tally_excludes_missing_by_default() {
        let col = vec![
            Value::Num(2.0),
            Value::Num(1.0),
            Value::Num(2.0),
            Value::missing(),
        ];
        let rows: Vec<usize> = (0..col.len()).collect();
        let (cats, nm) = tally(&col, &rows, false, None);
        assert_eq!(nm, 1.0);
        // sas_cmp order: 1 then 2.
        assert_eq!(cats.len(), 2);
        assert_eq!(cats[0].value, Value::Num(1.0));
        assert_eq!(cats[0].freq, 1.0);
        assert_eq!(cats[1].value, Value::Num(2.0));
        assert_eq!(cats[1].freq, 2.0);
    }

    #[test]
    fn tally_includes_missing_when_requested() {
        let col = vec![Value::Num(2.0), Value::missing(), Value::Num(2.0)];
        let rows: Vec<usize> = (0..col.len()).collect();
        let (cats, nm) = tally(&col, &rows, true, None);
        assert_eq!(nm, 1.0);
        // Missing sorts before numbers.
        assert_eq!(cats.len(), 2);
        assert!(cats[0].value.is_missing());
        assert_eq!(cats[0].freq, 1.0);
        assert_eq!(cats[1].value, Value::Num(2.0));
        assert_eq!(cats[1].freq, 2.0);
    }

    // ───────────────────────────── execute tests ───────────────────────────

    #[test]
    fn execute_one_way_default_excludes_missing() {
        let mut session = make_session();
        // x = 1,1,2,. -> non-missing denom = 3.
        let df = df!["x" => [Some(1.0_f64), Some(1.0), Some(2.0), None]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = FreqAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            tables: vec![tr(&["x"], false, None)],
            weight: None,
            by: Vec::new(),
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("The FREQ Procedure"), "{listing}");
        assert!(listing.contains("Frequency Missing = 1"), "{listing}");
        // 1: freq 2, percent 2/3 = 66.67; 2: freq 1, 33.33; cumulative 100.00.
        assert!(listing.contains("66.67"), "{listing}");
        assert!(listing.contains("33.33"), "{listing}");
        assert!(listing.contains("100.00"), "{listing}");
        // cumulative frequency 3 present.
        assert!(listing.contains('3'), "{listing}");
    }

    #[test]
    fn execute_one_way_missing_option_includes_it() {
        let mut session = make_session();
        let df = df!["x" => [Some(1.0_f64), Some(1.0), Some(2.0), None]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = FreqAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            tables: vec![tr(&["x"], true, None)],
            weight: None,
            by: Vec::new(),
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // With MISSING the denom is 4: 1 -> 2/4 = 50.00; missing -> 1/4 = 25.00.
        assert!(listing.contains("50.00"), "{listing}");
        assert!(listing.contains("25.00"), "{listing}");
        // No "Frequency Missing" footnote when MISSING is set.
        assert!(!listing.contains("Frequency Missing"), "{listing}");
    }

    #[test]
    fn execute_out_dataset() {
        let mut session = make_session();
        // x = a,a,b -> a:2 (66.67), b:1 (33.33).
        let df = df!["x" => ["a", "a", "b"]].unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = FreqAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            tables: vec![tr(
                &["x"],
                false,
                Some(DatasetRef { libref: Some("WORK".into()), name: "O".into() }),
            )],
            weight: None,
            by: Vec::new(),
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        assert_eq!(out.n_obs(), 2);
        let cat = read_col(&session, "O", "x");
        let count = read_col(&session, "O", "COUNT");
        let pct = read_col(&session, "O", "PERCENT");
        // sas_cmp order: a then b.
        assert_eq!(cat, vec![Value::Char("a".into()), Value::Char("b".into())]);
        assert_eq!(count, vec![Value::Num(2.0), Value::Num(1.0)]);
        if let (Value::Num(pa), Value::Num(pb)) = (&pct[0], &pct[1]) {
            assert!((pa - 200.0 / 3.0).abs() < 1e-9, "pa={pa}");
            assert!((pb - 100.0 / 3.0).abs() < 1e-9, "pb={pb}");
        } else {
            panic!("percent must be numeric: {pct:?}");
        }

        let log = session.log.into_string();
        assert!(
            log.contains("The data set WORK.O has 2 observations and 3 variables."),
            "log: {log}"
        );
    }

    #[test]
    fn execute_crosstab_counts_and_total() {
        let mut session = make_session();
        // 2x2: (a,1),(a,2),(b,1),(b,1)
        // rows a: 1->1, 2->1 ; b: 1->2, 2->0. grand=4.
        let df = df![
            "r" => ["a", "a", "b", "b"],
            "c" => [1.0_f64, 2.0, 1.0, 1.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("r"), num_meta("c")] };
        write_dataset(&mut session, "T", ds);

        let ast = FreqAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            tables: vec![tr(&["r", "c"], false, None)],
            weight: None,
            by: Vec::new(),
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("Table of r by c"), "{listing}");
        // Grand total 4 and column total for c=1 is 3.
        assert!(listing.contains("Total"), "{listing}");
        // The 4 stacked-cell legend.
        assert!(listing.contains("Row Pct"), "{listing}");
        // Grand-total percent 100.00 must appear.
        assert!(listing.contains("100.00"), "{listing}");
    }

    // ───────────────────────── chisq_sf tests ─────────────────────────

    #[test]
    fn chisq_sf_known_values() {
        // 95th percentile of chi-square(1) is 3.841 -> upper tail ~ 0.05.
        assert!((chisq_sf(3.841, 1.0) - 0.05).abs() < 1e-3, "{}", chisq_sf(3.841, 1.0));
        // At 0 the survival function is 1.
        assert!((chisq_sf(0.0, 1.0) - 1.0).abs() < 1e-12);
        // Far in the tail -> ~0.
        assert!(chisq_sf(100.0, 1.0) < 1e-3);
    }

    // ───────────────────────── display-option tests ─────────────────────────

    fn one_way_listing(opts: impl Fn(&mut TableRequest)) -> String {
        let mut session = make_session();
        let df = df!["x" => [Some(1.0_f64), Some(1.0), Some(2.0)]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);
        let mut req = tr(&["x"], false, None);
        opts(&mut req);
        let ast = FreqAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            tables: vec![req],
            weight: None,
            by: Vec::new(),
        };
        execute(&ast, &mut session).unwrap();
        session.listing.into_string()
    }

    #[test]
    fn one_way_nofreq_drops_frequency_column() {
        let l = one_way_listing(|r| r.nofreq = true);
        // The standalone "Frequency" header is gone, but "Cumulative
        // Frequency" (NOCUM not set) remains.
        let default = one_way_listing(|_| {});
        let default_freq = default.matches("Frequency").count();
        assert_eq!(l.matches("Frequency").count(), default_freq - 1, "{l}");
        assert!(l.contains("Percent"), "{l}");
        assert!(l.contains("Cumulative Frequency"), "{l}");
    }

    #[test]
    fn one_way_nopercent_drops_percent_columns() {
        let l = one_way_listing(|r| r.nopercent = true);
        assert!(!l.contains("Percent"), "{l}");
        assert!(l.contains("Frequency"), "{l}");
        assert!(l.contains("Cumulative Frequency"), "{l}");
    }

    #[test]
    fn one_way_nocum_drops_cumulative_columns() {
        let l = one_way_listing(|r| r.nocum = true);
        assert!(!l.contains("Cumulative"), "{l}");
        assert!(l.contains("Frequency"), "{l}");
        assert!(l.contains("Percent"), "{l}");
    }

    fn crosstab_listing(opts: impl Fn(&mut TableRequest)) -> String {
        let mut session = make_session();
        let df = df![
            "r" => ["a", "a", "b", "b"],
            "c" => [1.0_f64, 2.0, 1.0, 1.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("r"), num_meta("c")] };
        write_dataset(&mut session, "T", ds);
        let mut req = tr(&["r", "c"], false, None);
        opts(&mut req);
        let ast = FreqAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            tables: vec![req],
            weight: None,
            by: Vec::new(),
        };
        execute(&ast, &mut session).unwrap();
        session.listing.into_string()
    }

    #[test]
    fn crosstab_norow_drops_row_pct() {
        let l = crosstab_listing(|r| r.norow = true);
        assert!(!l.contains("Row Pct"), "{l}");
        assert!(l.contains("Col Pct"), "{l}");
    }

    #[test]
    fn crosstab_nocol_drops_col_pct() {
        let l = crosstab_listing(|r| r.nocol = true);
        assert!(!l.contains("Col Pct"), "{l}");
        assert!(l.contains("Row Pct"), "{l}");
    }

    #[test]
    fn crosstab_nofreq_keeps_others() {
        // NOFREQ drops the per-cell Frequency line; Percent/Row/Col remain.
        let l = crosstab_listing(|r| r.nofreq = true);
        assert!(l.contains("Percent"), "{l}");
        assert!(l.contains("Row Pct"), "{l}");
        assert!(l.contains("Col Pct"), "{l}");
        // The label row must still identify the row categories and Total.
        assert!(l.contains("Total"), "{l}");
    }

    // ───────────────────────── chisq block test ─────────────────────────

    #[test]
    fn crosstab_chisq_2x2_hand_computed() {
        // 2x2 table:
        //          c=1  c=2  | tot
        //   r=a :   10    20 |  30
        //   r=b :   30    40 |  70
        //   col :   40    60 | 100
        // Expected: e_a1=30*40/100=12, e_a2=18, e_b1=28, e_b2=42.
        // Pearson = (10-12)^2/12 + (20-18)^2/18 + (30-28)^2/28 + (40-42)^2/42
        //         = 4/12 + 4/18 + 4/28 + 4/42
        //         = 0.333333 + 0.222222 + 0.142857 + 0.095238 = 0.793651
        // DF = 1; p = chisq_sf(0.793651, 1) ~ 0.3730.
        let mut session = make_session();
        // Build column vectors that reproduce the table counts.
        let mut r: Vec<&str> = Vec::new();
        let mut c: Vec<f64> = Vec::new();
        for _ in 0..10 { r.push("a"); c.push(1.0); }
        for _ in 0..20 { r.push("a"); c.push(2.0); }
        for _ in 0..30 { r.push("b"); c.push(1.0); }
        for _ in 0..40 { r.push("b"); c.push(2.0); }
        let df = df!["r" => r, "c" => c].unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("r"), num_meta("c")] };
        write_dataset(&mut session, "T", ds);

        let mut req = tr(&["r", "c"], false, None);
        req.chisq = true;
        let ast = FreqAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            tables: vec![req],
            weight: None,
            by: Vec::new(),
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("Statistics for Table of r by c"), "{listing}");
        assert!(listing.contains("Chi-Square"), "{listing}");
        assert!(listing.contains("Likelihood Ratio Chi-Square"), "{listing}");
        // Pearson value formatted to 4 decimals.
        assert!(listing.contains("0.7937"), "{listing}");

        // Cross-check the numeric pieces directly.
        let pearson: f64 = 4.0 / 12.0 + 4.0 / 18.0 + 4.0 / 28.0 + 4.0 / 42.0;
        assert!((pearson - 0.793651).abs() < 1e-4, "{pearson}");
        let p = chisq_sf(pearson, 1.0);
        assert!((p - 0.3730).abs() < 1e-3, "p={p}");
    }

    // ───────────────────── M21.2 advanced statistics ─────────────────────

    /// Render the listing produced by a block fn for assertions.
    fn run_block<F: FnOnce(&mut Session)>(f: F) -> String {
        let mut session = make_session();
        f(&mut session);
        session.listing.into_string()
    }

    fn margins(freq: &[Vec<usize>]) -> (Vec<usize>, Vec<usize>, usize) {
        let nr = freq.len();
        let nc = freq[0].len();
        let row_tot: Vec<usize> = (0..nr).map(|r| freq[r].iter().sum()).collect();
        let col_tot: Vec<usize> = (0..nc).map(|c| (0..nr).map(|r| freq[r][c]).sum()).collect();
        let grand: usize = row_tot.iter().sum();
        (row_tot, col_tot, grand)
    }

    // ---- parser ----

    #[test]
    fn parse_new_stat_options() {
        let ast =
            parse_freq("proc freq data=a; tables a*b / chisq fisher agree measures trend; run;")
                .unwrap();
        let t = &ast.tables[0];
        assert!(t.chisq && t.fisher && t.agree && t.measures && t.trend);
    }

    #[test]
    fn parse_exact_and_relrisk_aliases() {
        let ast = parse_freq("proc freq data=a; tables a*b / exact relrisk; run;").unwrap();
        let t = &ast.tables[0];
        assert!(t.fisher, "exact -> fisher");
        assert!(t.measures, "relrisk -> measures");
    }

    // ---- CHISQ one-way goodness of fit ----

    #[test]
    fn chisq_one_way_equal_proportions() {
        // 4 categories, counts 10,20,30,40. N=100, exp=25 each.
        // chisq = (15²+5²+5²+15²)/25 = (225+25+25+225)/25 = 500/25 = 20.
        // DF=3, p = chisq_sf(20,3) ~ 0.00017.
        let cats = vec![
            Category { value: Value::Num(1.0), freq: 10.0 },
            Category { value: Value::Num(2.0), freq: 20.0 },
            Category { value: Value::Num(3.0), freq: 30.0 },
            Category { value: Value::Num(4.0), freq: 40.0 },
        ];
        let out = run_block(|s| chisq_one_way_block(s, &cats));
        assert!(out.contains("Chi-Square Test for Equal Proportions"), "{out}");
        assert!(out.contains("20.0000"), "{out}");
        let p = chisq_sf(20.0, 3.0);
        assert!((p - 0.00017).abs() < 1e-4, "p={p}");
    }

    #[test]
    fn chisq_one_way_uniform_is_zero() {
        let cats = vec![
            Category { value: Value::Num(1.0), freq: 25.0 },
            Category { value: Value::Num(2.0), freq: 25.0 },
            Category { value: Value::Num(3.0), freq: 25.0 },
            Category { value: Value::Num(4.0), freq: 25.0 },
        ];
        let out = run_block(|s| chisq_one_way_block(s, &cats));
        assert!(out.contains("0.0000"), "{out}");
    }

    #[test]
    fn chisq_one_way_degenerate_note() {
        let cats = vec![Category { value: Value::Num(1.0), freq: 5.0 }];
        let out = run_block(|s| chisq_one_way_block(s, &cats));
        assert!(out.contains("not computable"), "{out}");
    }

    // ---- Fisher exact ----

    #[test]
    fn fisher_2x2_symmetric_classic() {
        // [[3,1],[1,3]] : documented SAS two-sided p ~ 0.4857.
        let freq = vec![vec![3, 1], vec![1, 3]];
        let (rt, ct, g) = margins(&freq);
        let out = run_block(|s| fisher_block(s, &freq, &rt, &ct, g));
        assert!(out.contains("Fisher's Exact Test"), "{out}");
        assert!(out.contains("Two-sided Pr <= P"), "{out}");
        assert!(out.contains("0.4857"), "two-sided 0.4857 expected:\n{out}");
    }

    #[test]
    fn fisher_2x2_numeric_values() {
        // Recompute the canonical case exactly: r1=r2=c1=c2=4, n=8.
        // p(a)=C(4,a)C(4,4-a)/C(8,4). C(8,4)=70.
        // a=0:1*1/70, a=1:4*4/70, a=2:6*6/70, a=3:4*4/70, a=4:1*1/70.
        let c84 = 70.0;
        let pa = |a: u64| -> f64 {
            let lc = ln_choose(4, a) + ln_choose(4, 4 - a);
            lc.exp() / c84
        };
        // observed a=3 -> p_obs = 16/70.
        let p_obs = pa(3);
        assert!((p_obs - 16.0 / 70.0).abs() < 1e-12);
        // two-sided = sum of probs <= p_obs = a in {0,1,3,4} (a=2 is 36/70 > 16/70).
        let two = pa(0) + pa(1) + pa(3) + pa(4);
        assert!((two - (1.0 + 16.0 + 16.0 + 1.0) / 70.0).abs() < 1e-12);
        assert!((two - 0.485714).abs() < 1e-5, "two={two}");
        // right-sided P(A>=3) = (16+1)/70 = 0.242857.
        let right = pa(3) + pa(4);
        assert!((right - 17.0 / 70.0).abs() < 1e-12);
    }

    #[test]
    fn fisher_larger_than_2x2_deferred() {
        let freq = vec![vec![1, 2, 3], vec![4, 5, 6]];
        let (rt, ct, g) = margins(&freq);
        let out = run_block(|s| fisher_block(s, &freq, &rt, &ct, g));
        assert!(out.contains("larger than 2x2"), "{out}");
    }

    // ---- MEASURES (odds ratio / RR) ----

    #[test]
    fn measures_odds_ratio_exact() {
        // [[20,10],[5,25]] : OR = (20*25)/(10*5) = 500/50 = 10.
        let freq = vec![vec![20, 10], vec![5, 25]];
        let out = run_block(|s| measures_block(s, &freq));
        assert!(out.contains("Odds Ratio"), "{out}");
        assert!(out.contains("10.0000"), "OR=10 expected:\n{out}");
        // RR col1 = (20/30)/(5/30) = (0.6667)/(0.1667) = 4.
        assert!(out.contains("4.0000"), "RR col1 = 4 expected:\n{out}");
    }

    #[test]
    fn measures_odds_ratio_ci() {
        // OR=10, SE = sqrt(1/20+1/10+1/5+1/25)=sqrt(0.39)=0.62450.
        // ln10=2.302585; CI = exp(2.302585 ∓ 1.96*0.62450) = [2.9405, 34.008].
        let se = (1.0 / 20.0 + 1.0 / 10.0 + 1.0 / 5.0 + 1.0 / 25.0_f64).sqrt();
        assert!((se - 0.624500).abs() < 1e-4, "se={se}");
        let or: f64 = 10.0;
        let lo = (or.ln() - 1.96 * se).exp();
        let hi = (or.ln() + 1.96 * se).exp();
        assert!((lo - 2.9405).abs() < 1e-3, "lo={lo}");
        assert!((hi - 34.008).abs() < 1e-2, "hi={hi}");
    }

    #[test]
    fn measures_zero_cell_no_panic() {
        // b=0 -> OR undefined; must not panic and must print ".".
        let freq = vec![vec![5, 0], vec![3, 7]];
        let out = run_block(|s| measures_block(s, &freq));
        assert!(out.contains("Odds Ratio"), "{out}");
        // Odds ratio row carries "." because b=0.
        assert!(out.contains('.'), "{out}");
    }

    #[test]
    fn measures_requires_2x2() {
        let freq = vec![vec![1, 2, 3], vec![4, 5, 6]];
        let out = run_block(|s| measures_block(s, &freq));
        assert!(out.contains("require a 2x2"), "{out}");
    }

    // ---- AGREE (kappa) ----

    #[test]
    fn agree_kappa_hand_computed() {
        // Diagonal-heavy 2x2 agreement table:
        // [[20,5],[10,15]] : N=50.
        // Po = (20+15)/50 = 0.70.
        // row tot = [25,25], col tot = [30,20].
        // Pe = (25/50)(30/50) + (25/50)(20/50) = 0.5*0.6 + 0.5*0.4 = 0.3+0.2 = 0.5.
        // kappa = (0.70-0.50)/(1-0.50) = 0.20/0.50 = 0.40.
        let freq = vec![vec![20, 5], vec![10, 15]];
        let (rt, ct, g) = margins(&freq);
        let out = run_block(|s| agree_block(s, &freq, &rt, &ct, g));
        assert!(out.contains("Simple Kappa Coefficient"), "{out}");
        assert!(out.contains("0.4000"), "kappa=0.40 expected:\n{out}");
    }

    #[test]
    fn agree_perfect_agreement_kappa_one() {
        // Pure diagonal -> Po=1 -> kappa = 1.
        let freq = vec![vec![10, 0], vec![0, 10]];
        let (rt, ct, g) = margins(&freq);
        let out = run_block(|s| agree_block(s, &freq, &rt, &ct, g));
        assert!(out.contains("1.0000"), "kappa=1 expected:\n{out}");
    }

    #[test]
    fn agree_requires_square() {
        let freq = vec![vec![1, 2, 3], vec![4, 5, 6]];
        let (rt, ct, g) = margins(&freq);
        let out = run_block(|s| agree_block(s, &freq, &rt, &ct, g));
        assert!(out.contains("requires a square table"), "{out}");
    }

    #[test]
    fn agree_3x3_kappa() {
        // 3x3 with strong diagonal.
        // [[10,1,1],[1,10,1],[1,1,10]] N=36.
        // Po = 30/36 = 0.833333.
        // each row tot=12, col tot=12 -> Pe = 3*(12/36)(12/36) = 3*(1/9)=0.333333.
        // kappa = (0.833333-0.333333)/(1-0.333333) = 0.5/0.666667 = 0.75.
        let freq = vec![vec![10, 1, 1], vec![1, 10, 1], vec![1, 1, 10]];
        let (rt, ct, g) = margins(&freq);
        let out = run_block(|s| agree_block(s, &freq, &rt, &ct, g));
        assert!(out.contains("0.7500"), "kappa=0.75 expected:\n{out}");
    }

    // ---- TREND (Cochran-Armitage) ----

    #[test]
    fn trend_monotone_increasing() {
        // 2x3 table, ordinal columns 1..3, clear increasing trend in row 1.
        // row0 (cases): [5,10,20], row1 (controls): [20,10,5].
        // col tot = [25,20,25], N=70, r1=35, r2=35.
        let freq = vec![vec![5, 10, 20], vec![20, 10, 5]];
        let (rt, ct, g) = margins(&freq);
        let out = run_block(|s| trend_block(s, &freq, &rt, &ct, g));
        assert!(out.contains("Cochran-Armitage Trend Test"), "{out}");
        assert!(out.contains("Statistic (Z)"), "{out}");

        // Hand recompute: scores s=[1,2,3]. row0=[5,10,20], col tot=[25,20,25],
        // N=70, r1=35, r2=35.
        // T = Σ s_i (n_{1i} - r1*c_i/N)
        //   = 1*(5 - 35*25/70) + 2*(10 - 35*20/70) + 3*(20 - 35*25/70)
        //   = 1*(5-12.5) + 2*(10-10) + 3*(20-12.5)
        //   = -7.5 + 0 + 22.5 = 15.
        // Σ c_i s_i = 25*1+20*2+25*3 = 25+40+75 = 140.
        // Σ c_i s_i² = 25*1+20*4+25*9 = 25+80+225 = 330.
        // Var = (35*35/70)*(330 - 140²/70) = 17.5 * (330-280) = 17.5*50 = 875.
        // Z = 15/sqrt(875) = 15/29.58 = 0.5071.
        let t = 15.0_f64;
        let var: f64 = (35.0 * 35.0 / 70.0) * (330.0 - 140.0 * 140.0 / 70.0);
        let z = t / var.sqrt();
        assert!((z - 0.5071).abs() < 1e-3, "z={z}");
        assert!(out.contains("0.5071"), "Z=0.5071 expected:\n{out}");
        let p_two = (2.0 * (1.0 - probnorm(z.abs()))).min(1.0);
        assert!((p_two - 0.6121).abs() < 1e-3, "p_two={p_two}");
    }

    #[test]
    fn trend_requires_binary_dimension() {
        let freq = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        let (rt, ct, g) = margins(&freq);
        let out = run_block(|s| trend_block(s, &freq, &rt, &ct, g));
        assert!(out.contains("requires a 2xC or Rx2"), "{out}");
    }

    #[test]
    fn trend_rx2_orientation() {
        // 3x2 (rows are ordinal). Should compute (transposed roles), no panic.
        let freq = vec![vec![5, 20], vec![10, 10], vec![20, 5]];
        let (rt, ct, g) = margins(&freq);
        let out = run_block(|s| trend_block(s, &freq, &rt, &ct, g));
        assert!(out.contains("Statistic (Z)"), "{out}");
    }

    // ---- common.rs numeric helpers ----

    #[test]
    fn probnorm_known_values() {
        assert!((probnorm(0.0) - 0.5).abs() < 1e-9);
        assert!((probnorm(1.96) - 0.975).abs() < 1e-4, "{}", probnorm(1.96));
        assert!((probnorm(-1.96) - 0.025).abs() < 1e-4);
    }

    #[test]
    fn ln_choose_known_values() {
        assert!((ln_choose(8, 4).exp() - 70.0).abs() < 1e-6);
        assert!((ln_choose(5, 2).exp() - 10.0).abs() < 1e-6);
        assert!(ln_choose(3, 5) == f64::NEG_INFINITY);
    }

    // ---- end-to-end through execute() ----

    #[test]
    fn execute_fisher_measures_agree_end_to_end() {
        let mut session = make_session();
        // Build [[20,10],[5,25]] from raw columns.
        let mut r: Vec<&str> = Vec::new();
        let mut c: Vec<f64> = Vec::new();
        for _ in 0..20 { r.push("a"); c.push(1.0); }
        for _ in 0..10 { r.push("a"); c.push(2.0); }
        for _ in 0..5 { r.push("b"); c.push(1.0); }
        for _ in 0..25 { r.push("b"); c.push(2.0); }
        let df = df!["r" => r, "c" => c].unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("r"), num_meta("c")] };
        write_dataset(&mut session, "T", ds);

        let mut req = tr(&["r", "c"], false, None);
        req.fisher = true;
        req.measures = true;
        req.agree = true;
        let ast = FreqAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            tables: vec![req],
            weight: None,
            by: Vec::new(),
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("Fisher's Exact Test"), "{listing}");
        assert!(listing.contains("Estimates of the Relative Risk"), "{listing}");
        assert!(listing.contains("Simple Kappa Coefficient"), "{listing}");
        assert!(listing.contains("10.0000"), "OR=10:\n{listing}");
    }

    #[test]
    fn execute_one_way_chisq_end_to_end() {
        let mut session = make_session();
        let mut x: Vec<f64> = Vec::new();
        for _ in 0..10 { x.push(1.0); }
        for _ in 0..20 { x.push(2.0); }
        for _ in 0..30 { x.push(3.0); }
        for _ in 0..40 { x.push(4.0); }
        let df = df!["x" => x].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let mut req = tr(&["x"], false, None);
        req.chisq = true;
        let ast = FreqAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            tables: vec![req],
            weight: None,
            by: Vec::new(),
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("Chi-Square Test for Equal Proportions"), "{listing}");
        assert!(listing.contains("20.0000"), "{listing}");
    }

    // ───────────────────── M33.1 WEIGHT / BY / LIST / n-way ─────────────────────

    /// WEIGHT one-way: the cell frequency is the SUM OF WEIGHTS, not the count.
    /// x = [1, 1, 2]; w = [2, 3, 5].
    ///   cat 1 -> weight 2+3 = 5 ; cat 2 -> weight 5. denom = 10.
    ///   percent 1 -> 50.00 ; percent 2 -> 50.00. cumulative 5 then 10.
    #[test]
    fn weighted_one_way_sum_of_weights() {
        let mut session = make_session();
        let df = df![
            "x" => [1.0_f64, 1.0, 2.0],
            "w" => [2.0_f64, 3.0, 5.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("w")] };
        write_dataset(&mut session, "T", ds);

        let mut ast = fast(
            DatasetRef { libref: Some("WORK".into()), name: "T".into() },
            vec![tr(&["x"], false, None)],
        );
        ast.weight = Some("w".to_string());
        execute(&ast, &mut session).unwrap();
        let l = session.listing.into_string();
        // Frequencies 5 and 5 (sum of weights), each 50.00%, cum 5 then 10.
        assert!(l.contains("50.00"), "{l}");
        // Integer-valued weighted freqs print as integers (no decimals).
        assert!(l.contains(" 5 ") || l.contains(" 5\n") || l.contains("5  "), "{l}");
        assert!(l.contains("100.00"), "{l}");
    }

    /// WEIGHT excludes observations whose weight is missing or non-positive.
    /// x = [1, 1, 2, 2]; w = [4, ., -1, 6].
    ///   obs2 (w missing) dropped, obs3 (w=-1) dropped.
    ///   cat 1 -> 4 ; cat 2 -> 6. denom = 10. percents 40.00 / 60.00.
    #[test]
    fn weighted_excludes_missing_and_nonpositive() {
        let mut session = make_session();
        let df = df![
            "x" => [1.0_f64, 1.0, 2.0, 2.0],
            "w" => [Some(4.0_f64), None, Some(-1.0), Some(6.0)]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x"), num_meta("w")] };
        write_dataset(&mut session, "T", ds);

        let mut ast = fast(
            DatasetRef { libref: Some("WORK".into()), name: "T".into() },
            vec![tr(&["x"], false, None)],
        );
        ast.weight = Some("w".to_string());
        execute(&ast, &mut session).unwrap();
        let l = session.listing.into_string();
        assert!(l.contains("40.00"), "cat1 = 4/10 = 40.00:\n{l}");
        assert!(l.contains("60.00"), "cat2 = 6/10 = 60.00:\n{l}");
    }

    /// WEIGHT feeds CHISQ. 2x2 with weighted counts == the classic
    /// [[10,20],[30,40]] table built from unit cells with those weights.
    /// Pearson chi-square = 0.7937 (DF=1), as in `crosstab_chisq_2x2_hand_computed`.
    #[test]
    fn weighted_two_way_chisq() {
        let mut session = make_session();
        // One observation per cell, weight = the desired count.
        let df = df![
            "r" => ["a", "a", "b", "b"],
            "c" => [1.0_f64, 2.0, 1.0, 2.0],
            "w" => [10.0_f64, 20.0, 30.0, 40.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("r"), num_meta("c"), num_meta("w")],
        };
        write_dataset(&mut session, "T", ds);

        let mut req = tr(&["r", "c"], false, None);
        req.chisq = true;
        let mut ast = fast(
            DatasetRef { libref: Some("WORK".into()), name: "T".into() },
            vec![req],
        );
        ast.weight = Some("w".to_string());
        execute(&ast, &mut session).unwrap();
        let l = session.listing.into_string();
        // Weighted grand total = 100 ; Pearson = 0.7937.
        assert!(l.contains("0.7937"), "weighted Pearson 0.7937:\n{l}");
        // Weighted cell freq for (a,1) is 10 (integer-printed).
        assert!(l.contains("10"), "{l}");
    }

    /// BY splits the analysis into one section per group. class-like toy:
    /// g = [A,A,A,B,B] (sorted); x = [1,2,2,1,1].
    ///   Group A: x=1 freq 1 (33.33%), x=2 freq 2 (66.67%).
    ///   Group B: x=1 freq 2 (100.00%).
    #[test]
    fn by_groups_split_one_way() {
        let mut session = make_session();
        let df = df![
            "g" => ["A", "A", "A", "B", "B"],
            "x" => [1.0_f64, 2.0, 2.0, 1.0, 1.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let mut ast = fast(
            DatasetRef { libref: Some("WORK".into()), name: "T".into() },
            vec![tr(&["x"], false, None)],
        );
        ast.by = vec![("g".to_string(), false)];
        execute(&ast, &mut session).unwrap();
        let l = session.listing.into_string();
        assert!(l.contains("g=A"), "BY header for A:\n{l}");
        assert!(l.contains("g=B"), "BY header for B:\n{l}");
        // Group A percents 33.33 / 66.67 ; Group B 100.00.
        assert!(l.contains("33.33"), "{l}");
        assert!(l.contains("66.67"), "{l}");
        assert!(l.contains("100.00"), "{l}");
    }

    /// BY requires the input sorted by the BY var; otherwise the SAS error.
    #[test]
    fn by_unsorted_errors() {
        let mut session = make_session();
        let df = df![
            "g" => ["B", "A"],
            "x" => [1.0_f64, 1.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("g"), num_meta("x")] };
        write_dataset(&mut session, "T", ds);
        let mut ast = fast(
            DatasetRef { libref: Some("WORK".into()), name: "T".into() },
            vec![tr(&["x"], false, None)],
        );
        ast.by = vec![("g".to_string(), false)];
        let err = execute(&ast, &mut session).unwrap_err();
        assert!(
            err.to_string().contains("not sorted in ascending sequence"),
            "{err}"
        );
    }

    /// /LIST layout: one row per non-empty cell with Frequency / Percent /
    /// Cumulative columns; no grid Row/Col Pct.
    /// Cells (a,1)=1, (a,2)=1, (b,1)=2 ; grand=4.
    ///   (a,1): 1 / 25.00 / cum 1 / 25.00
    ///   (a,2): 1 / 25.00 / cum 2 / 50.00
    ///   (b,1): 2 / 50.00 / cum 4 / 100.00
    #[test]
    fn list_layout_rows() {
        let mut session = make_session();
        let df = df![
            "r" => ["a", "a", "b", "b"],
            "c" => [1.0_f64, 2.0, 1.0, 1.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("r"), num_meta("c")] };
        write_dataset(&mut session, "T", ds);

        let mut req = tr(&["r", "c"], false, None);
        req.list = true;
        let ast = fast(
            DatasetRef { libref: Some("WORK".into()), name: "T".into() },
            vec![req],
        );
        execute(&ast, &mut session).unwrap();
        let l = session.listing.into_string();
        // LIST: header columns, no "Row Pct"/"Col Pct".
        assert!(l.contains("Cumulative Frequency"), "{l}");
        assert!(!l.contains("Row Pct"), "LIST suppresses Row Pct:\n{l}");
        assert!(!l.contains("Col Pct"), "LIST suppresses Col Pct:\n{l}");
        // Cumulative percent reaches 100.00.
        assert!(l.contains("100.00"), "{l}");
        assert!(l.contains("50.00"), "{l}");
    }

    /// n-way (3-way) stratified rendering: one two-way table per leading value.
    /// s = [A,A,B,B]; r = [x,x,y,y]; c = [1,2,1,2]. Each stratum has 2 cells.
    #[test]
    fn n_way_stratified() {
        let mut session = make_session();
        let df = df![
            "s" => ["A", "A", "B", "B"],
            "r" => ["x", "x", "y", "y"],
            "c" => [1.0_f64, 2.0, 1.0, 2.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("s"), char_meta("r"), num_meta("c")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = fast(
            DatasetRef { libref: Some("WORK".into()), name: "T".into() },
            vec![tr(&["s", "r", "c"], false, None)],
        );
        execute(&ast, &mut session).unwrap();
        let l = session.listing.into_string();
        assert!(l.contains("Controlling for s=A"), "stratum A header:\n{l}");
        assert!(l.contains("Controlling for s=B"), "stratum B header:\n{l}");
        assert!(l.contains("Table of r by c"), "{l}");
    }

    #[test]
    fn parse_weight_by_list() {
        let ast = parse_freq(
            "proc freq data=a; weight wt; by g; tables x*y / list; run;",
        )
        .unwrap();
        assert_eq!(ast.weight.as_deref(), Some("wt"));
        assert_eq!(ast.by, vec![("g".to_string(), false)]);
        assert!(ast.tables[0].list);
        assert_eq!(ast.tables[0].vars, vec!["x", "y"]);
    }

    #[test]
    fn parse_three_way_spec() {
        let ast = parse_freq("proc freq data=a; tables a*b*c; run;").unwrap();
        assert_eq!(ast.tables[0].vars, vec!["a", "b", "c"]);
    }
}
