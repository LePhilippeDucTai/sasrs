//! PROC REPORT (bounded v1, LISTING only).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! ## Syntaxe v1
//! ```text
//! proc report data=<ref> [nowd|nowindow] [noheader] [headline] [headskip];
//!     column <name list>;          /* a.k.a. `columns` */
//!     define <var> / <usage> [order=asc|desc] ['label'] ;
//!     run; | quit;
//! ```
//!
//! Usages (sur DEFINE) :
//!   - `DISPLAY`  : affiche la valeur brute par observation.
//!   - `ORDER`    : variable de tri ; chaque valeur distincte → une ligne.
//!   - `GROUP`    : comme ORDER mais regroupe (collapse) les doublons.
//!   - `ANALYSIS` : variable d'agrégat ; statistique optionnelle parmi
//!                  SUM MEAN MIN MAX N STD (défaut SUM).
//!
//! Défauts d'usage (variable SANS define, comme SAS) :
//!   - numérique → ANALYSIS SUM
//!   - caractère → DISPLAY
//!
//! ## Sémantique (sous-ensemble fidèle)
//! - AUCUN GROUP/ORDER  → **rapport détaillé** : une ligne listing par
//!   observation, colonnes dans l'ordre COLUMN, valeur brute par cellule
//!   (les variables ANALYSIS impriment aussi leur valeur brute par ligne,
//!   comme SAS dans un rapport détaillé).
//! - AU MOINS UN GROUP/ORDER → **rapport résumé** : on trie/regroupe par
//!   le tuple des colonnes GROUP+ORDER (ordre/égalité via `Value::sas_cmp`,
//!   réutilise `common::group_by_keys`). ORDER conserve chaque valeur
//!   distincte ; GROUP réduit les doublons (la clé étant un tuple, GROUP et
//!   ORDER produisent les mêmes groupes : la distinction GROUP vs ORDER
//!   n'affecte v1 que l'affichage — voir DISPLAY ci-dessous). Pour chaque
//!   groupe, les colonnes ANALYSIS sont calculées via `means::compute` sur
//!   les valeurs non-missing du groupe (`common::partition_numeric`).
//! - Variables DISPLAY dans un rapport résumé : on imprime la valeur si elle
//!   est constante dans le groupe, sinon une cellule vide (simplification
//!   documentée).
//!
//! ## En-têtes
//! Label du DEFINE s'il est donné, sinon le NOM de la variable tel que
//! stocké (SAS met le nom en majuscules ; on garde la casse stockée —
//! simplification documentée). Ligne d'en-tête supprimée sous `noheader`.
//! Numériques formatés comme PRINT/means (`format_best`) ; missing → `.`.
//!
//! ## DEFERRALS (erreurs PROPRES + doc ici) — v1 ne supporte PAS :
//!   - usage `ACROSS`               → "PROC REPORT v1 does not support the
//!                                     ACROSS usage."
//!   - blocs `COMPUTE`/`ENDCOMP` et colonnes calculées
//!                                  → "PROC REPORT v1 does not support COMPUTE
//!                                     blocks."
//!   - lignes de synthèse `BREAK`/`RBREAK`
//!                                  → "PROC REPORT v1 does not support
//!                                     BREAK/RBREAK statements."
//!   - statements `LINE`            → "PROC REPORT v1 does not support LINE
//!                                     statements."
//!   - `WHERE`                      → "PROC REPORT v1 does not support the
//!                                     WHERE statement."
//!   - options DEFINE complexes au-delà de order=/label (FLOW, FORMAT=,
//!                                     WIDTH=, SPACING=, multi-label, ...)
//!                                  → "PROC REPORT v1 does not support the
//!                                     DEFINE option 'XXX'."
//!   - `OUT=` et tout dataset de sortie : v1 n'écrit AUCUN dataset et ne
//!     touche pas `last_dataset`.
//!   - options PROC autres que nowd/nowindow/noheader/headline/headskip
//!                                  → "Unexpected option 'XXX' on PROC REPORT
//!                                     statement."

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::parser::StatementStream;
use crate::procs::common::{decode_column, group_by_keys, partition_numeric};
use crate::procs::means;
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{format_best, Value, VarType};
use std::cmp::Ordering;

/// Usage of a column in the report.
#[derive(Debug, Clone, PartialEq)]
pub enum Usage {
    Display,
    Order,
    Group,
    /// ANALYSIS with a statistic keyword (sum/mean/min/max/n/std).
    Analysis(String),
}

/// Sort direction for ORDER/GROUP usage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OrderDir {
    Ascending,
    Descending,
}

/// A parsed DEFINE statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Define {
    pub var: String,
    pub usage: Usage,
    pub order: OrderDir,
    pub label: Option<String>,
}

pub struct ReportAst {
    pub data: Option<DatasetRef>,
    pub noheader: bool,
    /// COLUMN list (display order). `None` → all variables in dataset order.
    pub columns: Option<Vec<String>>,
    pub defines: Vec<Define>,
}

/// Statistic keywords accepted after an ANALYSIS usage on a DEFINE.
const ANALYSIS_STATS: &[&str] = &["sum", "mean", "min", "max", "n", "std"];

fn is_analysis_stat(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    ANALYSIS_STATS.iter().any(|k| *k == l)
}

/// Parse a PROC REPORT block. Called AFTER `proc report` has been consumed.
/// Consumes through `run;`/`quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<ReportAst> {
    let mut data: Option<DatasetRef> = None;
    let mut noheader = false;
    let mut columns: Option<Vec<String>> = None;
    let mut defines: Vec<Define> = Vec::new();

    // --- PROC REPORT statement options, until `;` ---
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
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse("expected '=' after DATA", ts.peek().span));
            }
            ts.next();
            data = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("nowd") || ts.peek().is_kw("nowindow") {
            // No-op: we never open an interactive window.
            ts.next();
        } else if ts.peek().is_kw("noheader") {
            ts.next();
            noheader = true;
        } else if ts.peek().is_kw("headline") || ts.peek().is_kw("headskip") {
            // No-op cosmetic options (rule line / skip line under headers).
            ts.next();
        } else {
            let span = ts.peek().span;
            let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
            return Err(SasError::parse(
                format!("Unexpected option '{bad}' on PROC REPORT statement."),
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; ---
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

        if ts.peek().is_kw("column") || ts.peek().is_kw("columns") {
            ts.next();
            columns = Some(ts.parse_name_list()?);
            ts.expect_semi()?;
        } else if ts.peek().is_kw("define") {
            ts.next();
            defines.push(parse_define(ts)?);
        } else if ts.peek().is_kw("compute") {
            return Err(SasError::runtime(
                "PROC REPORT v1 does not support COMPUTE blocks.",
            ));
        } else if ts.peek().is_kw("break") || ts.peek().is_kw("rbreak") {
            return Err(SasError::runtime(
                "PROC REPORT v1 does not support BREAK/RBREAK statements.",
            ));
        } else if ts.peek().is_kw("line") {
            return Err(SasError::runtime(
                "PROC REPORT v1 does not support LINE statements.",
            ));
        } else if ts.peek().is_kw("where") {
            return Err(SasError::runtime(
                "PROC REPORT v1 does not support the WHERE statement.",
            ));
        } else {
            let span = ts.peek().span;
            let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
            return Err(SasError::parse(
                format!("Unexpected statement '{bad}' in PROC REPORT."),
                span,
            ));
        }
    }

    Ok(ReportAst {
        data,
        noheader,
        columns,
        defines,
    })
}

/// Parse a DEFINE statement body, after `define` was consumed, through `;`.
/// `define <var> / <usage> [order=asc|desc] ['label'] ;`
fn parse_define(ts: &mut StatementStream) -> Result<Define> {
    // Variable name.
    let var = match ts.peek().ident().map(str::to_string) {
        Some(v) => {
            ts.next();
            v
        }
        None => {
            return Err(SasError::parse(
                "expected a variable name after DEFINE",
                ts.peek().span,
            ));
        }
    };

    // Optional `/` introducing attributes. If the statement ends right away
    // (just `define var;`), SAS uses the column's default usage; we mirror
    // that by leaving usage unset here and resolving the default at execute.
    let mut usage: Option<Usage> = None;
    let mut order = OrderDir::Ascending;
    let mut label: Option<String> = None;

    if ts.peek().kind == TokenKind::Slash {
        ts.next(); // consume '/'
        loop {
            match &ts.peek().kind {
                TokenKind::Semi | TokenKind::Eof => break,
                TokenKind::Str { value, .. } => {
                    label = Some(value.clone());
                    ts.next();
                }
                TokenKind::Ident(raw) => {
                    let kw = raw.to_ascii_lowercase();
                    match kw.as_str() {
                        "display" => {
                            usage = Some(Usage::Display);
                            ts.next();
                        }
                        "order" => {
                            // `order` is BOTH a usage AND an option `order=`.
                            // Disambiguate via the following token.
                            ts.next();
                            if ts.peek().kind == TokenKind::Eq {
                                ts.next(); // '='
                                order = parse_order_dir(ts)?;
                                // `order=` does not by itself set the usage;
                                // it only applies to ORDER/GROUP. If usage is
                                // still unset, treat the column as ORDER.
                                if usage.is_none() {
                                    usage = Some(Usage::Order);
                                }
                            } else {
                                usage = Some(Usage::Order);
                            }
                        }
                        "group" => {
                            usage = Some(Usage::Group);
                            ts.next();
                        }
                        "analysis" => {
                            ts.next();
                            // Optional statistic keyword follows.
                            let stat = if let Some(s) = ts.peek().ident() {
                                if is_analysis_stat(s) {
                                    let st = s.to_ascii_lowercase();
                                    ts.next();
                                    st
                                } else {
                                    "sum".to_string()
                                }
                            } else {
                                "sum".to_string()
                            };
                            usage = Some(Usage::Analysis(stat));
                        }
                        // A bare statistic keyword (e.g. `define x / sum;`) is
                        // shorthand for ANALYSIS <stat> in SAS.
                        s if is_analysis_stat(s) => {
                            usage = Some(Usage::Analysis(s.to_string()));
                            ts.next();
                        }
                        "across" => {
                            return Err(SasError::runtime(
                                "PROC REPORT v1 does not support the ACROSS usage.",
                            ));
                        }
                        "computed" => {
                            return Err(SasError::runtime(
                                "PROC REPORT v1 does not support COMPUTE blocks.",
                            ));
                        }
                        other => {
                            return Err(SasError::runtime(format!(
                                "PROC REPORT v1 does not support the DEFINE option '{}'.",
                                other.to_uppercase()
                            )));
                        }
                    }
                }
                _ => {
                    return Err(SasError::parse(
                        "unexpected token in DEFINE statement",
                        ts.peek().span,
                    ));
                }
            }
        }
    }

    ts.expect_semi()?;

    // If no explicit usage was given, leave a placeholder that resolves to the
    // SAS type-based default at execute time. We encode "unset" as Display
    // here only when we KNOW the column; but since type is unknown at parse,
    // signal "unset" via a sentinel. Simplest: store usage=None semantics by
    // defaulting to Display and recording whether it was explicit.
    let usage = usage.unwrap_or(Usage::Display);

    Ok(Define {
        var,
        usage,
        order,
        label,
    })
}

fn parse_order_dir(ts: &mut StatementStream) -> Result<OrderDir> {
    match ts.peek().ident() {
        Some(s) if s.eq_ignore_ascii_case("descending") || s.eq_ignore_ascii_case("desc") => {
            ts.next();
            Ok(OrderDir::Descending)
        }
        Some(s) if s.eq_ignore_ascii_case("ascending") || s.eq_ignore_ascii_case("asc") => {
            ts.next();
            Ok(OrderDir::Ascending)
        }
        _ => Err(SasError::parse(
            "expected ASCENDING or DESCENDING after ORDER=",
            ts.peek().span,
        )),
    }
}

/// Resolve `data=` or `_LAST_` into a concrete DatasetRef.
fn resolve_input(ast: &ReportAst, session: &Session) -> Result<DatasetRef> {
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

/// Resolved per-column plan entry: index into the dataset, effective usage,
/// order direction, and the header text to display.
struct ColPlan {
    idx: usize,
    usage: Usage,
    dir: OrderDir,
    header: String,
}

/// Render a Value into a listing cell (numeric via format_best, missing → ".").
fn fmt_cell(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.clone(),
    }
}

/// Execute PROC REPORT. Called by `procs::execute_proc`.
pub fn execute(ast: &ReportAst, session: &mut Session) -> Result<()> {
    let in_ref = resolve_input(ast, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();
    let display_name = in_ref.display();

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }
    let n_obs = ds.n_obs();

    // --- Resolve the column list (display order). ---
    let col_names: Vec<String> = match &ast.columns {
        Some(list) => list.clone(),
        None => ds.vars.iter().map(|m| m.name.clone()).collect(),
    };

    // --- Build the per-column plan, applying DEFINEs and type defaults. ---
    let mut plan: Vec<ColPlan> = Vec::with_capacity(col_names.len());
    for cname in &col_names {
        let idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(cname))
            .ok_or_else(|| {
                SasError::runtime(format!("Variable {} not found.", cname.to_uppercase()))
            })?;

        let def = ast
            .defines
            .iter()
            .find(|d| d.var.eq_ignore_ascii_case(cname));

        let usage = match def {
            Some(d) => d.usage.clone(),
            None => match ds.vars[idx].ty {
                VarType::Num => Usage::Analysis("sum".to_string()),
                VarType::Char => Usage::Display,
            },
        };
        let dir = def.map(|d| d.order).unwrap_or(OrderDir::Ascending);
        let header = match def.and_then(|d| d.label.clone()) {
            Some(lbl) => lbl,
            None => ds.vars[idx].name.clone(),
        };

        plan.push(ColPlan {
            idx,
            usage,
            dir,
            header,
        });
    }

    // Decode every planned column once.
    let decoded: Vec<Vec<Value>> = plan
        .iter()
        .map(|c| decode_column(&ds, c.idx))
        .collect::<Result<_>>()?;

    // Determine whether this is a summary report.
    let group_positions: Vec<usize> = plan
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c.usage, Usage::Group | Usage::Order))
        .map(|(i, _)| i)
        .collect();
    let is_summary = !group_positions.is_empty();

    // --- Headers & alignments ---
    let headers: Vec<String> = plan.iter().map(|c| c.header.clone()).collect();
    let aligns: Vec<Align> = plan
        .iter()
        .map(|c| match c.usage {
            Usage::Analysis(_) => Align::Right,
            _ => match ds.vars[c.idx].ty {
                VarType::Num => Align::Right,
                VarType::Char => Align::Left,
            },
        })
        .collect();

    let rows: Vec<Vec<String>> = if !is_summary {
        // ── Detail report: one listing row per observation. ──
        let mut rows = Vec::with_capacity(n_obs);
        for r in 0..n_obs {
            let row: Vec<String> = decoded.iter().map(|col| fmt_cell(&col[r])).collect();
            rows.push(row);
        }
        rows
    } else {
        // ── Summary report: group by GROUP+ORDER key columns. ──
        let key_refs: Vec<&Vec<Value>> = group_positions.iter().map(|&p| &decoded[p]).collect();
        let mut groups = group_by_keys(&key_refs, n_obs);

        // Apply DESCENDING direction lexicographically over the key tuple if
        // any key column requests it. group_by_keys returns ascending order;
        // we re-sort honoring each key column's direction.
        let dirs: Vec<OrderDir> = group_positions.iter().map(|&p| plan[p].dir).collect();
        groups.sort_by(|(a, _), (b, _)| {
            for ((x, y), dir) in a.iter().zip(b).zip(&dirs) {
                let mut c = x.sas_cmp(y);
                if *dir == OrderDir::Descending {
                    c = c.reverse();
                }
                if c != Ordering::Equal {
                    return c;
                }
            }
            Ordering::Equal
        });

        let mut rows = Vec::with_capacity(groups.len());
        for (_key, grp_rows) in &groups {
            let mut row: Vec<String> = Vec::with_capacity(plan.len());
            for (ci, c) in plan.iter().enumerate() {
                let cell = match &c.usage {
                    Usage::Group | Usage::Order => {
                        // Constant within the group by construction (key col).
                        fmt_cell(&decoded[ci][grp_rows[0]])
                    }
                    Usage::Analysis(stat) => {
                        let (xs, nmiss) = partition_numeric(&decoded[ci], grp_rows);
                        let v = means::compute(stat, &xs, nmiss);
                        fmt_cell(&v)
                    }
                    Usage::Display => {
                        // Print the value if constant within the group, else
                        // blank (documented simplification).
                        let first = &decoded[ci][grp_rows[0]];
                        let constant = grp_rows
                            .iter()
                            .all(|&r| decoded[ci][r].sas_cmp(first) == Ordering::Equal);
                        if constant {
                            fmt_cell(first)
                        } else {
                            String::new()
                        }
                    }
                };
                row.push(cell);
            }
            rows.push(row);
        }
        rows
    };

    // --- Write the listing. ---
    session.listing.page_header();
    if ast.noheader {
        // Suppress the header row: pass empty header strings so write_table
        // still column-aligns the data without printing a header line.
        write_table_noheader(session, &aligns, &rows);
    } else {
        session.listing.write_table(&headers, &aligns, &rows);
    }

    // NOTE — observations read (plural invariable, as in PRINT).
    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        n_obs, display_name
    ));

    // PROC REPORT v1 has NO output dataset: do NOT set last_dataset.
    Ok(())
}

/// Render a table without a header row. We compute column widths from the
/// data cells and align each column, mirroring the listing's table layout
/// but skipping the header line entirely (NOHEADER option).
fn write_table_noheader(session: &mut Session, aligns: &[Align], rows: &[Vec<String>]) {
    let ncol = aligns.len();
    if ncol == 0 {
        return;
    }
    let mut widths = vec![0usize; ncol];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.len());
        }
    }
    for row in rows {
        let mut line = String::new();
        for (i, cell) in row.iter().enumerate() {
            if i > 0 {
                line.push_str("  ");
            }
            let w = widths[i];
            match aligns[i] {
                Align::Right => {
                    let pad = w.saturating_sub(cell.len());
                    line.push_str(&" ".repeat(pad));
                    line.push_str(cell);
                }
                Align::Left => {
                    line.push_str(cell);
                    let pad = w.saturating_sub(cell.len());
                    line.push_str(&" ".repeat(pad));
                }
            }
        }
        session.listing.write_line(line.trim_end());
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

    fn parse_report(src: &str) -> Result<ReportAst> {
        let source = SourceFile::new(src);
        let mut ts = crate::parser::StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "report"
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
            length: 8,
            format: None,
            label: None,
        }
    }

    fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
        session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
        session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
    }

    // ───────────────────────── parse tests ─────────────────────────

    #[test]
    fn parse_minimal() {
        let ast = parse_report("proc report data=a nowd; run;").unwrap();
        assert_eq!(ast.data.as_ref().unwrap().name, "a");
        assert!(!ast.noheader);
        assert!(ast.columns.is_none());
        assert!(ast.defines.is_empty());
    }

    #[test]
    fn parse_column_and_defines() {
        let ast = parse_report(
            "proc report data=a nowd; column region sales; \
             define region / group 'Region'; \
             define sales / analysis sum 'Total Sales'; run;",
        )
        .unwrap();
        assert_eq!(
            ast.columns,
            Some(vec!["region".to_string(), "sales".to_string()])
        );
        assert_eq!(ast.defines.len(), 2);
        assert_eq!(ast.defines[0].usage, Usage::Group);
        assert_eq!(ast.defines[0].label.as_deref(), Some("Region"));
        assert_eq!(ast.defines[1].usage, Usage::Analysis("sum".to_string()));
        assert_eq!(ast.defines[1].label.as_deref(), Some("Total Sales"));
    }

    #[test]
    fn parse_order_descending() {
        let ast =
            parse_report("proc report data=a; define x / order order=descending; run;").unwrap();
        assert_eq!(ast.defines[0].usage, Usage::Order);
        assert_eq!(ast.defines[0].order, OrderDir::Descending);
    }

    #[test]
    fn parse_analysis_default_stat_is_sum() {
        let ast = parse_report("proc report data=a; define x / analysis; run;").unwrap();
        assert_eq!(ast.defines[0].usage, Usage::Analysis("sum".to_string()));
    }

    #[test]
    fn parse_noheader_option() {
        let ast = parse_report("proc report data=a noheader; run;").unwrap();
        assert!(ast.noheader);
    }

    #[test]
    fn parse_columns_keyword_alias() {
        let ast = parse_report("proc report data=a; columns x y; run;").unwrap();
        assert_eq!(ast.columns, Some(vec!["x".to_string(), "y".to_string()]));
    }

    #[test]
    fn parse_bad_proc_option_errors() {
        let r = parse_report("proc report data=a frobnicate; run;");
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("FROBNICATE"));
    }

    #[test]
    fn parse_across_usage_errors_cleanly() {
        let r = parse_report("proc report data=a; define x / across; run;");
        let msg = r.err().unwrap().to_string();
        assert!(msg.contains("ACROSS"), "msg: {msg}");
    }

    #[test]
    fn parse_compute_block_errors_cleanly() {
        let r = parse_report("proc report data=a; compute after; endcomp; run;");
        let msg = r.err().unwrap().to_string();
        assert!(msg.contains("COMPUTE"), "msg: {msg}");
    }

    #[test]
    fn parse_break_errors_cleanly() {
        let r = parse_report("proc report data=a; break after region / summarize; run;");
        let msg = r.err().unwrap().to_string();
        assert!(msg.contains("BREAK"), "msg: {msg}");
    }

    #[test]
    fn parse_unknown_define_option_errors() {
        let r = parse_report("proc report data=a; define x / display flow; run;");
        let msg = r.err().unwrap().to_string();
        assert!(msg.contains("FLOW"), "msg: {msg}");
    }

    // ───────────────────────── execute tests ─────────────────────────

    #[test]
    fn detail_report_explicit_column_order() {
        let mut session = make_session();
        let df = df![
            "name" => ["Alice", "Bob", "Carol"],
            "age" => [30.0_f64, 25.0, 40.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("name"), num_meta("age")],
        };
        write_dataset(&mut session, "T", ds);

        // Reverse order: age then name. All DISPLAY (force via define for age
        // so it does NOT trigger summary; numeric default would be ANALYSIS
        // but with no group it's still a detail report → raw per-row value).
        let ast = ReportAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            noheader: false,
            columns: Some(vec!["age".into(), "name".into()]),
            defines: vec![],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // All three names and ages present (raw per-row values).
        assert!(listing.contains("Alice"), "listing: {listing}");
        assert!(listing.contains("Bob"), "listing: {listing}");
        assert!(listing.contains("Carol"), "listing: {listing}");
        assert!(listing.contains("30"), "listing: {listing}");
        assert!(listing.contains("25"), "listing: {listing}");
        assert!(listing.contains("40"), "listing: {listing}");
        // age column header before name (column order honored).
        let i_age = listing.find("age").unwrap();
        let i_name = listing.find("name").unwrap();
        assert!(i_age < i_name, "age header should precede name: {listing}");
    }

    #[test]
    fn summary_report_group_sum_and_mean() {
        let mut session = make_session();
        let df = df![
            "region" => ["East", "East", "West"],
            "sales" => [10.0_f64, 30.0, 100.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("region"), num_meta("sales")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = ReportAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            noheader: false,
            columns: Some(vec!["region".into(), "sales".into()]),
            defines: vec![
                Define {
                    var: "region".into(),
                    usage: Usage::Group,
                    order: OrderDir::Ascending,
                    label: None,
                },
                Define {
                    var: "sales".into(),
                    usage: Usage::Analysis("sum".into()),
                    order: OrderDir::Ascending,
                    label: None,
                },
            ],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // East sum = 40, West sum = 100. Two group rows.
        assert!(listing.contains("East"), "listing: {listing}");
        assert!(listing.contains("West"), "listing: {listing}");
        assert!(listing.contains("40"), "East total 40: {listing}");
        assert!(listing.contains("100"), "West total 100: {listing}");
    }

    #[test]
    fn summary_report_mean_stat() {
        let mut session = make_session();
        let df = df![
            "g" => ["a", "a", "b"],
            "x" => [2.0_f64, 4.0, 9.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("g"), num_meta("x")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = ReportAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            noheader: false,
            columns: Some(vec!["g".into(), "x".into()]),
            defines: vec![
                Define {
                    var: "g".into(),
                    usage: Usage::Group,
                    order: OrderDir::Ascending,
                    label: None,
                },
                Define {
                    var: "x".into(),
                    usage: Usage::Analysis("mean".into()),
                    order: OrderDir::Ascending,
                    label: None,
                },
            ],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // group a mean = 3, group b mean = 9.
        assert!(listing.contains("3"), "a mean 3: {listing}");
        assert!(listing.contains("9"), "b mean 9: {listing}");
    }

    #[test]
    fn order_keeps_distinct_rows_group_collapses() {
        // ORDER variable with one analysis column: each distinct value of the
        // order var produces one row, identical to GROUP for a key tuple.
        let mut session = make_session();
        let df = df![
            "k" => [1.0_f64, 1.0, 2.0],
            "v" => [5.0_f64, 7.0, 11.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("k"), num_meta("v")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = ReportAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            noheader: false,
            columns: Some(vec!["k".into(), "v".into()]),
            defines: vec![
                Define {
                    var: "k".into(),
                    usage: Usage::Order,
                    order: OrderDir::Ascending,
                    label: None,
                },
                Define {
                    var: "v".into(),
                    usage: Usage::Analysis("sum".into()),
                    order: OrderDir::Ascending,
                    label: None,
                },
            ],
        };
        execute(&ast, &mut session).unwrap();

        let listing = session.listing.into_string();
        // k=1 → sum 12, k=2 → sum 11. Two rows.
        assert!(listing.contains("12"), "k=1 sum 12: {listing}");
        assert!(listing.contains("11"), "k=2 sum 11: {listing}");
    }

    #[test]
    fn define_label_appears_and_noheader_suppresses() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64, 2.0]].unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x")],
        };
        write_dataset(&mut session, "T", ds);

        // With label, no group → detail report. Header shows label.
        let ast = ReportAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            noheader: false,
            columns: Some(vec!["x".into()]),
            defines: vec![Define {
                var: "x".into(),
                usage: Usage::Display,
                order: OrderDir::Ascending,
                label: Some("My X Label".into()),
            }],
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("My X Label"), "label header: {listing}");

        // Now noheader: label must NOT appear.
        let mut session2 = make_session();
        let df2 = df!["x" => [1.0_f64, 2.0]].unwrap();
        let ds2 = SasDataset {
            df: df2,
            vars: vec![num_meta("x")],
        };
        write_dataset(&mut session2, "T", ds2);
        let ast2 = ReportAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            noheader: true,
            columns: Some(vec!["x".into()]),
            defines: vec![Define {
                var: "x".into(),
                usage: Usage::Display,
                order: OrderDir::Ascending,
                label: Some("My X Label".into()),
            }],
        };
        execute(&ast2, &mut session2).unwrap();
        let listing2 = session2.listing.into_string();
        assert!(
            !listing2.contains("My X Label"),
            "noheader must suppress label: {listing2}"
        );
        // Data still present.
        assert!(listing2.contains('1'), "data present: {listing2}");
    }

    #[test]
    fn default_usages_numeric_analysis_char_display() {
        // No defines: numeric → ANALYSIS SUM, char → DISPLAY. Because there's
        // no group/order, this is a DETAIL report (raw per-row values).
        let mut session = make_session();
        let df = df![
            "name" => ["A", "B"],
            "n" => [3.0_f64, 4.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("name"), num_meta("n")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = ReportAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            noheader: false,
            columns: None,
            defines: vec![],
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // Detail report → both raw rows present.
        assert!(listing.contains("A"), "listing: {listing}");
        assert!(listing.contains("B"), "listing: {listing}");
        assert!(listing.contains('3'), "listing: {listing}");
        assert!(listing.contains('4'), "listing: {listing}");
    }

    #[test]
    fn missing_values_excluded_from_group_mean() {
        let mut session = make_session();
        let df = df![
            "g" => ["a", "a", "a"],
            "x" => [Some(2.0_f64), None, Some(4.0)]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("g"), num_meta("x")],
        };
        write_dataset(&mut session, "T", ds);

        let ast = ReportAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            noheader: false,
            columns: Some(vec!["g".into(), "x".into()]),
            defines: vec![
                Define {
                    var: "g".into(),
                    usage: Usage::Group,
                    order: OrderDir::Ascending,
                    label: None,
                },
                Define {
                    var: "x".into(),
                    usage: Usage::Analysis("mean".into()),
                    order: OrderDir::Ascending,
                    label: None,
                },
            ],
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // mean over non-missing [2,4] = 3 (NOT (2+4)/3).
        assert!(listing.contains('3'), "mean excludes missing: {listing}");
    }

    #[test]
    fn no_last_dataset_errors() {
        let mut session = make_session();
        let ast = ReportAst {
            data: None,
            noheader: false,
            columns: None,
            defines: vec![],
        };
        let r = execute(&ast, &mut session);
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("_LAST_"));
    }

    #[test]
    fn unknown_column_errors() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64]].unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x")],
        };
        write_dataset(&mut session, "T", ds);
        let ast = ReportAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            noheader: false,
            columns: Some(vec!["nope".into()]),
            defines: vec![],
        };
        let r = execute(&ast, &mut session);
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("NOPE"));
    }

    #[test]
    fn report_does_not_set_last_dataset() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64]].unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x")],
        };
        write_dataset(&mut session, "T", ds);
        // last_dataset is WORK.T after write.
        let before = session.last_dataset.clone();
        let ast = ReportAst {
            data: Some(DatasetRef {
                libref: Some("WORK".into()),
                name: "T".into(),
            }),
            noheader: false,
            columns: None,
            defines: vec![],
        };
        execute(&ast, &mut session).unwrap();
        // REPORT must not change last_dataset.
        assert_eq!(session.last_dataset, before);
    }
}
