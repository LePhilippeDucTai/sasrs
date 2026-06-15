//! Dictionary tables pour PROC SQL (jalon M20.3).
//!
//! SAS expose des tables système en lecture seule décrivant l'état de la
//! session. PROC SQL les lit via le libref virtuel `DICTIONARY` ; le DATA step
//! et les autres procs via les vues `sashelp.v*`. M20.3 couvre :
//!
//! - `DICTIONARY.TABLES`  ≡ `sashelp.vtable`  : une ligne par dataset connu
//!   dans chaque bibliothèque assignée (colonnes `libname`, `memname`,
//!   `memtype`, `nobs`, `nvar`).
//! - `DICTIONARY.COLUMNS` ≡ `sashelp.vcolumn` : une ligne par variable de
//!   chaque dataset (`libname`, `memname`, `name`, `type`, `length`, `npos`,
//!   `varnum`, `label`, `format`, `informat`).
//! - `DICTIONARY.MACROS`  ≡ `sashelp.vmacro`  : une ligne par variable macro
//!   globale (`scope`, `name`, `value`).
//!
//! # Conventions de fidélité
//! - Noms de colonnes SAS dictionary en MINUSCULES.
//! - `type` = `'num'` / `'char'` (minuscules).
//! - `libname`/`memname` en MAJUSCULES (comme SAS les stocke).
//! - Le DataFrame généré rejoint le pipeline standard de `plan.rs` : WHERE /
//!   SELECT / ORDER BY s'y appliquent comme sur une table ordinaire. Les
//!   colonnes numériques sont des `Float64` (modèle SAS), donc
//!   `normalize_specials` les laisse intactes (pas de NaN-payload ici).

use crate::error::Result;
use crate::session::Session;
use polars::prelude::*;

/// Vrai si la référence `libref.name` désigne une dictionary table (à
/// matérialiser à la volée) plutôt qu'un dataset stocké. Reconnaît :
/// - libref `DICTIONARY` + membre `TABLES`/`COLUMNS`/`MACROS` ;
/// - libref `SASHELP` + vue `VTABLE`/`VCOLUMN`/`VMACRO` (+ alias `VMEMBER`).
pub(crate) fn dictionary_kind(libref: &str, name: &str) -> Option<DictKind> {
    let lib = libref.to_uppercase();
    let mem = name.to_uppercase();
    match lib.as_str() {
        "DICTIONARY" => match mem.as_str() {
            "TABLES" => Some(DictKind::Tables),
            "COLUMNS" => Some(DictKind::Columns),
            "MACROS" => Some(DictKind::Macros),
            _ => None,
        },
        "SASHELP" => match mem.as_str() {
            "VTABLE" | "VMEMBER" => Some(DictKind::Tables),
            "VCOLUMN" => Some(DictKind::Columns),
            "VMACRO" => Some(DictKind::Macros),
            _ => None,
        },
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DictKind {
    Tables,
    Columns,
    Macros,
}

/// Construit la LazyFrame d'une dictionary table à partir de l'état de session
/// (bibliothèques + datasets pour TABLES/COLUMNS ; variables macro globales
/// pour MACROS).
pub(crate) fn build_dictionary(session: &Session, kind: DictKind) -> Result<LazyFrame> {
    let df = match kind {
        DictKind::Tables => build_tables(session)?,
        DictKind::Columns => build_columns(session)?,
        DictKind::Macros => build_macros(session)?,
    };
    Ok(df.lazy())
}

/// Énumère (libname MAJUSCULE, memname MAJUSCULE) des datasets de chaque
/// bibliothèque assignée, triés (libname, memname) pour un ordre déterministe.
fn enumerate_members(session: &Session) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for lib in session.libs.librefs() {
        let Ok(provider) = session.libs.get(&lib) else {
            continue;
        };
        let Ok(members) = provider.list() else {
            continue;
        };
        for m in members {
            out.push((lib.clone(), m.to_uppercase()));
        }
    }
    out.sort();
    out
}

fn build_tables(session: &Session) -> Result<DataFrame> {
    let members = enumerate_members(session);
    let mut libname = Vec::with_capacity(members.len());
    let mut memname = Vec::with_capacity(members.len());
    let mut memtype = Vec::with_capacity(members.len());
    let mut nobs = Vec::with_capacity(members.len());
    let mut nvar = Vec::with_capacity(members.len());

    for (lib, mem) in &members {
        let provider = session.libs.get(lib)?;
        // `read` charge le dataset eager : nécessaire pour `nobs` exact et le
        // décompte de variables. Si la lecture échoue (table corrompue), on
        // ignore la table plutôt que de faire échouer toute la requête.
        let Ok((ds, _notes)) = provider.read(mem) else {
            continue;
        };
        libname.push(lib.clone());
        memname.push(mem.clone());
        memtype.push("DATA".to_string());
        nobs.push(ds.df.height() as f64);
        nvar.push(ds.vars.len() as f64);
    }

    let df = df![
        "libname" => libname,
        "memname" => memname,
        "memtype" => memtype,
        "nobs" => nobs,
        "nvar" => nvar,
    ]?;
    Ok(df)
}

fn build_columns(session: &Session) -> Result<DataFrame> {
    let members = enumerate_members(session);
    let mut libname = Vec::new();
    let mut memname = Vec::new();
    let mut name = Vec::new();
    let mut ty = Vec::new();
    let mut length = Vec::new();
    let mut npos = Vec::new();
    let mut varnum = Vec::new();
    let mut label: Vec<Option<String>> = Vec::new();
    let mut format: Vec<String> = Vec::new();
    let mut informat: Vec<String> = Vec::new();

    for (lib, mem) in &members {
        let provider = session.libs.get(lib)?;
        let Ok((ds, _notes)) = provider.read(mem) else {
            continue;
        };
        let mut pos: i64 = 0;
        for (i, v) in ds.vars.iter().enumerate() {
            libname.push(lib.clone());
            memname.push(mem.clone());
            name.push(v.name.clone());
            ty.push(match v.ty {
                crate::value::VarType::Num => "num".to_string(),
                crate::value::VarType::Char => "char".to_string(),
            });
            length.push(v.length as f64);
            npos.push(pos as f64);
            varnum.push((i + 1) as f64);
            label.push(v.label.clone());
            format.push(v.format.clone().unwrap_or_default());
            // VarMeta ne porte pas d'informat dédié → chaîne vide (fidèle au
            // rendu SAS d'une variable sans informat explicite).
            informat.push(String::new());
            pos += v.length as i64;
        }
    }

    let df = df![
        "libname" => libname,
        "memname" => memname,
        "name" => name,
        "type" => ty,
        "length" => length,
        "npos" => npos,
        "varnum" => varnum,
        "label" => label,
        "format" => format,
        "informat" => informat,
    ]?;
    Ok(df)
}

fn build_macros(session: &Session) -> Result<DataFrame> {
    let symbols = session.macro_engine.global_symbols();
    let mut pairs: Vec<(String, String)> = symbols.into_iter().collect();
    // Ordre déterministe par nom.
    pairs.sort();

    let mut scope = Vec::with_capacity(pairs.len());
    let mut name = Vec::with_capacity(pairs.len());
    let mut value = Vec::with_capacity(pairs.len());
    for (n, v) in pairs {
        scope.push(macro_scope(&n).to_string());
        name.push(n);
        value.push(v);
    }

    let df = df![
        "scope" => scope,
        "name" => name,
        "value" => value,
    ]?;
    Ok(df)
}

/// Classe un nom de variable macro globale en scope SAS. Les variables
/// automatiques amorcées par le moteur (`SYS*`) sont rapportées `AUTOMATIC` ;
/// le reste est `GLOBAL`. Nom attendu en MAJUSCULES (clé de `global_symbols`).
fn macro_scope(name: &str) -> &'static str {
    const AUTOMATIC: &[&str] = &[
        "SYSDATE9", "SYSDATE", "SYSTIME", "SYSDAY", "SYSVER", "SYSSCP",
    ];
    if AUTOMATIC.contains(&name) {
        "AUTOMATIC"
    } else {
        "GLOBAL"
    }
}
