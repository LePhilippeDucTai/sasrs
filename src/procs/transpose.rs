//! PROC TRANSPOSE (jalon M7).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc transpose data=a out=b [prefix=P] [name=_name_] ; [by v...;]
//! [id v;] [var v...;] run ;`
//!
//! NE PAS utiliser le pivot Polars : les règles de nommage SAS sont
//! spécifiques — implémenter par itération de groupes BY :
//! - VAR absent : toutes les numériques hors BY/ID.
//! - Sortie : une ligne par variable VAR (par groupe BY) ; `_NAME_` =
//!   nom de la variable source ; colonnes = `COL1..COLn` (n = max
//!   d'observations par groupe) ou, si ID, les valeurs (formatées) de
//!   la variable ID — valeurs dupliquées dans un groupe → ERROR comme
//!   SAS ("The ID value ... occurs twice in the same BY group"),
//!   noms invalides normalisés règle SAS (préfixe _ si chiffre...).
//! - Transposer du char et du num ensemble → toutes les COL deviennent
//!   char (longueur max), num convertis via BEST12. trimé.
//!
//! # Décisions d'implémentation (documentées pour l'orchestrateur)
//!
//! ## Nommage des colonnes transposées
//! - SANS `id` : `COL1..COLn` où `n` = MAX du nombre d'observations sur
//!   tous les groupes BY (les groupes plus courts sont complétés par des
//!   missings, comme SAS). Avec `prefix=P` : `P1..Pn`.
//! - AVEC `id` : une colonne par valeur DISTINCTE de la variable ID, dans
//!   l'ordre de PREMIÈRE APPARITION dans les données (choix documenté ;
//!   SAS utilise l'ordre de première apparition). Les valeurs sont
//!   formatées : char telle quelle (trimée), num via `format_best(v,12)`
//!   trimé. Les noms invalides sont normalisés (cf. `normalize_name`).
//!   Une valeur d'ID dupliquée DANS UN GROUPE BY → ERROR exacte SAS.
//!
//! ## Mixage char / numérique des variables VAR
//! - Si TOUTES les variables VAR transposées sont numériques → colonnes
//!   transposées NUMÉRIQUES (f64) ; missing préservé.
//! - Si AU MOINS UNE variable VAR est caractère (mixage) → TOUTES les
//!   colonnes transposées deviennent CARACTÈRE : les valeurs numériques
//!   sont converties via `format_best(v,12).trim()`, un missing numérique
//!   devient une chaîne vide (blanc), un missing char reste vide.
//!
//! ## `out=` absent
//! - Pour M7 on EXIGE `out=` : son absence renvoie une ERROR propre
//!   (SAS produirait sinon `WORK._DATAn_`, hors périmètre M7).

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::missing::{num_to_value, value_to_num};
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{format_best, Value, VarType};
use polars::prelude::*;
use std::cmp::Ordering;

pub struct TransposeAst {
    pub data: Option<DatasetRef>,
    pub out: Option<DatasetRef>,
    pub prefix: Option<String>,
    pub by: Vec<String>,
    pub id: Option<String>,
    pub var: Vec<String>,
    /// Name of the `_NAME_` column (from `name=`); defaults to `_NAME_`.
    pub name: Option<String>,
}

/// Parse `proc transpose [data=a] [out=b] [prefix=P] [name=N] ; [by v...;]
/// [id v;] [var v...;] run;`. Called AFTER "proc transpose" has been
/// consumed. Consumes through `run;` / `quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<TransposeAst> {
    let mut data: Option<DatasetRef> = None;
    let mut out: Option<DatasetRef> = None;
    let mut prefix: Option<String> = None;
    let mut name: Option<String> = None;

    // --- PROC TRANSPOSE statement options, until `;` ---
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next(); // consume `;`
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            ts.next();
            expect_eq(ts, "DATA")?;
            data = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("out") {
            ts.next();
            expect_eq(ts, "OUT")?;
            out = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("prefix") {
            ts.next();
            expect_eq(ts, "PREFIX")?;
            prefix = Some(expect_ident(ts, "PREFIX")?);
        } else if ts.peek().is_kw("name") {
            ts.next();
            expect_eq(ts, "NAME")?;
            name = Some(expect_ident(ts, "NAME")?);
        } else {
            let span = ts.peek().span;
            let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
            return Err(SasError::parse(
                format!("Unexpected option '{bad}' on PROC TRANSPOSE statement."),
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; ---
    let mut by: Vec<String> = Vec::new();
    let mut id: Option<String> = None;
    let mut var: Vec<String> = Vec::new();

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

        if ts.peek().is_kw("by") {
            ts.next();
            by = ts.parse_name_list()?;
            ts.expect_semi()?;
        } else if ts.peek().is_kw("id") {
            ts.next();
            // Single ID variable for M7: take the first name, skip the rest.
            let names = ts.parse_name_list()?;
            id = names.into_iter().next();
            ts.expect_semi()?;
        } else if ts.peek().is_kw("var") {
            ts.next();
            var = ts.parse_name_list()?;
            ts.expect_semi()?;
        } else {
            // Unknown sub-statement: skip it (recovery, like sort/means).
            ts.skip_to_semi();
        }
    }

    Ok(TransposeAst {
        data,
        out,
        prefix,
        by,
        id,
        var,
        name,
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

fn expect_ident(ts: &mut StatementStream, opt: &str) -> Result<String> {
    match ts.peek().ident().map(str::to_string) {
        Some(s) => {
            ts.next();
            Ok(s)
        }
        None => Err(SasError::parse(
            format!("expected an identifier after {opt}="),
            ts.peek().span,
        )),
    }
}

/// Resolve `data=` or `_LAST_` into a concrete DatasetRef.
fn resolve_input(ast: &TransposeAst, session: &Session) -> Result<DatasetRef> {
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

/// Decode one column of a SasDataset into a `Vec<Value>` (downcast once;
/// local equivalent of sort.rs::decode_column — never decode per cell).
fn decode_column(ds: &SasDataset, col_idx: usize) -> Result<Vec<Value>> {
    let series = ds.df.get_columns()[col_idx].as_materialized_series();
    let values = match ds.vars[col_idx].ty {
        VarType::Num => series.f64()?.iter().map(num_to_value).collect(),
        VarType::Char => series
            .str()?
            .iter()
            .map(|o| Value::Char(o.unwrap_or("").to_string()))
            .collect(),
    };
    Ok(values)
}

/// Resolve a variable name to its column index (case-insensitive), erroring
/// like SAS when absent.
fn resolve_var(ds: &SasDataset, vname: &str) -> Result<usize> {
    ds.vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(vname))
        .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", vname.to_uppercase())))
}

/// Format an ID value into a column name candidate (char trimmed; numeric
/// via BEST12. trimmed). Then normalize to a valid SAS name.
fn id_value_to_name(v: &Value) -> String {
    let raw = match v {
        Value::Char(s) => s.trim().to_string(),
        Value::Num(f) => format_best(*f, 12).trim().to_string(),
        Value::Missing(k) => k.display(),
    };
    normalize_name(&raw)
}

/// Normalize an arbitrary string into a valid SAS variable name: replace
/// invalid characters by `_`, prefix `_` when the first char is a digit or
/// the string is empty. (Conservative; SAS uses VALIDVARNAME=V7 by default.)
fn normalize_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().max(1));
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let starts_bad = out
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(true);
    if out.is_empty() || starts_bad {
        let mut prefixed = String::with_capacity(out.len() + 1);
        prefixed.push('_');
        prefixed.push_str(&out);
        prefixed
    } else {
        out
    }
}

/// Convert a Value into its CHAR representation when transposed columns are
/// character (mixing rule). Numeric → BEST12. trimmed; missing → blank.
fn value_to_char(v: &Value) -> Option<String> {
    match v {
        Value::Char(s) => {
            let t = s.trim_end();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Value::Num(f) => Some(format_best(*f, 12).trim().to_string()),
        Value::Missing(_) => None,
    }
}

/// Group row indices by the BY-tuple, preserving first-appearance order of
/// the groups and input order within each group. With no BY columns, one
/// group containing all rows in input order.
fn group_by_tuple(
    by_values: &[Vec<Value>],
    n_obs: usize,
) -> Vec<(Vec<Value>, Vec<usize>)> {
    let mut groups: Vec<(Vec<Value>, Vec<usize>)> = Vec::new();
    for row in 0..n_obs {
        let key: Vec<Value> = by_values.iter().map(|c| c[row].clone()).collect();
        let pos = groups.iter().position(|(k, _)| {
            k.len() == key.len()
                && k.iter()
                    .zip(&key)
                    .all(|(a, b)| a.sas_cmp(b) == Ordering::Equal)
        });
        match pos {
            Some(p) => groups[p].1.push(row),
            None => groups.push((key, vec![row])),
        }
    }
    groups
}

/// Execute PROC TRANSPOSE. Called by `procs::execute_proc` (timing wrapper).
pub fn execute(ast: &TransposeAst, session: &mut Session) -> Result<()> {
    let in_ref = resolve_input(ast, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }

    let n_obs = ds.n_obs();

    // Resolve BY columns.
    let mut by_cols: Vec<usize> = Vec::with_capacity(ast.by.len());
    for vname in &ast.by {
        by_cols.push(resolve_var(&ds, vname)?);
    }

    // Resolve ID column (if any).
    let id_col: Option<usize> = match &ast.id {
        Some(vname) => Some(resolve_var(&ds, vname)?),
        None => None,
    };

    // Determine VAR list: explicit `var`, else all NUMERIC variables not in
    // BY and not the ID variable.
    let var_cols: Vec<usize> = if !ast.var.is_empty() {
        let mut v = Vec::with_capacity(ast.var.len());
        for vname in &ast.var {
            v.push(resolve_var(&ds, vname)?);
        }
        v
    } else {
        (0..ds.vars.len())
            .filter(|&i| {
                ds.vars[i].ty == VarType::Num
                    && !by_cols.contains(&i)
                    && Some(i) != id_col
            })
            .collect()
    };

    if var_cols.is_empty() {
        return Err(SasError::runtime(
            "No variables to transpose (the VAR list is empty).",
        ));
    }

    // Decode BY, ID, and VAR columns once each.
    let by_values: Vec<Vec<Value>> = by_cols
        .iter()
        .map(|&ci| decode_column(&ds, ci))
        .collect::<Result<_>>()?;
    let id_values: Option<Vec<Value>> = match id_col {
        Some(ci) => Some(decode_column(&ds, ci)?),
        None => None,
    };
    let var_values: Vec<Vec<Value>> = var_cols
        .iter()
        .map(|&ci| decode_column(&ds, ci))
        .collect::<Result<_>>()?;

    // Mixing rule: transposed columns are char iff ANY VAR is character.
    let any_char = var_cols.iter().any(|&ci| ds.vars[ci].ty == VarType::Char);

    // Group rows by the BY tuple (first-appearance order).
    let groups = group_by_tuple(&by_values, n_obs);

    // Determine the transposed columns layout.
    // WITHOUT ID: COL1..COLn (or P1..Pn), n = max group size.
    // WITH ID: one column per distinct ID value (first appearance order).
    let prefix = ast.prefix.as_deref().unwrap_or("COL");

    // Column names + per-(group,var) value access.
    // We materialise, for every output row, the list of transposed cell
    // Values in the layout column order, then build series.
    let name_col = ast.name.as_deref().unwrap_or("_NAME_");

    // Each output row carries: BY group key (for the leading BY cells),
    // the source var name (for _NAME_), and the transposed cells.
    struct OutRow {
        by_key: Vec<Value>,
        source_name: String,
        cells: Vec<Value>,
    }
    let mut out_rows: Vec<OutRow> = Vec::new();

    // Transposed column names, computed below per layout.
    let trans_names: Vec<String>;

    if let Some(idv) = &id_values {
        // Distinct ID values in first-appearance order across all data.
        let mut distinct: Vec<Value> = Vec::new();
        for v in idv.iter() {
            if !distinct.iter().any(|d| d.sas_cmp(v) == Ordering::Equal) {
                distinct.push(v.clone());
            }
        }
        trans_names = distinct.iter().map(id_value_to_name).collect();

        for (key, grp_rows) in &groups {
            // Map each distinct ID value -> the row (within this group) whose
            // ID matches it. Duplicate ID within a group -> ERROR.
            let mut row_for_id: Vec<Option<usize>> = vec![None; distinct.len()];
            for &r in grp_rows {
                let di = distinct
                    .iter()
                    .position(|d| d.sas_cmp(&idv[r]) == Ordering::Equal)
                    .expect("ID value must be in the distinct set");
                if row_for_id[di].is_some() {
                    let disp = id_value_display(&idv[r]);
                    return Err(SasError::runtime(format!(
                        "The ID value \"{}\" occurs twice in the same BY group.",
                        disp
                    )));
                }
                row_for_id[di] = Some(r);
            }
            for (vi, &vci) in var_cols.iter().enumerate() {
                let mut cells: Vec<Value> = Vec::with_capacity(distinct.len());
                for &maybe_row in &row_for_id {
                    let v = match maybe_row {
                        Some(r) => var_values[vi][r].clone(),
                        None => Value::missing(),
                    };
                    cells.push(v);
                }
                out_rows.push(OutRow {
                    by_key: key.clone(),
                    source_name: ds.vars[vci].name.clone(),
                    cells,
                });
            }
        }
    } else {
        // COL1..COLn where n = max group size.
        let n_cols = groups.iter().map(|(_, r)| r.len()).max().unwrap_or(0);
        trans_names = (1..=n_cols).map(|j| format!("{prefix}{j}")).collect();

        for (key, grp_rows) in &groups {
            for (vi, &vci) in var_cols.iter().enumerate() {
                let mut cells: Vec<Value> = Vec::with_capacity(n_cols);
                for j in 0..n_cols {
                    let v = match grp_rows.get(j) {
                        Some(&r) => var_values[vi][r].clone(),
                        None => Value::missing(),
                    };
                    cells.push(v);
                }
                out_rows.push(OutRow {
                    by_key: key.clone(),
                    source_name: ds.vars[vci].name.clone(),
                    cells,
                });
            }
        }
    }

    // --- Build the output DataFrame column by column ---
    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();

    // Leading BY columns (copy input VarMeta).
    for (bi, &col_idx) in by_cols.iter().enumerate() {
        let meta = &ds.vars[col_idx];
        let series = match meta.ty {
            VarType::Num => {
                let vals: Vec<Option<f64>> =
                    out_rows.iter().map(|r| value_to_num(&r.by_key[bi])).collect();
                Series::new(meta.name.as_str().into(), vals)
            }
            VarType::Char => {
                let vals: Vec<Option<String>> = out_rows
                    .iter()
                    .map(|r| char_cell(&r.by_key[bi]))
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
        };
        columns.push(series.into());
        vars.push(meta.clone());
    }

    // _NAME_ column (char). Length = max source-name length.
    let name_vals: Vec<Option<String>> = out_rows
        .iter()
        .map(|r| Some(r.source_name.clone()))
        .collect();
    let name_len = out_rows
        .iter()
        .map(|r| r.source_name.len())
        .max()
        .unwrap_or(8)
        .max(1);
    columns.push(Series::new(name_col.into(), name_vals).into());
    vars.push(char_var_meta(name_col, name_len));

    // Transposed columns.
    if any_char {
        // CHAR columns. Length = max char-cell length across all cells.
        let mut char_len = 1usize;
        for ci in 0..trans_names.len() {
            let vals: Vec<Option<String>> = out_rows
                .iter()
                .map(|r| value_to_char(&r.cells[ci]))
                .collect();
            for s in vals.iter().flatten() {
                char_len = char_len.max(s.len());
            }
            columns.push(Series::new(trans_names[ci].as_str().into(), vals).into());
        }
        for nm in &trans_names {
            vars.push(char_var_meta(nm, char_len));
        }
    } else {
        // NUMERIC columns.
        for ci in 0..trans_names.len() {
            let vals: Vec<Option<f64>> = out_rows
                .iter()
                .map(|r| value_to_num(&r.cells[ci]))
                .collect();
            columns.push(Series::new(trans_names[ci].as_str().into(), vals).into());
            vars.push(num_var_meta(&trans_names[ci]));
        }
    }

    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars };

    // out= is required for M7.
    let out_ref = ast.out.clone().ok_or_else(|| {
        SasError::runtime("The OUT= option is required for PROC TRANSPOSE.")
    })?;
    let out_libref = out_ref.libref_or_work();
    let out_table = out_ref.name.to_uppercase();
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

/// Display form of an ID value for the duplicate-error message (char trimmed,
/// numeric via BEST12. trimmed, missing via its display char).
fn id_value_display(v: &Value) -> String {
    match v {
        Value::Char(s) => s.trim().to_string(),
        Value::Num(f) => format_best(*f, 12).trim().to_string(),
        Value::Missing(k) => k.display(),
    }
}

/// Encode a Value into an Option<String> for a CHAR output column (blank /
/// missing → None).
fn char_cell(v: &Value) -> Option<String> {
    match v {
        Value::Char(s) if s.trim_end().is_empty() => None,
        Value::Char(s) => Some(s.trim_end().to_string()),
        Value::Missing(_) => None,
        Value::Num(_) => None,
    }
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

    fn parse_transpose(src: &str) -> Result<TransposeAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "transpose"
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

    fn char_meta(name: &str, length: usize) -> VarMeta {
        VarMeta {
            name: name.to_string(),
            ty: VarType::Char,
            length,
            format: None,
            label: None,
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

    fn out_ref(name: &str) -> DatasetRef {
        DatasetRef {
            libref: Some("WORK".into()),
            name: name.into(),
        }
    }

    fn data_ref(name: &str) -> Option<DatasetRef> {
        Some(DatasetRef {
            libref: Some("WORK".into()),
            name: name.into(),
        })
    }

    // ───────────────────────────── parse tests ─────────────────────────────

    #[test]
    fn parse_full_statement() {
        let ast =
            parse_transpose("proc transpose data=a out=b prefix=p; by g; id k; var x y; run;")
                .unwrap();
        assert_eq!(ast.data.as_ref().unwrap().name, "a");
        assert_eq!(ast.out.as_ref().unwrap().name, "b");
        assert_eq!(ast.prefix.as_deref(), Some("p"));
        assert_eq!(ast.by, vec!["g".to_string()]);
        assert_eq!(ast.id.as_deref(), Some("k"));
        assert_eq!(ast.var, vec!["x".to_string(), "y".to_string()]);
    }

    #[test]
    fn parse_name_option() {
        let ast = parse_transpose("proc transpose data=a out=b name=src; var x; run;").unwrap();
        assert_eq!(ast.name.as_deref(), Some("src"));
    }

    #[test]
    fn parse_unknown_option_errors() {
        let r = parse_transpose("proc transpose data=a bogus; run;");
        assert!(r.is_err());
        let msg = r.err().unwrap().to_string();
        assert!(msg.contains("BOGUS"), "msg: {msg}");
    }

    #[test]
    fn parse_unknown_substatement_skipped() {
        // The DELETE substatement is unrecognized and should be skipped.
        let ast =
            parse_transpose("proc transpose data=a out=b; delete foo; var x; run;").unwrap();
        assert_eq!(ast.var, vec!["x".to_string()]);
    }

    // ───────────────────────── normalize_name tests ────────────────────────

    #[test]
    fn normalize_name_rules() {
        assert_eq!(normalize_name("abc"), "abc");
        assert_eq!(normalize_name("1x"), "_1x");
        assert_eq!(normalize_name(""), "_");
        assert_eq!(normalize_name("a b"), "a_b");
        assert_eq!(normalize_name("a-b"), "a_b");
    }

    // ───────────────────────────── execute tests ───────────────────────────

    #[test]
    fn execute_simple_no_by_no_id() {
        let mut session = make_session();
        let df = df!["x" => [10.0_f64, 20.0, 30.0]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = TransposeAst {
            data: data_ref("T"),
            out: Some(out_ref("O")),
            prefix: None,
            by: vec![],
            id: None,
            var: vec!["x".into()],
            name: None,
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        assert_eq!(out.n_obs(), 1);
        // _NAME_ = "x"
        let name = read_col(&session, "O", "_NAME_");
        assert_eq!(name, vec![Value::Char("x".into())]);
        // COL1..COL3 = 10,20,30
        assert_eq!(read_col(&session, "O", "COL1"), vec![Value::Num(10.0)]);
        assert_eq!(read_col(&session, "O", "COL2"), vec![Value::Num(20.0)]);
        assert_eq!(read_col(&session, "O", "COL3"), vec![Value::Num(30.0)]);
    }

    #[test]
    fn execute_prefix_renames_cols() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64, 2.0]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = TransposeAst {
            data: data_ref("T"),
            out: Some(out_ref("O")),
            prefix: Some("V".into()),
            by: vec![],
            id: None,
            var: vec!["x".into()],
            name: None,
        };
        execute(&ast, &mut session).unwrap();

        assert_eq!(read_col(&session, "O", "V1"), vec![Value::Num(1.0)]);
        assert_eq!(read_col(&session, "O", "V2"), vec![Value::Num(2.0)]);
    }

    #[test]
    fn execute_with_by_pads_shorter_group() {
        let mut session = make_session();
        // group g=1 has 2 rows, g=2 has 1 row -> max 2 cols, g=2 padded.
        let df = df![
            "g" => [1.0_f64, 1.0, 2.0],
            "x" => [10.0_f64, 11.0, 20.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("g"), num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = TransposeAst {
            data: data_ref("T"),
            out: Some(out_ref("O")),
            prefix: None,
            by: vec!["g".into()],
            id: None,
            var: vec!["x".into()],
            name: None,
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        assert_eq!(out.n_obs(), 2); // one row per (group × var)

        let g = read_col(&session, "O", "g");
        let c1 = read_col(&session, "O", "COL1");
        let c2 = read_col(&session, "O", "COL2");
        assert_eq!(g, vec![Value::Num(1.0), Value::Num(2.0)]);
        assert_eq!(c1, vec![Value::Num(10.0), Value::Num(20.0)]);
        // g=1 -> 11; g=2 padded with missing.
        assert_eq!(c2[0], Value::Num(11.0));
        assert!(c2[1].is_missing());
    }

    #[test]
    fn execute_with_id_names_columns() {
        let mut session = make_session();
        let df = df![
            "k" => ["red", "blue"],
            "x" => [1.0_f64, 2.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("k", 4), num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = TransposeAst {
            data: data_ref("T"),
            out: Some(out_ref("O")),
            prefix: None,
            by: vec![],
            id: Some("k".into()),
            var: vec!["x".into()],
            name: None,
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        assert_eq!(out.n_obs(), 1);
        // Columns named by ID values in first-appearance order: red, blue.
        let cols: Vec<String> = out.vars.iter().map(|m| m.name.clone()).collect();
        assert!(cols.contains(&"red".to_string()), "cols: {cols:?}");
        assert!(cols.contains(&"blue".to_string()), "cols: {cols:?}");
        assert_eq!(read_col(&session, "O", "red"), vec![Value::Num(1.0)]);
        assert_eq!(read_col(&session, "O", "blue"), vec![Value::Num(2.0)]);
    }

    #[test]
    fn execute_with_id_numeric_values_normalized() {
        let mut session = make_session();
        // numeric ID values 1,2 -> names "_1","_2" (start with digit).
        let df = df![
            "k" => [1.0_f64, 2.0],
            "x" => [7.0_f64, 8.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("k"), num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = TransposeAst {
            data: data_ref("T"),
            out: Some(out_ref("O")),
            prefix: None,
            by: vec![],
            id: Some("k".into()),
            var: vec!["x".into()],
            name: None,
        };
        execute(&ast, &mut session).unwrap();

        let cols: Vec<String> = {
            let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
            out.vars.iter().map(|m| m.name.clone()).collect()
        };
        assert!(cols.contains(&"_1".to_string()), "cols: {cols:?}");
        assert!(cols.contains(&"_2".to_string()), "cols: {cols:?}");
        assert_eq!(read_col(&session, "O", "_1"), vec![Value::Num(7.0)]);
        assert_eq!(read_col(&session, "O", "_2"), vec![Value::Num(8.0)]);
    }

    #[test]
    fn execute_duplicate_id_in_group_errors() {
        let mut session = make_session();
        // Two rows with the same ID "a" in the (single) BY group -> ERROR.
        let df = df![
            "k" => ["a", "a"],
            "x" => [1.0_f64, 2.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("k", 1), num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = TransposeAst {
            data: data_ref("T"),
            out: Some(out_ref("O")),
            prefix: None,
            by: vec![],
            id: Some("k".into()),
            var: vec!["x".into()],
            name: None,
        };
        let r = execute(&ast, &mut session);
        assert!(r.is_err());
        let msg = r.err().unwrap().to_string();
        assert!(
            msg.contains("The ID value \"a\" occurs twice in the same BY group."),
            "msg: {msg}"
        );
    }

    #[test]
    fn execute_mixing_char_and_numeric_makes_char_cols() {
        let mut session = make_session();
        // var x (num), var y (char) -> all COL columns become char.
        let df = df![
            "x" => [1.0_f64, 2.0],
            "y" => ["a", "b"]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x"), char_meta("y", 1)] };
        write_dataset(&mut session, "T", ds);

        let ast = TransposeAst {
            data: data_ref("T"),
            out: Some(out_ref("O")),
            prefix: None,
            by: vec![],
            id: None,
            var: vec!["x".into(), "y".into()],
            name: None,
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        // Two output rows: one per var.
        assert_eq!(out.n_obs(), 2);
        // COL columns must be char.
        for nm in ["COL1", "COL2"] {
            let meta = out.vars.iter().find(|m| m.name == nm).unwrap();
            assert_eq!(meta.ty, VarType::Char, "col {nm} should be char");
        }
        // Row 0 = var x: numeric values rendered as char "1","2".
        let c1 = read_col(&session, "O", "COL1");
        let c2 = read_col(&session, "O", "COL2");
        assert_eq!(c1[0], Value::Char("1".into()));
        assert_eq!(c2[0], Value::Char("2".into()));
        // Row 1 = var y: char values "a","b".
        assert_eq!(c1[1], Value::Char("a".into()));
        assert_eq!(c2[1], Value::Char("b".into()));

        // _NAME_ rows are the source names x, y.
        let name = read_col(&session, "O", "_NAME_");
        assert_eq!(
            name,
            vec![Value::Char("x".into()), Value::Char("y".into())]
        );
    }

    #[test]
    fn execute_default_var_all_numeric_excludes_by_and_id() {
        let mut session = make_session();
        // var list empty -> all numeric except BY(g) and ID(k): only x.
        let df = df![
            "g" => [1.0_f64],
            "k" => [5.0_f64],
            "x" => [9.0_f64]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("g"), num_meta("k"), num_meta("x")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = TransposeAst {
            data: data_ref("T"),
            out: Some(out_ref("O")),
            prefix: None,
            by: vec!["g".into()],
            id: Some("k".into()),
            var: vec![],
            name: None,
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        // Single var x -> one output row.
        assert_eq!(out.n_obs(), 1);
        let name = read_col(&session, "O", "_NAME_");
        assert_eq!(name, vec![Value::Char("x".into())]);
    }

    #[test]
    fn execute_name_option_renames_name_col() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = TransposeAst {
            data: data_ref("T"),
            out: Some(out_ref("O")),
            prefix: None,
            by: vec![],
            id: None,
            var: vec!["x".into()],
            name: Some("source".into()),
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("O").unwrap();
        let cols: Vec<String> = out.vars.iter().map(|m| m.name.clone()).collect();
        assert!(cols.contains(&"source".to_string()), "cols: {cols:?}");
        assert!(!cols.contains(&"_NAME_".to_string()), "cols: {cols:?}");
    }

    #[test]
    fn execute_missing_out_errors() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = TransposeAst {
            data: data_ref("T"),
            out: None,
            prefix: None,
            by: vec![],
            id: None,
            var: vec!["x".into()],
            name: None,
        };
        let r = execute(&ast, &mut session);
        assert!(r.is_err());
        let msg = r.err().unwrap().to_string();
        assert!(msg.contains("OUT="), "msg: {msg}");
    }

    #[test]
    fn execute_emits_dataset_note() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64, 2.0]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("x")] };
        write_dataset(&mut session, "T", ds);

        let ast = TransposeAst {
            data: data_ref("T"),
            out: Some(out_ref("O")),
            prefix: None,
            by: vec![],
            id: None,
            var: vec!["x".into()],
            name: None,
        };
        execute(&ast, &mut session).unwrap();

        let log = session.log.into_string();
        assert!(
            log.contains("The data set WORK.O has 1 observations and"),
            "log: {log}"
        );
    }
}
