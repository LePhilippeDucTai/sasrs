//! PROC SORT (jalon M3).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc sort data=a [out=b] [nodupkey] [noduprecs] ; by [descending] v1
//! [descending] v2... ; run ;`
//!
//! ## Piège central : la collation SAS des missings
//! Ordre numérique SAS : `._ < . < .A < ... < .Z < nombres`. Les flags
//! nulls_first/last de Polars ne connaissent pas les missings spéciaux
//! (NaN-payload). L'en-tête historique proposait une colonne de rang i8
//! compagnon. Or `Value::sas_cmp` ENCODE DÉJÀ cette collation (cf.
//! `value.rs` : `._ < . < .A.. < nombres`, et `. = .` est vrai). On
//! choisit donc la voie recommandée par PLAN.md : décoder chaque colonne
//! clé UNE fois en `Vec<Value>` (downcast par colonne, jamais par cellule
//! — checklist point 3), construire un vecteur d'indices `0..n` et le
//! trier de façon STABLE avec un comparateur qui applique `sas_cmp` clé
//! par clé (inversé pour DESCENDING). La colonne de rang devient inutile.
//! Le réordonnancement final se fait en UN SEUL `df.take` sur toutes les
//! colonnes.
//!
//! ## Collation caractère
//! `sas_cmp` compare les chaînes via `trim_end()` (les blancs de fin sont
//! ignorés, comme SAS qui padde l'opérande le plus court). C'est fidèle à
//! SAS et indépendant du padding de stockage. DESCENDING inverse l'ordre
//! de la clé concernée.
//!
//! - NODUPKEY : déduplication sur les clés BY après tri (garder la
//!   première), NOTE "N observations with duplicate key values were
//!   deleted." ; NODUPRECS (alias NODUP) compare la LIGNE ENTIÈRE.
//! - out= absent → remplace le dataset d'entrée.
//! - Conserver les VarMeta du dataset d'entrée à l'identique.

use crate::ast::DatasetRef;
use crate::dataset::SasDataset;
use crate::error::{Result, SasError};
use crate::missing::num_to_value;
use crate::parser::StatementStream;
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{Value, VarType};
use polars::prelude::*;
use std::cmp::Ordering;

pub struct SortAst {
    pub data: Option<DatasetRef>,
    pub out: Option<DatasetRef>,
    pub by: Vec<(String, bool)>, // (var, descending)
    pub nodupkey: bool,
    pub noduprecs: bool,
}

/// Parse `proc sort [data=a] [out=b] [nodupkey] [noduprecs|nodup] ;
/// by [descending] v1 [descending] v2... ; run ;`. Called AFTER
/// "proc sort" has been consumed. Consumes through `run;` / `quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<SortAst> {
    let mut data: Option<DatasetRef> = None;
    let mut out: Option<DatasetRef> = None;
    let mut nodupkey = false;
    let mut noduprecs = false;

    // --- PROC SORT statement options, until `;` ---
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
        } else if ts.peek().is_kw("nodupkey") {
            ts.next();
            nodupkey = true;
        } else if ts.peek().is_kw("noduprecs") || ts.peek().is_kw("nodup") {
            ts.next();
            noduprecs = true;
        } else {
            let span = ts.peek().span;
            let bad = ts.peek().ident().unwrap_or("?").to_uppercase();
            return Err(SasError::parse(
                format!("Unexpected option '{bad}' on PROC SORT statement."),
                span,
            ));
        }
    }

    // --- sub-statements : BY (mandatory) until run;/quit; ---
    let mut by: Vec<(String, bool)> = Vec::new();
    let mut saw_by = false;

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
            ts.next(); // consume "by"
            saw_by = true;
            // Parse [descending] var pairs until `;`.
            loop {
                if ts.peek().kind == TokenKind::Semi {
                    ts.next();
                    break;
                }
                if ts.peek().kind == TokenKind::Eof {
                    break;
                }
                let descending = if ts.peek().is_kw("descending") {
                    ts.next();
                    true
                } else {
                    false
                };
                let tok = ts.peek().clone();
                match tok.ident() {
                    Some(name) => {
                        ts.next();
                        by.push((name.to_string(), descending));
                    }
                    None => {
                        return Err(SasError::parse(
                            "expected a variable name in the BY statement",
                            tok.span,
                        ));
                    }
                }
            }
        } else {
            // Unknown sub-statement: skip it (recovery).
            ts.skip_to_semi();
        }
    }

    if !saw_by {
        return Err(SasError::runtime(
            "No BY statement was specified for PROC SORT.",
        ));
    }
    if by.is_empty() {
        return Err(SasError::runtime(
            "The BY statement must specify at least one variable.",
        ));
    }

    Ok(SortAst {
        data,
        out,
        by,
        nodupkey,
        noduprecs,
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
fn resolve_input(ast: &SortAst, session: &Session) -> Result<DatasetRef> {
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

/// Decode one column of a SasDataset into a `Vec<Value>` (downcast once).
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

/// Execute PROC SORT. Called by `procs::execute_proc` which wraps with timing.
pub fn execute(ast: &SortAst, session: &mut Session) -> Result<()> {
    let in_ref = resolve_input(ast, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }

    let n_obs = ds.n_obs();

    // Resolve BY variable column indices (validate existence).
    // (n_obs used below to size the index vector.)
    let mut key_cols: Vec<usize> = Vec::with_capacity(ast.by.len());
    for (vname, _) in &ast.by {
        let idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(vname));
        match idx {
            Some(i) => key_cols.push(i),
            None => {
                return Err(SasError::runtime(format!(
                    "Variable {} not found.",
                    vname.to_uppercase()
                )));
            }
        }
    }

    // Decode each key column ONCE into Vec<Value>.
    let key_values: Vec<Vec<Value>> = key_cols
        .iter()
        .map(|&ci| decode_column(&ds, ci))
        .collect::<Result<_>>()?;
    let descending: Vec<bool> = ast.by.iter().map(|(_, d)| *d).collect();

    // Stable sort of an index vector via sas_cmp, key by key.
    // sas_cmp already encodes the SAS missing collation (`._ < . < .A.. <
    // numbers`), so no companion rank column is needed.
    let mut order: Vec<usize> = (0..n_obs).collect();
    order.sort_by(|&a, &b| {
        for (k, key) in key_values.iter().enumerate() {
            let mut c = key[a].sas_cmp(&key[b]);
            if descending[k] {
                c = c.reverse();
            }
            if c != Ordering::Equal {
                return c;
            }
        }
        Ordering::Equal
    });

    // Reorder ALL columns in a single take.
    let idx_ca = IdxCa::from_vec(
        "".into(),
        order.iter().map(|&i| i as IdxSize).collect::<Vec<_>>(),
    );
    let sorted_df = ds.df.take(&idx_ca)?;

    // Build the post-sort dataset (keep input VarMeta verbatim).
    let mut result = SasDataset {
        df: sorted_df,
        vars: ds.vars.clone(),
    };

    // NODUPKEY / NODUPRECS deduplication (after sort).
    let mut deleted = 0usize;
    if ast.nodupkey || ast.noduprecs {
        // Re-decode the sorted columns we need for comparison.
        let cmp_cols: Vec<usize> = if ast.noduprecs {
            (0..result.vars.len()).collect()
        } else {
            key_cols.clone()
        };
        let cols: Vec<Vec<Value>> = cmp_cols
            .iter()
            .map(|&ci| decode_column(&result, ci))
            .collect::<Result<_>>()?;

        let n = result.n_obs();
        let mut keep: Vec<IdxSize> = Vec::with_capacity(n);
        for row in 0..n {
            let dup = row > 0
                && cols.iter().all(|c| {
                    let prev = row - 1;
                    c[prev].sas_cmp(&c[row]) == Ordering::Equal
                });
            if dup {
                deleted += 1;
            } else {
                keep.push(row as IdxSize);
            }
        }
        if deleted > 0 {
            let keep_ca = IdxCa::from_vec("".into(), keep);
            result.df = result.df.take(&keep_ca)?;
        }
    }

    // Determine output destination.
    let out_ref = ast.out.clone().unwrap_or_else(|| in_ref.clone());
    let out_libref = out_ref.libref_or_work();
    let out_table = out_ref.name.to_uppercase();

    let out_provider = session.libs.get(&out_libref)?;
    out_provider.write(&out_table, &result)?;
    session.last_dataset = Some(format!("{out_libref}.{out_table}"));

    // NOTE de déduplication (pluriel invariable).
    if deleted > 0 {
        if ast.nodupkey {
            session.log.note(&format!(
                "{deleted} observations with duplicate key values were deleted."
            ));
        } else {
            session.log.note(&format!(
                "{deleted} duplicate observations were deleted."
            ));
        }
    }

    // PROC SORT n'émet pas de NOTE "observations read" en SAS standard ;
    // execute_proc ajoute la NOTE de timing "PROCEDURE SORT used".
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::missing::encode_special;
    use crate::session::Session;
    use crate::source::SourceFile;
    use crate::value::{MissingKind, VarType};
    use polars::df;
    use std::path::PathBuf;

    fn make_session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    fn parse_sort(src: &str) -> Result<SortAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "sort"
        parse(&mut ts)
    }

    // --- parse tests ---

    #[test]
    fn parse_minimal_by() {
        let ast = parse_sort("proc sort data=a; by x; run;").unwrap();
        assert_eq!(ast.data.as_ref().unwrap().name, "a");
        assert!(ast.out.is_none());
        assert_eq!(ast.by, vec![("x".to_string(), false)]);
        assert!(!ast.nodupkey);
        assert!(!ast.noduprecs);
    }

    #[test]
    fn parse_out_and_nodupkey() {
        let ast = parse_sort("proc sort data=lib.a out=work.b nodupkey; by x; run;").unwrap();
        assert_eq!(ast.data.as_ref().unwrap().libref.as_deref(), Some("lib"));
        assert_eq!(ast.out.as_ref().unwrap().name, "b");
        assert!(ast.nodupkey);
    }

    #[test]
    fn parse_noduprecs_alias() {
        let a = parse_sort("proc sort data=a noduprecs; by x; run;").unwrap();
        assert!(a.noduprecs);
        let b = parse_sort("proc sort data=a nodup; by x; run;").unwrap();
        assert!(b.noduprecs);
    }

    #[test]
    fn parse_descending_multiple() {
        let ast =
            parse_sort("proc sort data=a; by descending x y descending z; run;").unwrap();
        assert_eq!(
            ast.by,
            vec![
                ("x".to_string(), true),
                ("y".to_string(), false),
                ("z".to_string(), true),
            ]
        );
    }

    #[test]
    fn parse_no_data_uses_last() {
        let ast = parse_sort("proc sort; by x; run;").unwrap();
        assert!(ast.data.is_none());
    }

    #[test]
    fn parse_missing_by_errors() {
        let result = parse_sort("proc sort data=a; run;");
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("BY"), "msg: {msg}");
    }

    #[test]
    fn parse_unknown_option_errors() {
        let result = parse_sort("proc sort data=a bogus; by x; run;");
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("BOGUS"), "msg: {msg}");
    }

    // --- execute tests ---

    fn write_num_dataset(session: &mut Session, table: &str, name: &str, xs: Vec<Option<f64>>) {
        let df = df![name => xs].unwrap();
        let vars = vec![VarMeta {
            name: name.to_string(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        }];
        let ds = SasDataset { df, vars };
        session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
        session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
    }

    fn read_num_col(session: &Session, table: &str, col: &str) -> Vec<Value> {
        let (ds, _) = session.libs.get("WORK").unwrap().read(table).unwrap();
        let idx = ds.vars.iter().position(|m| m.name == col).unwrap();
        decode_column(&ds, idx).unwrap()
    }

    #[test]
    fn execute_ascending_missing_collation() {
        // Mix special and ordinary missings with numbers:
        // ._ (underscore), . (dot=null), .A (letter 0), 5, 2.
        // Expected ascending order: ._ < . < .A < 2 < 5.
        let mut session = make_session();
        let xs = vec![
            Some(5.0),
            None,                                   // .
            Some(encode_special(MissingKind::Letter(0))), // .A
            Some(encode_special(MissingKind::Underscore)), // ._
            Some(2.0),
        ];
        write_num_dataset(&mut session, "T", "x", xs);

        let ast = SortAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            out: None,
            by: vec![("x".to_string(), false)],
            nodupkey: false,
            noduprecs: false,
        };
        execute(&ast, &mut session).unwrap();

        let got = read_num_col(&session, "T", "x");
        assert_eq!(
            got,
            vec![
                Value::Missing(MissingKind::Underscore),
                Value::Missing(MissingKind::Dot),
                Value::Missing(MissingKind::Letter(0)),
                Value::Num(2.0),
                Value::Num(5.0),
            ]
        );
    }

    #[test]
    fn execute_descending() {
        let mut session = make_session();
        write_num_dataset(
            &mut session,
            "T",
            "x",
            vec![Some(1.0), Some(3.0), Some(2.0)],
        );

        let ast = SortAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            out: None,
            by: vec![("x".to_string(), true)],
            nodupkey: false,
            noduprecs: false,
        };
        execute(&ast, &mut session).unwrap();

        let got = read_num_col(&session, "T", "x");
        assert_eq!(got, vec![Value::Num(3.0), Value::Num(2.0), Value::Num(1.0)]);
    }

    #[test]
    fn execute_multikey_num_then_char() {
        let mut session = make_session();
        let df = df![
            "g" => [2.0_f64, 1.0, 1.0, 2.0],
            "s" => ["b", "z", "a", "a"]
        ]
        .unwrap();
        let vars = vec![
            VarMeta { name: "g".into(), ty: VarType::Num, length: 8, format: None, label: None },
            VarMeta { name: "s".into(), ty: VarType::Char, length: 1, format: None, label: None },
        ];
        let ds = SasDataset { df, vars };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

        let ast = SortAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            out: None,
            by: vec![("g".to_string(), false), ("s".to_string(), false)],
            nodupkey: false,
            noduprecs: false,
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("T").unwrap();
        let g: Vec<f64> = out.df.column("g").unwrap().f64().unwrap().into_no_null_iter().collect();
        let s: Vec<String> = out
            .df
            .column("s")
            .unwrap()
            .str()
            .unwrap()
            .iter()
            .map(|o| o.unwrap().to_string())
            .collect();
        // (g=1,s=a),(g=1,s=z),(g=2,s=a),(g=2,s=b)
        assert_eq!(g, vec![1.0, 1.0, 2.0, 2.0]);
        assert_eq!(s, vec!["a", "z", "a", "b"]);
    }

    #[test]
    fn execute_nodupkey_deletes_and_notes() {
        let mut session = make_session();
        write_num_dataset(
            &mut session,
            "T",
            "x",
            vec![Some(1.0), Some(1.0), Some(2.0), Some(2.0), Some(2.0)],
        );

        let ast = SortAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            out: None,
            by: vec![("x".to_string(), false)],
            nodupkey: true,
            noduprecs: false,
        };
        execute(&ast, &mut session).unwrap();

        let got = read_num_col(&session, "T", "x");
        assert_eq!(got, vec![Value::Num(1.0), Value::Num(2.0)]);

        let log = session.log.into_string();
        assert!(
            log.contains("3 observations with duplicate key values were deleted."),
            "log: {log}"
        );
    }

    #[test]
    fn execute_noduprecs_whole_row() {
        // Same key but different other column => NODUPKEY would drop, but
        // NODUPRECS keeps (rows differ). Then a true full duplicate is dropped.
        let mut session = make_session();
        let df = df![
            "x" => [1.0_f64, 1.0, 1.0],
            "y" => ["a", "b", "b"]
        ]
        .unwrap();
        let vars = vec![
            VarMeta { name: "x".into(), ty: VarType::Num, length: 8, format: None, label: None },
            VarMeta { name: "y".into(), ty: VarType::Char, length: 1, format: None, label: None },
        ];
        let ds = SasDataset { df, vars };
        session.libs.get("WORK").unwrap().write("T", &ds).unwrap();

        let ast = SortAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            out: None,
            by: vec![("x".to_string(), false)],
            nodupkey: false,
            noduprecs: true,
        };
        execute(&ast, &mut session).unwrap();

        let (out, _) = session.libs.get("WORK").unwrap().read("T").unwrap();
        // After sort by x: rows (1,a),(1,b),(1,b) => drop the 3rd (full dup of 2nd).
        assert_eq!(out.n_obs(), 2);
        let log = session.log.into_string();
        assert!(log.contains("1 duplicate observations were deleted."), "log: {log}");
    }

    #[test]
    fn execute_out_creates_new_leaves_input() {
        let mut session = make_session();
        write_num_dataset(
            &mut session,
            "IN",
            "x",
            vec![Some(3.0), Some(1.0), Some(2.0)],
        );

        let ast = SortAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "IN".into() }),
            out: Some(DatasetRef { libref: Some("WORK".into()), name: "OUT".into() }),
            by: vec![("x".to_string(), false)],
            nodupkey: false,
            noduprecs: false,
        };
        execute(&ast, &mut session).unwrap();

        // OUT is sorted.
        let out = read_num_col(&session, "OUT", "x");
        assert_eq!(out, vec![Value::Num(1.0), Value::Num(2.0), Value::Num(3.0)]);
        // IN is untouched (original order).
        let inp = read_num_col(&session, "IN", "x");
        assert_eq!(inp, vec![Value::Num(3.0), Value::Num(1.0), Value::Num(2.0)]);
        // last_dataset points at OUT.
        assert_eq!(session.last_dataset.as_deref(), Some("WORK.OUT"));
    }

    #[test]
    fn execute_no_out_replaces_input() {
        let mut session = make_session();
        write_num_dataset(
            &mut session,
            "T",
            "x",
            vec![Some(3.0), Some(1.0), Some(2.0)],
        );

        let ast = SortAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            out: None,
            by: vec![("x".to_string(), false)],
            nodupkey: false,
            noduprecs: false,
        };
        execute(&ast, &mut session).unwrap();

        let got = read_num_col(&session, "T", "x");
        assert_eq!(got, vec![Value::Num(1.0), Value::Num(2.0), Value::Num(3.0)]);
    }

    #[test]
    fn execute_uses_last_when_no_data() {
        let mut session = make_session();
        write_num_dataset(
            &mut session,
            "LASTONE",
            "x",
            vec![Some(2.0), Some(1.0)],
        );
        // last_dataset = WORK.LASTONE set by helper.

        let ast = SortAst {
            data: None,
            out: None,
            by: vec![("x".to_string(), false)],
            nodupkey: false,
            noduprecs: false,
        };
        execute(&ast, &mut session).unwrap();

        let got = read_num_col(&session, "LASTONE", "x");
        assert_eq!(got, vec![Value::Num(1.0), Value::Num(2.0)]);
    }

    #[test]
    fn execute_unknown_by_var_errors() {
        let mut session = make_session();
        write_num_dataset(&mut session, "T", "x", vec![Some(1.0)]);

        let ast = SortAst {
            data: Some(DatasetRef { libref: Some("WORK".into()), name: "T".into() }),
            out: None,
            by: vec![("nope".to_string(), false)],
            nodupkey: false,
            noduprecs: false,
        };
        let result = execute(&ast, &mut session);
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("NOPE") && msg.contains("not found"), "msg: {msg}");
    }
}
