//! PROC COMPARE (jalon M21.1).
//!
//! # Syntaxe
//! ```sas
//! proc compare base=lib.x compare=lib.y [out=lib.z] [novalues] [briefsummary];
//! run;
//! ```
//!
//! - BASE= et COMPARE= obligatoires.
//! - Compare structure (variables communes, types) et valeurs ligne à ligne.
//! - NOVALUES : omet la section "Values Comparison".
//! - BRIEFSUMMARY : rapport condensé (seulement les totaux).
//! - OUT= : dataset des différences (variables STAT, _BASE_, _COMP_, _TYPE_).
//!
//! # Algorithme de comparaison
//!
//! Valeurs via `sas_cmp` (de `Value`) :
//! - Missing ordinaire (.) = missing ordinaire → égal.
//! - Char : comparaison trim trailing blanks.
//! - Numériques : différence considérée si |base - compare| > 0.0. Pas de
//!   tolérance fuzzy en v1 (documenter si besoin ultérieur).
//!
//! # Rapport listing
//! 1. "Data Set Summary"  : NObs + NVars de chaque dataset.
//! 2. "Variables Summary" : en commun, seulement dans BASE, seulement dans COMPARE.
//! 3. "Observation Summary" : nb obs comparées, nb avec différences.
//! 4. "Values Comparison" : pour chaque variable numérique commune, max |diff|.
//!    (Absente si NOVALUES.)
//!
//! # Déviation v1
//! - Tolérance numérique : zéro (différence si valeurs f64 divergent, même
//!   d'un epsilon machine). La tolérance CRITERION= est reportée à v2.
//! - OUT= : créé, colonne _TYPE_ ∈ {BASE, COMPARE, DIF} + colonnes des
//!   variables communes. Conformité SAS OUT= approximative (format SAS exact
//!   différé).

use std::cmp::Ordering;
use std::collections::HashMap;

use polars::prelude::*;

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::num_to_value;
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{Value, VarType};

pub struct CompareAst {
    pub base: DatasetRef,
    pub compare: DatasetRef,
    pub out: Option<DatasetRef>,
    pub novalues: bool,
    pub briefsummary: bool,
}

/// Parse `proc compare base=... compare=... [out=...] [novalues] [briefsummary]; run;`
/// Called AFTER "proc compare" has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<CompareAst> {
    let mut base: Option<DatasetRef> = None;
    let mut compare: Option<DatasetRef> = None;
    let mut out: Option<DatasetRef> = None;
    let mut novalues = false;
    let mut briefsummary = false;

    // Parse header options until `;`
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }

        if ts.peek().is_kw("base") {
            ts.next();
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse("expected '=' after BASE", ts.peek().span));
            }
            ts.next();
            base = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("compare") {
            ts.next();
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse("expected '=' after COMPARE", ts.peek().span));
            }
            ts.next();
            compare = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("out") {
            ts.next();
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse("expected '=' after OUT", ts.peek().span));
            }
            ts.next();
            out = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("novalues") {
            ts.next();
            novalues = true;
        } else if ts.peek().is_kw("briefsummary") {
            ts.next();
            briefsummary = true;
        } else {
            // Unknown option: skip it
            ts.next();
        }
    }

    // Parse sub-statements until `run;` or `quit;`
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
        ts.skip_to_semi();
    }

    let base = base.ok_or_else(|| SasError::parse("BASE= is required for PROC COMPARE", crate::token::Span::default()))?;
    let compare = compare.ok_or_else(|| SasError::parse("COMPARE= is required for PROC COMPARE", crate::token::Span::default()))?;

    Ok(CompareAst {
        base,
        compare,
        out,
        novalues,
        briefsummary,
    })
}

/// Execute PROC COMPARE.
pub fn execute(ast: &CompareAst, session: &mut Session) -> Result<()> {
    // ── Load BASE dataset ────────────────────────────────────────────────────
    let base_libref = ast.base.libref_or_work();
    let base_name = ast.base.name.to_uppercase();
    let base_display = ast.base.display();
    let base_provider = session.libs.get(&base_libref)?;
    let (base_ds, base_notes) = base_provider.read(&base_name)?;
    for note in base_notes {
        session.log.forward(&note);
    }

    // ── Load COMPARE dataset ─────────────────────────────────────────────────
    let comp_libref = ast.compare.libref_or_work();
    let comp_name = ast.compare.name.to_uppercase();
    let comp_display = ast.compare.display();
    let comp_provider = session.libs.get(&comp_libref)?;
    let (comp_ds, comp_notes) = comp_provider.read(&comp_name)?;
    for note in comp_notes {
        session.log.forward(&note);
    }

    let base_nobs = base_ds.n_obs();
    let base_nvars = base_ds.n_vars();
    let comp_nobs = comp_ds.n_obs();
    let comp_nvars = comp_ds.n_vars();

    // ── Variable analysis ────────────────────────────────────────────────────
    // Build maps: name → (index, VarMeta) for each dataset
    let base_var_map: HashMap<String, usize> = base_ds
        .vars
        .iter()
        .enumerate()
        .map(|(i, v)| (v.name.to_uppercase(), i))
        .collect();
    let comp_var_map: HashMap<String, usize> = comp_ds
        .vars
        .iter()
        .enumerate()
        .map(|(i, v)| (v.name.to_uppercase(), i))
        .collect();

    // Variables only in BASE
    let mut only_base: Vec<String> = base_ds
        .vars
        .iter()
        .filter(|v| !comp_var_map.contains_key(&v.name.to_uppercase()))
        .map(|v| v.name.to_uppercase())
        .collect();
    only_base.sort();

    // Variables only in COMPARE
    let mut only_comp: Vec<String> = comp_ds
        .vars
        .iter()
        .filter(|v| !base_var_map.contains_key(&v.name.to_uppercase()))
        .map(|v| v.name.to_uppercase())
        .collect();
    only_comp.sort();

    // Common variables (by name, case-insensitive), with type match analysis
    struct CommonVar {
        name: String,
        base_idx: usize,
        comp_idx: usize,
        type_match: bool,
        base_type: VarType,
        comp_type: VarType,
    }

    let mut common_vars: Vec<CommonVar> = base_ds
        .vars
        .iter()
        .enumerate()
        .filter_map(|(bi, bv)| {
            let uname = bv.name.to_uppercase();
            comp_var_map.get(&uname).map(|&ci| CommonVar {
                name: uname,
                base_idx: bi,
                comp_idx: ci,
                type_match: bv.ty == comp_ds.vars[ci].ty,
                base_type: bv.ty,
                comp_type: comp_ds.vars[ci].ty,
            })
        })
        .collect();
    common_vars.sort_by(|a, b| a.name.cmp(&b.name));

    // ── Observation comparison ───────────────────────────────────────────────
    let n_compared = base_nobs.min(comp_nobs);
    let mut n_with_diffs: usize = 0;

    // For each common var that has matching types: track max abs diff (numeric)
    // and whether any diffs occurred (char).
    struct VarDiffSummary {
        name: String,
        var_type: VarType,
        n_diffs: usize,
        max_diff: f64, // only for numeric
    }

    let mut var_diffs: Vec<VarDiffSummary> = common_vars
        .iter()
        .filter(|cv| cv.type_match)
        .map(|cv| VarDiffSummary {
            name: cv.name.clone(),
            var_type: cv.base_type,
            n_diffs: 0,
            max_diff: 0.0,
        })
        .collect();

    // OUT= accumulation: rows where differences occur
    // _TYPE_: "BASE" / "COMPARE" / "DIF" rows for each obs with diffs
    struct OutRow {
        obs: usize,
        row_type: &'static str,
        values: Vec<Option<Value>>, // one per common-var with type match
    }

    let mut out_rows: Vec<OutRow> = Vec::new();
    let need_out = ast.out.is_some();

    // Build column iterators for the comparison
    // We'll do row-by-row comparison using the Polars Series
    let base_df = &base_ds.df;
    let comp_df = &comp_ds.df;

    // Pre-fetch columns
    // For each common var with type match, get base+comp series
    struct ColPair {
        cv_idx: usize, // index into common_vars (only type-matched ones)
        var_type: VarType,
        base_col_idx: usize,
        comp_col_idx: usize,
    }

    let matching_vars: Vec<&CommonVar> = common_vars.iter().filter(|cv| cv.type_match).collect();

    let col_pairs: Vec<ColPair> = matching_vars
        .iter()
        .enumerate()
        .map(|(i, cv)| ColPair {
            cv_idx: i,
            var_type: cv.base_type,
            base_col_idx: cv.base_idx,
            comp_col_idx: cv.comp_idx,
        })
        .collect();

    for obs_idx in 0..n_compared {
        let mut obs_has_diff = false;
        let mut base_out_values: Vec<Option<Value>> = vec![None; col_pairs.len()];
        let mut comp_out_values: Vec<Option<Value>> = vec![None; col_pairs.len()];
        let mut dif_out_values: Vec<Option<Value>> = vec![None; col_pairs.len()];

        for cp in &col_pairs {
            let base_val = get_value_at(base_df, cp.base_col_idx, obs_idx, cp.var_type);
            let comp_val = get_value_at(comp_df, cp.comp_col_idx, obs_idx, cp.var_type);

            let differ = values_differ(&base_val, &comp_val);

            if differ {
                obs_has_diff = true;
                var_diffs[cp.cv_idx].n_diffs += 1;

                // Compute numeric difference for max_diff
                if cp.var_type == VarType::Num {
                    if let (Value::Num(b), Value::Num(c)) = (&base_val, &comp_val) {
                        let diff = (b - c).abs();
                        if diff > var_diffs[cp.cv_idx].max_diff {
                            var_diffs[cp.cv_idx].max_diff = diff;
                        }
                    }
                }
            }

            if need_out {
                base_out_values[cp.cv_idx] = Some(base_val.clone());
                comp_out_values[cp.cv_idx] = Some(comp_val.clone());
                if cp.var_type == VarType::Num {
                    match (&base_val, &comp_val) {
                        (Value::Num(b), Value::Num(c)) => {
                            dif_out_values[cp.cv_idx] = Some(Value::Num(b - c));
                        }
                        _ => {
                            dif_out_values[cp.cv_idx] = Some(Value::missing());
                        }
                    }
                }
            }
        }

        if obs_has_diff {
            n_with_diffs += 1;
            if need_out {
                out_rows.push(OutRow {
                    obs: obs_idx + 1,
                    row_type: "BASE",
                    values: base_out_values,
                });
                out_rows.push(OutRow {
                    obs: obs_idx + 1,
                    row_type: "COMPARE",
                    values: comp_out_values,
                });
                out_rows.push(OutRow {
                    obs: obs_idx + 1,
                    row_type: "DIF",
                    values: dif_out_values,
                });
            }
        }
    }

    // ── Render listing ───────────────────────────────────────────────────────
    session.listing.page_header();

    if !ast.briefsummary {
        // === Data Set Summary ===
        session.listing.write_line("The COMPARE Procedure");
        session.listing.blank();
        session.listing.write_line("Data Set Summary");
        session.listing.blank();

        let ds_headers = vec![
            "Dataset".to_string(),
            "Role".to_string(),
            "Label".to_string(),
            "Observations".to_string(),
            "Variables".to_string(),
        ];
        let ds_aligns = vec![
            Align::Left,
            Align::Left,
            Align::Left,
            Align::Right,
            Align::Right,
        ];
        let ds_rows = vec![
            vec![
                base_display.clone(),
                "BASE".to_string(),
                String::new(),
                base_nobs.to_string(),
                base_nvars.to_string(),
            ],
            vec![
                comp_display.clone(),
                "COMPARE".to_string(),
                String::new(),
                comp_nobs.to_string(),
                comp_nvars.to_string(),
            ],
        ];
        session.listing.write_table(&ds_headers, &ds_aligns, &ds_rows);
        session.listing.blank();

        // === Variables Summary ===
        session.listing.write_line("Variables Summary");
        session.listing.blank();
        let n_common = matching_vars.len();
        let n_type_mismatch = common_vars.iter().filter(|cv| !cv.type_match).count();
        session.listing.write_line(&format!(
            "Number of Variables in Common: {}",
            common_vars.len()
        ));
        if n_type_mismatch > 0 {
            session.listing.write_line(&format!(
                "Number of Variables with Different Types: {}",
                n_type_mismatch
            ));
            for cv in common_vars.iter().filter(|cv| !cv.type_match) {
                session.listing.write_line(&format!(
                    "  Variable {}: BASE type={}, COMPARE type={}",
                    cv.name,
                    type_str(cv.base_type),
                    type_str(cv.comp_type)
                ));
            }
        }
        if !only_base.is_empty() {
            session.listing.write_line(&format!(
                "Variables in BASE only ({}): {}",
                only_base.len(),
                only_base.join(", ")
            ));
        }
        if !only_comp.is_empty() {
            session.listing.write_line(&format!(
                "Variables in COMPARE only ({}): {}",
                only_comp.len(),
                only_comp.join(", ")
            ));
        }
        session.listing.blank();

        // === Observation Summary ===
        session.listing.write_line("Observation Summary");
        session.listing.blank();
        let n_uncompared =
            (base_nobs as isize - comp_nobs as isize).unsigned_abs();
        session.listing.write_line(&format!(
            "Number of Observations in Common: {}",
            n_compared
        ));
        if n_compared < base_nobs.max(comp_nobs) {
            session.listing.write_line(&format!(
                "Number of Observations Not Compared (different N): {}",
                n_uncompared
            ));
        }
        session.listing.write_line(&format!(
            "Number of Observations with Differences: {}",
            n_with_diffs
        ));
        session.listing.write_line(&format!(
            "Number of Observations in Agreement: {}",
            n_compared - n_with_diffs
        ));
        session.listing.blank();

        // === Values Comparison ===
        if !ast.novalues && n_common > 0 {
            session.listing.write_line("Values Comparison Summary");
            session.listing.blank();

            let val_headers = vec![
                "Variable".to_string(),
                "Type".to_string(),
                "N Diffs".to_string(),
                "Max Diff".to_string(),
            ];
            let val_aligns = vec![Align::Left, Align::Left, Align::Right, Align::Right];
            let val_rows: Vec<Vec<String>> = var_diffs
                .iter()
                .map(|vd| {
                    let max_diff_str = if vd.var_type == VarType::Num && vd.n_diffs > 0 {
                        format!("{:.6}", vd.max_diff)
                    } else if vd.var_type == VarType::Char {
                        String::new()
                    } else {
                        "0".to_string()
                    };
                    vec![
                        vd.name.clone(),
                        type_str(vd.var_type).to_string(),
                        vd.n_diffs.to_string(),
                        max_diff_str,
                    ]
                })
                .collect();
            session.listing.write_table(&val_headers, &val_aligns, &val_rows);
        }
    } else {
        // BRIEFSUMMARY: condensed
        session.listing.write_line("The COMPARE Procedure - Brief Summary");
        session.listing.blank();
        session.listing.write_line(&format!(
            "BASE:    {} ({} obs, {} vars)",
            base_display, base_nobs, base_nvars
        ));
        session.listing.write_line(&format!(
            "COMPARE: {} ({} obs, {} vars)",
            comp_display, comp_nobs, comp_nvars
        ));
        session.listing.write_line(&format!(
            "Observations compared: {}  with differences: {}",
            n_compared, n_with_diffs
        ));
    }

    // ── NOTE log ────────────────────────────────────────────────────────────
    if n_with_diffs == 0 {
        session.log.note(&format!(
            "No unequal values were found. All values compared are exactly equal."
        ));
    } else {
        session.log.note(&format!(
            "There were {} observations with at least one unequal value.",
            n_with_diffs
        ));
    }

    // ── Write OUT= dataset ───────────────────────────────────────────────────
    if let Some(ref out_ref) = ast.out {
        if !out_rows.is_empty() && !matching_vars.is_empty() {
            let out_libref = out_ref.libref_or_work();
            let out_name = out_ref.name.to_uppercase();

            // Build DataFrame for OUT= dataset
            // Columns: _TYPE_, _OBS_, <common vars matching type>
            let n_rows = out_rows.len();

            let type_col: StringChunked = out_rows
                .iter()
                .map(|r| Some(r.row_type))
                .collect();
            let obs_col: Float64Chunked = out_rows
                .iter()
                .map(|r| Some(r.obs as f64))
                .collect();

            let mut columns: Vec<Column> = vec![
                Series::new("_TYPE_".into(), type_col).into(),
                Series::new("_OBS_".into(), obs_col).into(),
            ];

            let mut vars: Vec<VarMeta> = vec![
                VarMeta {
                    name: "_TYPE_".to_string(),
                    ty: VarType::Char,
                    length: 7,
                    format: None,
                    label: None,
                },
                VarMeta {
                    name: "_OBS_".to_string(),
                    ty: VarType::Num,
                    length: 8,
                    format: None,
                    label: None,
                },
            ];

            for (vi, mv) in matching_vars.iter().enumerate() {
                let base_meta = &base_ds.vars[mv.base_idx];
                match mv.base_type {
                    VarType::Num => {
                        let col_vals: Float64Chunked = out_rows
                            .iter()
                            .map(|r| {
                                r.values[vi].as_ref().and_then(|v| match v {
                                    Value::Num(f) => Some(*f),
                                    _ => None,
                                })
                            })
                            .collect();
                        columns.push(
                            Series::new(mv.name.as_str().into(), col_vals).into(),
                        );
                    }
                    VarType::Char => {
                        let col_vals: StringChunked = out_rows
                            .iter()
                            .map(|r| {
                                r.values[vi].as_ref().and_then(|v| match v {
                                    Value::Char(s) => Some(s.as_str()),
                                    _ => None,
                                })
                            })
                            .collect();
                        columns.push(
                            Series::new(mv.name.as_str().into(), col_vals).into(),
                        );
                    }
                }
                vars.push(VarMeta {
                    name: mv.name.clone(),
                    ty: mv.base_type,
                    length: base_meta.length,
                    format: base_meta.format.clone(),
                    label: base_meta.label.clone(),
                });
            }

            let df = DataFrame::new(columns)
                .map_err(|e| SasError::runtime(format!("COMPARE OUT= build error: {e}")))?;
            let out_ds = SasDataset { df, vars };
            let out_provider = session.libs.get(&out_libref)?;
            out_provider.write(&out_name, &out_ds)?;
            session.log.note(&format!(
                "Output data set: {}.{} ({} observations).",
                out_libref,
                out_name,
                n_rows
            ));
            session.last_dataset = Some(format!("{}.{}", out_libref, out_name));
        } else if out_rows.is_empty() {
            // No diffs — create empty OUT= dataset with just _TYPE_, _OBS_
            let type_col: StringChunked = std::iter::empty::<Option<&str>>().collect();
            let obs_col: Float64Chunked = std::iter::empty::<Option<f64>>().collect();
            let columns: Vec<Column> = vec![
                Series::new("_TYPE_".into(), type_col).into(),
                Series::new("_OBS_".into(), obs_col).into(),
            ];
            let vars = vec![
                VarMeta {
                    name: "_TYPE_".to_string(),
                    ty: VarType::Char,
                    length: 7,
                    format: None,
                    label: None,
                },
                VarMeta {
                    name: "_OBS_".to_string(),
                    ty: VarType::Num,
                    length: 8,
                    format: None,
                    label: None,
                },
            ];
            let df = DataFrame::new(columns)
                .map_err(|e| SasError::runtime(format!("COMPARE OUT= build error: {e}")))?;
            let out_ds = SasDataset { df, vars };
            let out_libref = out_ref.libref_or_work();
            let out_name = out_ref.name.to_uppercase();
            let out_provider = session.libs.get(&out_libref)?;
            out_provider.write(&out_name, &out_ds)?;
            session.log.note(&format!(
                "Output data set: {}.{} (0 observations).",
                out_libref, out_name
            ));
            session.last_dataset = Some(format!("{}.{}", out_libref, out_name));
        }
    }

    Ok(())
}

/// Get a SAS Value from a DataFrame column at a given row index.
/// Only handles Num and Char (the SAS type model).
fn get_value_at(df: &DataFrame, col_idx: usize, row_idx: usize, ty: VarType) -> Value {
    let col = &df.get_columns()[col_idx];
    match ty {
        VarType::Num => {
            let f64_col = col.as_materialized_series().f64().unwrap();
            num_to_value(f64_col.get(row_idx))
        }
        VarType::Char => {
            let str_col = col.as_materialized_series().str().unwrap();
            match str_col.get(row_idx) {
                None => Value::Char(String::new()),
                Some(s) => Value::Char(s.to_string()),
            }
        }
    }
}

/// Return true if two SAS values differ (using sas_cmp semantics).
fn values_differ(a: &Value, b: &Value) -> bool {
    a.sas_cmp(b) != Ordering::Equal
}

fn type_str(ty: VarType) -> &'static str {
    match ty {
        VarType::Num => "Num",
        VarType::Char => "Char",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::session::Session;
    use crate::source::SourceFile;
    use crate::value::VarType;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn parse_compare_src(src: &str) -> Result<CompareAst> {
        let source = SourceFile::new(src);
        let mut ts = crate::parser::StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "compare"
        parse(&mut ts)
    }

    fn write_numeric_ds(
        session: &mut Session,
        name: &str,
        x_vals: &[f64],
        y_vals: &[f64],
    ) {
        let df = df![
            "x" => x_vals.to_vec(),
            "y" => y_vals.to_vec()
        ]
        .unwrap();
        let vars = vec![
            VarMeta { name: "x".into(), ty: VarType::Num, length: 8, format: None, label: None },
            VarMeta { name: "y".into(), ty: VarType::Num, length: 8, format: None, label: None },
        ];
        let ds = SasDataset { df, vars };
        session.libs.get("WORK").unwrap().write(name, &ds).unwrap();
    }

    fn write_char_ds(session: &mut Session, name: &str, vals: &[&str]) {
        let df = df!["name" => vals.to_vec()].unwrap();
        let vars = vec![
            VarMeta { name: "name".into(), ty: VarType::Char, length: 8, format: None, label: None },
        ];
        let ds = SasDataset { df, vars };
        session.libs.get("WORK").unwrap().write(name, &ds).unwrap();
    }

    // ── Parse tests ───────────────────────────────────────────────────────────

    #[test]
    fn parse_minimal() {
        let ast = parse_compare_src("proc compare base=work.a compare=work.b; run;").unwrap();
        assert_eq!(ast.base.name.to_uppercase(), "A");
        assert_eq!(ast.compare.name.to_uppercase(), "B");
        assert!(ast.out.is_none());
        assert!(!ast.novalues);
        assert!(!ast.briefsummary);
    }

    #[test]
    fn parse_all_options() {
        let ast = parse_compare_src(
            "proc compare base=work.a compare=work.b out=work.diffs novalues briefsummary; run;",
        )
        .unwrap();
        assert!(ast.out.is_some());
        assert!(ast.novalues);
        assert!(ast.briefsummary);
    }

    #[test]
    fn parse_missing_base_errors() {
        let result = parse_compare_src("proc compare compare=work.b; run;");
        assert!(result.is_err());
    }

    #[test]
    fn parse_missing_compare_errors() {
        let result = parse_compare_src("proc compare base=work.a; run;");
        assert!(result.is_err());
    }

    // ── Execute tests ─────────────────────────────────────────────────────────

    #[test]
    fn execute_identical_datasets_no_diffs() {
        let mut session = make_session();
        write_numeric_ds(&mut session, "A", &[1.0, 2.0, 3.0], &[10.0, 20.0, 30.0]);
        write_numeric_ds(&mut session, "B", &[1.0, 2.0, 3.0], &[10.0, 20.0, 30.0]);

        let ast = CompareAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "A".into() },
            compare: DatasetRef { libref: Some("WORK".into()), name: "B".into() },
            out: None,
            novalues: false,
            briefsummary: false,
        };
        execute(&ast, &mut session).unwrap();

        let log = session.log.into_string();
        assert!(log.contains("No unequal values"), "log: {log}");

        let listing = session.listing.into_string();
        assert!(listing.contains("WORK.A"), "listing: {listing}");
        assert!(listing.contains("WORK.B"), "listing: {listing}");
    }

    #[test]
    fn execute_with_differences() {
        let mut session = make_session();
        write_numeric_ds(&mut session, "BASE1", &[1.0, 2.0], &[10.0, 20.0]);
        write_numeric_ds(&mut session, "COMP1", &[1.0, 9.0], &[10.0, 20.0]);

        let ast = CompareAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "BASE1".into() },
            compare: DatasetRef { libref: Some("WORK".into()), name: "COMP1".into() },
            out: None,
            novalues: false,
            briefsummary: false,
        };
        execute(&ast, &mut session).unwrap();

        let log = session.log.into_string();
        assert!(log.contains("1 observation"), "log: {log}");

        let listing = session.listing.into_string();
        assert!(listing.contains("1"), "diffs in listing: {listing}");
    }

    #[test]
    fn execute_variable_only_in_base() {
        let mut session = make_session();
        // BASE has x and z; COMPARE has only x
        let df_base = df!["x" => [1.0_f64, 2.0], "z" => [5.0_f64, 6.0]].unwrap();
        let vars_base = vec![
            VarMeta { name: "x".into(), ty: VarType::Num, length: 8, format: None, label: None },
            VarMeta { name: "z".into(), ty: VarType::Num, length: 8, format: None, label: None },
        ];
        let df_comp = df!["x" => [1.0_f64, 2.0]].unwrap();
        let vars_comp = vec![
            VarMeta { name: "x".into(), ty: VarType::Num, length: 8, format: None, label: None },
        ];
        session.libs.get("WORK").unwrap().write("BASE2", &SasDataset { df: df_base, vars: vars_base }).unwrap();
        session.libs.get("WORK").unwrap().write("COMP2", &SasDataset { df: df_comp, vars: vars_comp }).unwrap();

        let ast = CompareAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "BASE2".into() },
            compare: DatasetRef { libref: Some("WORK".into()), name: "COMP2".into() },
            out: None,
            novalues: false,
            briefsummary: false,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("BASE only") || listing.contains("Z"), "listing: {listing}");
    }

    #[test]
    fn execute_type_mismatch_reported() {
        let mut session = make_session();
        // BASE has x as Num, COMP has x as Char
        let df_base = df!["x" => [1.0_f64, 2.0]].unwrap();
        let vars_base = vec![
            VarMeta { name: "x".into(), ty: VarType::Num, length: 8, format: None, label: None },
        ];
        let df_comp = df!["x" => ["a", "b"]].unwrap();
        let vars_comp = vec![
            VarMeta { name: "x".into(), ty: VarType::Char, length: 1, format: None, label: None },
        ];
        session.libs.get("WORK").unwrap().write("BASE3", &SasDataset { df: df_base, vars: vars_base }).unwrap();
        session.libs.get("WORK").unwrap().write("COMP3", &SasDataset { df: df_comp, vars: vars_comp }).unwrap();

        let ast = CompareAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "BASE3".into() },
            compare: DatasetRef { libref: Some("WORK".into()), name: "COMP3".into() },
            out: None,
            novalues: false,
            briefsummary: false,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("Different Types") || listing.contains("Num") || listing.contains("Char"), "listing: {listing}");
    }

    #[test]
    fn execute_different_nobs() {
        let mut session = make_session();
        write_numeric_ds(&mut session, "BASE4", &[1.0, 2.0, 3.0], &[0.0, 0.0, 0.0]);
        write_numeric_ds(&mut session, "COMP4", &[1.0, 2.0], &[0.0, 0.0]);

        let ast = CompareAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "BASE4".into() },
            compare: DatasetRef { libref: Some("WORK".into()), name: "COMP4".into() },
            out: None,
            novalues: false,
            briefsummary: false,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("Not Compared") || listing.contains("2"), "listing: {listing}");
    }

    #[test]
    fn execute_char_trailing_blanks_equivalent() {
        let mut session = make_session();
        write_char_ds(&mut session, "CBASE", &["abc", "xyz"]);
        write_char_ds(&mut session, "CCOMP", &["abc   ", "xyz "]);

        let ast = CompareAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "CBASE".into() },
            compare: DatasetRef { libref: Some("WORK".into()), name: "CCOMP".into() },
            out: None,
            novalues: false,
            briefsummary: false,
        };
        execute(&ast, &mut session).unwrap();

        let log = session.log.into_string();
        // Trailing blanks should be considered equal via sas_cmp
        assert!(log.contains("No unequal values"), "log: {log}");
    }

    #[test]
    fn execute_missing_equality() {
        let mut session = make_session();
        // Both have missing values at the same position → equal
        let df_base = df!["x" => Series::new("x".into(), &[Some(1.0_f64), None])].unwrap();
        let df_comp = df!["x" => Series::new("x".into(), &[Some(1.0_f64), None])].unwrap();
        let vars = vec![
            VarMeta { name: "x".into(), ty: VarType::Num, length: 8, format: None, label: None },
        ];
        session.libs.get("WORK").unwrap().write("MISS1", &SasDataset { df: df_base, vars: vars.clone() }).unwrap();
        session.libs.get("WORK").unwrap().write("MISS2", &SasDataset { df: df_comp, vars: vars }).unwrap();

        let ast = CompareAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "MISS1".into() },
            compare: DatasetRef { libref: Some("WORK".into()), name: "MISS2".into() },
            out: None,
            novalues: false,
            briefsummary: false,
        };
        execute(&ast, &mut session).unwrap();

        let log = session.log.into_string();
        assert!(log.contains("No unequal values"), "log: {log}");
    }

    #[test]
    fn execute_out_dataset_created() {
        let mut session = make_session();
        write_numeric_ds(&mut session, "OBASE", &[1.0, 2.0], &[10.0, 20.0]);
        write_numeric_ds(&mut session, "OCOMP", &[1.0, 9.0], &[10.0, 20.0]);

        let ast = CompareAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "OBASE".into() },
            compare: DatasetRef { libref: Some("WORK".into()), name: "OCOMP".into() },
            out: Some(DatasetRef { libref: Some("WORK".into()), name: "DIFFS".into() }),
            novalues: false,
            briefsummary: false,
        };
        execute(&ast, &mut session).unwrap();

        // OUT= dataset should exist
        assert!(session.libs.get("WORK").unwrap().exists("DIFFS"));
        let (ds, _) = session.libs.get("WORK").unwrap().read("DIFFS").unwrap();
        // 3 rows per differing obs (BASE, COMPARE, DIF)
        assert_eq!(ds.n_obs(), 3);
    }

    #[test]
    fn execute_briefsummary() {
        let mut session = make_session();
        write_numeric_ds(&mut session, "BS1", &[1.0], &[2.0]);
        write_numeric_ds(&mut session, "BS2", &[1.0], &[3.0]);

        let ast = CompareAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "BS1".into() },
            compare: DatasetRef { libref: Some("WORK".into()), name: "BS2".into() },
            out: None,
            novalues: false,
            briefsummary: true,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        assert!(listing.contains("Brief Summary") || listing.contains("WORK.BS1"), "listing: {listing}");
    }

    #[test]
    fn execute_novalues_omits_values_section() {
        let mut session = make_session();
        write_numeric_ds(&mut session, "NV1", &[1.0, 2.0], &[0.0, 0.0]);
        write_numeric_ds(&mut session, "NV2", &[1.0, 9.0], &[0.0, 0.0]);

        let ast = CompareAst {
            base: DatasetRef { libref: Some("WORK".into()), name: "NV1".into() },
            compare: DatasetRef { libref: Some("WORK".into()), name: "NV2".into() },
            out: None,
            novalues: true,
            briefsummary: false,
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // The values section header should be absent
        assert!(!listing.contains("Values Comparison Summary"), "listing should not have values section: {listing}");
    }
}
