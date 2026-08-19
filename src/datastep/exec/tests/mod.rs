use super::*;
use crate::datastep::compile;
use crate::parser::StatementStream;
use crate::source::SourceFile;
use polars::df;
use std::path::PathBuf;

fn session() -> Session {
    Session::new(None, PathBuf::from("."), true).unwrap()
}

fn write_class(session: &Session, table: &str) {
    let df = df!(
        "Age" => [Some(14.0), None, Some(13.0)],
        "Name" => ["Alfred", "Alice", "Barbara"],
    )
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "Age".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
        VarMeta {
            name: "Name".into(),
            ty: VarType::Char,
            length: 7,
            format: None,
            label: None,
        },
    ];
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(table, &SasDataset { df, vars })
        .unwrap();
}

fn run(src: &str, session: &mut Session) -> Result<StepStats> {
    let file = SourceFile::new(src);
    let mut ts = StatementStream::new(&file).unwrap();
    assert!(ts.next().is_kw("data"));
    let ast = crate::parser::datastep::parse_data_step(&mut ts).unwrap();
    let prog = compile(&ast, session)?;
    execute(prog, session)
}

fn read_work(session: &Session, table: &str) -> SasDataset {
    session.libs.get("WORK").unwrap().read(table).unwrap().0
}

// ── DO itératif / DELETE (M2) ────────────────────────────────────────

/// Lit la valeur f64 de la colonne `col`, ligne 0, de WORK.`table`.
fn num_at(session: &Session, table: &str, col: &str, row: usize) -> Option<f64> {
    read_work(session, table)
        .df
        .column(col)
        .unwrap()
        .f64()
        .unwrap()
        .get(row)
}

// ── Options de dataset + OUTPUT ciblé (M2, lot 4) ────────────────────

/// Mini-CLASS à trois variables (Name char, Sex char, Age num) pour les
/// tests d'options de dataset et de sorties multiples.
fn write_class_full(session: &Session, table: &str) {
    let df = df!(
        "Name" => ["Alfred", "Alice", "Barbara", "Henry"],
        "Sex" => ["M", "F", "F", "M"],
        "Age" => [Some(14.0), None, Some(13.0), Some(15.0)],
    )
    .unwrap();
    let vars = vec![
        VarMeta {
            name: "Name".into(),
            ty: VarType::Char,
            length: 7,
            format: None,
            label: None,
        },
        VarMeta {
            name: "Sex".into(),
            ty: VarType::Char,
            length: 1,
            format: None,
            label: None,
        },
        VarMeta {
            name: "Age".into(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        },
    ];
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(table, &SasDataset { df, vars })
        .unwrap();
}

// ── SET multi-datasets + BY + FIRST./LAST. (M3) ──────────────────────

/// Écrit un dataset entièrement numérique : colonnes (nom, valeurs).
fn write_num_ds(session: &Session, table: &str, cols: &[(&str, Vec<Option<f64>>)]) {
    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();
    for (name, vals) in cols {
        columns.push(Series::new((*name).into(), vals.clone()).into());
        vars.push(VarMeta {
            name: (*name).to_string(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        });
    }
    let df = DataFrame::new(columns).unwrap();
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(table, &SasDataset { df, vars })
        .unwrap();
}

fn some(vals: &[f64]) -> Vec<Option<f64>> {
    vals.iter().copied().map(Some).collect()
}

/// Colonne f64 complète de WORK.`table`.
fn col(session: &Session, table: &str, col: &str) -> Vec<Option<f64>> {
    read_work(session, table)
        .df
        .column(col)
        .unwrap()
        .f64()
        .unwrap()
        .iter()
        .collect()
}

// ── MERGE avec BY : match-merge SAS (M3-3) ───────────────────────────
//
// Plusieurs sorties sont COMPARÉES À UNE SORTIE SAS CALCULÉE À LA MAIN
// (indiqué dans le commentaire de chaque test).

/// Colonne char complète de WORK.`table`.
fn col_str(session: &Session, table: &str, col: &str) -> Vec<Option<String>> {
    read_work(session, table)
        .df
        .column(col)
        .unwrap()
        .str()
        .unwrap()
        .iter()
        .map(|o| o.map(str::to_string))
        .collect()
}

// ── FILE / PUT (M14.2) ───────────────────────────────────────────────

/// Extrait les lignes PUT du log (celles qui ne sont ni vides, ni un
/// écho de source numéroté, ni une NOTE/WARNING/ERROR).
fn put_log_lines(log: &str) -> Vec<String> {
    // L'écho de source SAS est de la forme "<num>     <texte>" : un nombre
    // suivi d'AU MOINS deux espaces (padding à la colonne 6) puis du texte.
    // Une ligne PUT purement numérique ("42") n'a pas ce padding.
    fn is_source_echo(l: &str) -> bool {
        let mut it = l.char_indices();
        let mut end = 0;
        for (i, c) in it.by_ref() {
            if c.is_ascii_digit() {
                end = i + 1;
            } else {
                break;
            }
        }
        if end == 0 {
            return false;
        }
        // Au moins deux espaces après le nombre.
        l[end..].starts_with("  ")
    }
    log.lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.is_empty()
                && !t.starts_with("NOTE:")
                && !t.starts_with("WARNING:")
                && !t.starts_with("ERROR:")
                && !is_source_echo(l)
                // Les continuations de NOTE timing ("real time...").
                && !t.starts_with("real time")
                && !t.starts_with("cpu time")
        })
        .map(|l| l.to_string())
        .collect()
}

// =====================================================================
// M15.6 — CALL routines
// =====================================================================

fn num_col(ds: &SasDataset, name: &str) -> Vec<Option<f64>> {
    let c = ds.df.column(name).unwrap().f64().unwrap();
    (0..ds.n_obs()).map(|i| c.get(i)).collect()
}

fn str_col(ds: &SasDataset, name: &str) -> Vec<String> {
    let c = ds.df.column(name).unwrap().str().unwrap();
    (0..ds.n_obs())
        .map(|i| c.get(i).unwrap_or("").to_string())
        .collect()
}

// ── M16.4 : SET options END= / NOBS= / POINT= + multi-datasets ────────

fn run_err(src: &str, session: &mut Session) -> String {
    let file = SourceFile::new(src);
    let mut ts = StatementStream::new(&file).unwrap();
    assert!(ts.next().is_kw("data"));
    let ast = crate::parser::datastep::parse_data_step(&mut ts).unwrap();
    match compile(&ast, session).and_then(|p| execute(p, session)) {
        Ok(_) => panic!("expected an error, got Ok"),
        Err(e) => e.to_string(),
    }
}

// ── M16.5 : UPDATE / MODIFY ──────────────────────────────────────────

/// Écrit un dataset avec une colonne char `key` et des colonnes num.
/// `keys` = valeurs de la clé char ; `cols` = (nom, valeurs num).
fn write_keyed_ds(
    session: &Session,
    table: &str,
    key: &str,
    keys: &[&str],
    cols: &[(&str, Vec<Option<f64>>)],
) {
    let mut columns: Vec<Column> = Vec::new();
    let mut vars: Vec<VarMeta> = Vec::new();
    columns.push(Series::new(key.into(), keys.to_vec()).into());
    vars.push(VarMeta {
        name: key.to_string(),
        ty: VarType::Char,
        length: 8,
        format: None,
        label: None,
    });
    for (name, vals) in cols {
        columns.push(Series::new((*name).into(), vals.clone()).into());
        vars.push(VarMeta {
            name: (*name).to_string(),
            ty: VarType::Num,
            length: 8,
            format: None,
            label: None,
        });
    }
    let df = DataFrame::new(columns).unwrap();
    session
        .libs
        .get("WORK")
        .unwrap()
        .write(table, &SasDataset { df, vars })
        .unwrap();
}

mod array;
mod end;
mod file;
mod hash1;
mod hash2;
mod invalid;
mod link;
mod multiple;
mod prx;
mod retain;
mod select;
mod set;
mod update_modify;
mod where_informat;
mod where_option;
