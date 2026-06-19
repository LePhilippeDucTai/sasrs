//! PROC DISTANCE — distance/dissimilarity matrix between observations (M27).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc distance data=<ref> out=<ref> [method=euclid|L2|cityblock|L1|Linf|
//!  Chebychev|cosine|corr]; var <name list>; run;`
//!
//! ## Périmètre
//! - `data=`, `out=` (matrice stockée), `method=` (défaut EUCLID).
//! - `var` : variables numériques formant les coordonnées de chaque obs.
//! - `out=` absent : NOTE "No output dataset specified ..." + listing affiché.
//! - Différé : `SHAPE=`, `FREQ`, normalisation, `id=`.
//!
//! ## Sortie
//! - Listing : matrice n×n (n = nombre d'observations), 4 décimales, lignes/
//!   colonnes Row<i>/Col<j>.
//! - `out=` : dataset avec `_TYPE_`="DISTANCE", `_NAME_`=Row<i>, puis Col1..Coln.

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::decode_column;
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::VarType;
use polars::prelude::*;

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistMethod {
    Euclid,
    CityBlock,
    Chebychev,
    Cosine,
    Corr,
}

impl DistMethod {
    fn title(self) -> &'static str {
        match self {
            DistMethod::Euclid => "Euclidean",
            DistMethod::CityBlock => "City Block (L1)",
            DistMethod::Chebychev => "Chebychev (Linf)",
            DistMethod::Cosine => "Cosine",
            DistMethod::Corr => "Correlation",
        }
    }
}

pub struct DistanceAst {
    pub data: Option<DatasetRef>,
    pub out: Option<DatasetRef>,
    pub method: DistMethod,
    pub var: Vec<String>,
}

// ───────────────────────── Parser ─────────────────────────

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

fn parse_method(ts: &mut StatementStream) -> Result<DistMethod> {
    let span = ts.peek().span;
    let name = ts
        .peek()
        .ident()
        .map(str::to_string)
        .ok_or_else(|| SasError::parse("expected a method name after METHOD=", span))?;
    ts.next();
    match name.to_ascii_lowercase().as_str() {
        "euclid" | "euclidean" | "l2" => Ok(DistMethod::Euclid),
        "cityblock" | "l1" => Ok(DistMethod::CityBlock),
        "linf" | "chebychev" | "chebyshev" => Ok(DistMethod::Chebychev),
        "cosine" => Ok(DistMethod::Cosine),
        "corr" | "correlation" => Ok(DistMethod::Corr),
        other => Err(SasError::parse(
            format!("Unknown METHOD= value '{}' on PROC DISTANCE.", other.to_uppercase()),
            span,
        )),
    }
}

/// Parse `proc distance [data=a] [out=b] [method=m]; [var ...;] run;`.
/// Called AFTER "proc distance" has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<DistanceAst> {
    let mut data: Option<DatasetRef> = None;
    let mut out: Option<DatasetRef> = None;
    let mut method = DistMethod::Euclid;

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
        } else if ts.peek().is_kw("out") {
            ts.next();
            expect_eq(ts, "OUT")?;
            out = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("method") {
            ts.next();
            expect_eq(ts, "METHOD")?;
            method = parse_method(ts)?;
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!(
                    "Unexpected option '{}' on PROC DISTANCE statement.",
                    name.to_uppercase()
                ),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC DISTANCE statement.",
                span,
            ));
        }
    }

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
        if ts.peek().is_kw("var") {
            ts.next();
            var = ts.parse_name_list()?;
            ts.expect_semi()?;
        } else {
            ts.skip_to_semi();
        }
    }

    Ok(DistanceAst {
        data,
        out,
        method,
        var,
    })
}

fn resolve_input(data: &Option<DatasetRef>, session: &Session) -> Result<DatasetRef> {
    match data {
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

/// Distance between two coordinate vectors under the given method.
pub fn distance(method: DistMethod, a: &[f64], b: &[f64]) -> f64 {
    match method {
        DistMethod::Euclid => a
            .iter()
            .zip(b)
            .map(|(x, y)| (x - y) * (x - y))
            .sum::<f64>()
            .sqrt(),
        DistMethod::CityBlock => a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum(),
        DistMethod::Chebychev => a
            .iter()
            .zip(b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0_f64, f64::max),
        DistMethod::Cosine => {
            let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
            let na: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
            let nb: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
            if na == 0.0 || nb == 0.0 {
                0.0
            } else {
                1.0 - dot / (na * nb)
            }
        }
        DistMethod::Corr => {
            let p = a.len() as f64;
            if p < 2.0 {
                return 0.0;
            }
            let ma = a.iter().sum::<f64>() / p;
            let mb = b.iter().sum::<f64>() / p;
            let mut sab = 0.0;
            let mut saa = 0.0;
            let mut sbb = 0.0;
            for (x, y) in a.iter().zip(b) {
                let dx = x - ma;
                let dy = y - mb;
                sab += dx * dy;
                saa += dx * dx;
                sbb += dy * dy;
            }
            if saa == 0.0 || sbb == 0.0 {
                0.0
            } else {
                1.0 - sab / (saa.sqrt() * sbb.sqrt())
            }
        }
    }
}

// ───────────────────────── execute ─────────────────────────

pub fn execute(ast: &DistanceAst, session: &mut Session) -> Result<()> {
    if ast.var.is_empty() {
        return Err(SasError::runtime("PROC DISTANCE requires a VAR statement."));
    }

    let in_ref = resolve_input(&ast.data, session)?;
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

    // Resolve VAR columns (numeric only).
    let mut cols: Vec<usize> = Vec::with_capacity(ast.var.len());
    for nm in &ast.var {
        match ds.vars.iter().position(|m| m.name.eq_ignore_ascii_case(nm)) {
            Some(i) if ds.vars[i].ty == VarType::Num => cols.push(i),
            _ => {
                return Err(SasError::runtime(format!(
                    "Variable '{}' not found in dataset '{}'.",
                    nm, display
                )))
            }
        }
    }
    let p = cols.len();

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

    // One coordinate vector per observation.
    let coords: Vec<Vec<f64>> = (0..n_read)
        .map(|r| decoded.iter().map(|col| col[r]).collect())
        .collect();
    let n = coords.len();

    // Symmetric n×n distance matrix.
    let mut dist = vec![vec![0.0_f64; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let d = distance(ast.method, &coords[i], &coords[j]);
            dist[i][j] = d;
            dist[j][i] = d;
        }
    }

    // ───────────────────────── listing ─────────────────────────
    session.listing.page_header();
    centered(session, "The DISTANCE Procedure");
    session.listing.blank();
    centered(session, &format!("Distance Method: {}", ast.method.title()));
    session.listing.blank();
    centered(session, &format!("N = {}    Variables = {}", n, p));
    session.listing.blank();
    centered(session, "Distance Matrix");
    session.listing.blank();
    {
        let mut headers: Vec<String> = vec![String::new()];
        let mut aligns: Vec<Align> = vec![Align::Left];
        for j in 0..n {
            headers.push(format!("Col{}", j + 1));
            aligns.push(Align::Right);
        }
        let mut rows: Vec<Vec<String>> = Vec::with_capacity(n);
        for i in 0..n {
            let mut row = vec![format!("Row{}", i + 1)];
            for j in 0..n {
                row.push(format!("{:.4}", dist[i][j]));
            }
            rows.push(row);
        }
        session.listing.write_table(&headers, &aligns, &rows);
        session.listing.blank();
    }

    // ───────────────────────── out= ─────────────────────────
    match &ast.out {
        None => {
            session
                .log
                .note("No output dataset specified for PROC DISTANCE, results not stored.");
        }
        Some(out_ref) => {
            let mut columns: Vec<Column> = Vec::with_capacity(n + 2);
            let mut vars: Vec<crate::dataset::VarMeta> = Vec::with_capacity(n + 2);

            // _TYPE_ : "DISTANCE" for every row.
            let type_vals: Vec<&str> = vec!["DISTANCE"; n];
            columns.push(Series::new("_TYPE_".into(), type_vals).into());
            vars.push(char_var_meta("_TYPE_", "DISTANCE".len()));

            // _NAME_ : Row<i>.
            let name_vals: Vec<String> = (0..n).map(|i| format!("Row{}", i + 1)).collect();
            let name_len = name_vals.iter().map(|s| s.len()).max().unwrap_or(1);
            columns.push(
                Series::new(
                    "_NAME_".into(),
                    name_vals.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                )
                .into(),
            );
            vars.push(char_var_meta("_NAME_", name_len));

            // Col1..Coln : distances.
            for j in 0..n {
                let col_name = format!("Col{}", j + 1);
                let vals: Vec<f64> = (0..n).map(|i| dist[i][j]).collect();
                columns.push(Series::new(col_name.as_str().into(), vals).into());
                vars.push(num_var_meta(&col_name));
            }

            let df = DataFrame::new(columns)?;
            let out_ds = SasDataset { df, vars };

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
        }
    }

    Ok(())
}

fn num_var_meta(name: &str) -> crate::dataset::VarMeta {
    crate::dataset::VarMeta {
        name: name.to_string(),
        ty: VarType::Num,
        length: 8,
        format: None,
        label: None,
    }
}

fn char_var_meta(name: &str, len: usize) -> crate::dataset::VarMeta {
    crate::dataset::VarMeta {
        name: name.to_string(),
        ty: VarType::Char,
        length: len.max(1),
        format: None,
        label: None,
    }
}

use crate::dataset::SasDataset;

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

    fn parse_distance(src: &str) -> Result<DistanceAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "distance"
        parse(&mut ts)
    }

    #[test]
    fn parse_minimal() {
        let ast = parse_distance("proc distance data=a out=b; var x y; run;").unwrap();
        assert_eq!(ast.data.as_ref().unwrap().name, "a");
        assert_eq!(ast.out.as_ref().unwrap().name, "b");
        assert_eq!(ast.method, DistMethod::Euclid);
        assert_eq!(ast.var, vec!["x", "y"]);
    }

    #[test]
    fn parse_methods() {
        assert_eq!(
            parse_distance("proc distance method=cityblock; var x; run;").unwrap().method,
            DistMethod::CityBlock
        );
        assert_eq!(
            parse_distance("proc distance method=L2; var x; run;").unwrap().method,
            DistMethod::Euclid
        );
        assert_eq!(
            parse_distance("proc distance method=chebychev; var x; run;").unwrap().method,
            DistMethod::Chebychev
        );
        assert_eq!(
            parse_distance("proc distance method=cosine; var x; run;").unwrap().method,
            DistMethod::Cosine
        );
    }

    /// Oracle: 3 points in 3-space — x=(0,1,0), y=(0,0,1), z=(1,0,0).
    /// All pairwise Euclidean distances = sqrt(2). 3×3 matrix, zero diagonal.
    #[test]
    fn euclid_three_points_oracle() {
        let x = [0.0, 0.0, 1.0];
        let y = [1.0, 0.0, 0.0];
        let z = [0.0, 1.0, 0.0];
        let s2 = 2.0_f64.sqrt();
        assert!((distance(DistMethod::Euclid, &x, &y) - s2).abs() < 1e-12);
        assert!((distance(DistMethod::Euclid, &x, &z) - s2).abs() < 1e-12);
        assert!((distance(DistMethod::Euclid, &y, &z) - s2).abs() < 1e-12);
        assert!(distance(DistMethod::Euclid, &x, &x).abs() < 1e-12);
    }

    #[test]
    fn cityblock_and_chebychev_oracle() {
        let a = [1.0, 2.0, 3.0];
        let b = [4.0, 0.0, 3.0];
        // L1 = |1-4|+|2-0|+|3-3| = 3+2+0 = 5
        assert!((distance(DistMethod::CityBlock, &a, &b) - 5.0).abs() < 1e-12);
        // Linf = max(3,2,0) = 3
        assert!((distance(DistMethod::Chebychev, &a, &b) - 3.0).abs() < 1e-12);
    }

    /// 1D fixture: x=(1,2,3,7,8,9), out= dataset stores the 6×6 matrix.
    #[test]
    fn execute_writes_out_dataset() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64, 2.0, 3.0, 7.0, 8.0, 9.0]].unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x")],
        };
        write_dataset(&mut session, "PTS", ds);

        let ast = DistanceAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "PTS".into() }),
            out: Some(DatasetRef { libref: Some("WORK".into()), name: "DIST".into() }),
            method: DistMethod::Euclid,
            var: vec!["x".into()],
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("DIST").unwrap();
        assert_eq!(out.n_obs(), 6);
        // _TYPE_, _NAME_, Col1..Col6 = 8 variables.
        assert_eq!(out.vars.len(), 8);
        // Col6 distance for Row1 (x=1) is |1-9| = 8.
        let col6 = out.df.column("Col6").unwrap().f64().unwrap();
        assert_eq!(col6.get(0), Some(8.0));
        // Diagonal: Row3 vs Col3 = 0.
        let col3 = out.df.column("Col3").unwrap().f64().unwrap();
        assert_eq!(col3.get(2), Some(0.0));
    }

    #[test]
    fn execute_no_out_emits_note() {
        let mut session = make_session();
        let df = df!["x" => [1.0_f64, 2.0, 3.0]].unwrap();
        let ds = SasDataset {
            df,
            vars: vec![num_meta("x")],
        };
        write_dataset(&mut session, "PTS", ds);

        let ast = DistanceAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "PTS".into() }),
            out: None,
            method: DistMethod::Euclid,
            var: vec!["x".into()],
        };
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(
            log.contains("No output dataset specified for PROC DISTANCE"),
            "{log}"
        );
    }
}
