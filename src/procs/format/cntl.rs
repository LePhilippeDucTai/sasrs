//! M39.2 — `CNTLOUT=`/`CNTLIN=` : catalogue de formats ↔ dataset de contrôle.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `PROC FORMAT CNTLOUT=ds;` dépose une observation PAR PLAGE de chaque
//! format VALUE/INVALUE actuellement dans le catalogue ciblé (`WORK` ou
//! `LIBRARY=<libref>`) dans le dataset `ds`. `PROC FORMAT CNTLIN=ds;` fait
//! l'inverse : lit `ds` et (re)définit les formats correspondants dans le
//! catalogue, exactement comme s'ils avaient été écrits en `VALUE`/`INVALUE`
//! (même NOTE « Format X has been output. », même interaction avec `LIB=`).
//!
//! ## Colonnes produites/lues (sous-ensemble fidèle des colonnes réelles SAS)
//!
//! | Colonne  | Type | Rôle                                                          |
//! |----------|------|----------------------------------------------------------------|
//! | FMTNAME  | char | nom du format, SANS le `$` (le `$` est encodé par TYPE)        |
//! | START    | char | borne basse, texte ; vide si LOW ou si la ligne est OTHER       |
//! | END      | char | borne haute, texte ; vide si HIGH ou si la ligne est OTHER      |
//! | LABEL    | char | libellé de la plage (résultat pour un INVALUE, cf. ci-dessous)  |
//! | TYPE     | char | `N`=format num, `C`=format char, `I`=informat num, `J`=informat char |
//! | SEXCL    | char | `Y`/`N` — borne basse exclusive (`<-`)                          |
//! | EEXCL    | char | `Y`/`N` — borne haute exclusive (`-<`)                          |
//! | HLO      | char | lettres parmi `L`(OW)/`H`(IGH)/`O`(THER) ; vide sinon           |
//!
//! Colonnes IMPLÉMENTÉES uniquement (le sur-ensemble réel SAS ajoute MIN/MAX/
//! DEFAULT/LENGTH/FUZZ/PREFIX/MULT/FILL/NOEDIT/DECSEP/DIG3SEP/… — hors
//! périmètre M39.2, repris en partie par M43.1 pour les largeurs
//! MIN=/MAX=/DEFAULT=). FMTNAME/START/END/LABEL/TYPE sont considérées
//! obligatoires en lecture (`CNTLIN=`) ; SEXCL/EEXCL/HLO sont optionnelles
//! (absentes → borne inclusive, pas de marqueur LOW/HIGH/OTHER).
//!
//! ## INVALUE : encodage du résultat dans LABEL
//!
//! Un `INVALUE` peut produire un résultat numérique, caractère, `_SAME_`, ou
//! manquant (`.`/`._`/`.A`..`.Z`). La colonne LABEL est unique et caractère :
//! on y encode donc le résultat en texte (`21`, `Small`, `_SAME_`, `.`,
//! `._`, `.A`) et on le redécode symétriquement au `CNTLIN=`. Ambiguïté
//! documentée : un LABEL caractère qui vaut LITTÉRALEMENT `_SAME_` ou un
//! motif de manquant sera relu comme `_SAME_`/manquant plutôt que comme la
//! chaîne caractère — cas de bord non couvert, comme en SAS réel où le même
//! canal texte porte ces deux usages.
//!
//! ## PICTURE (TYPE=P) : hors périmètre
//!
//! Les formats PICTURE ne sont ni écrits par `CNTLOUT=` ni reconstruits par
//! `CNTLIN=` dans cette build : leurs directives (PREFIX/MULT/FILL) n'ont pas
//! de colonne dans le jeu minimal ci-dessus, et les approximer silencieusement
//! produirait un round-trip qui A L'AIR fidèle mais ne l'est pas. `CNTLOUT=`
//! émet une NOTE indiquant le nombre de formats PICTURE omis (s'il y en a) ;
//! `CNTLIN=` émet une NOTE et saute toute observation `TYPE='P'` rencontrée.
//!
//! ## Déterminisme de l'ordre des observations
//!
//! Le catalogue est stocké en `HashMap` (ordre d'itération non déterministe
//! d'un run à l'autre). `CNTLOUT=` trie donc les formats par (FMTNAME, TYPE)
//! avant d'émettre leurs lignes — nécessaire pour que le listing/snapshot
//! soit reproductible ET pour que `CNTLOUT→CNTLIN→CNTLOUT` soit
//! octet-identique (le second passage retrouve le même ordre de tri, pas
//! l'ordre d'un HashMap). À l'intérieur d'un même format, les plages gardent
//! l'ordre du dataset lu (donc l'ordre de définition d'origine), OTHER en
//! dernier.

use super::*;
use crate::ast::DatasetRef;
use crate::dataset::SasDataset;
use crate::formats::FormatCatalog;
use crate::procs::common::{char_var_meta, decode_column, open_resolved};
use crate::value::Value;
use polars::prelude::*;
use std::collections::HashMap;

/// Reconstructed VALUE/INVALUE entries from a `CNTLIN=` dataset, in the
/// dataset's own row order — see [`read_cntlin`].
type CntlinEntries = (Vec<(String, UserFormat)>, Vec<(String, UserInformat)>);

/// One row of the CNTLOUT/CNTLIN control dataset (see module doc for the
/// column semantics).
struct CntlRow {
    fmtname: String,
    start: String,
    end: String,
    label: String,
    ty: char,
    sexcl: bool,
    eexcl: bool,
    hlo: String,
}

// ── CNTLOUT= : catalogue → dataset ──────────────────────────────────────────

/// Numeric bound → text. Rust's default `f64` `Display` produces the
/// shortest decimal string that reparses to the exact same value, without
/// scientific notation — exactly what a round-trip-safe CNTLOUT column needs.
fn format_num_bound(n: f64) -> String {
    format!("{n}")
}

fn bound_text(b: &Bound) -> String {
    match b {
        Bound::Low | Bound::High => String::new(),
        Bound::Num(n) => format_num_bound(*n),
        Bound::Char(s) => s.clone(),
    }
}

fn bound_hlo(b: &Bound) -> Option<char> {
    match b {
        Bound::Low => Some('L'),
        Bound::High => Some('H'),
        _ => None,
    }
}

fn range_row(
    fmtname: &str,
    ty: char,
    from: &Bound,
    to: &Bound,
    from_excl: bool,
    to_excl: bool,
    label: &str,
) -> CntlRow {
    let mut hlo = String::new();
    if let Some(c) = bound_hlo(from) {
        hlo.push(c);
    }
    if let Some(c) = bound_hlo(to) {
        hlo.push(c);
    }
    CntlRow {
        fmtname: fmtname.to_string(),
        start: bound_text(from),
        end: bound_text(to),
        label: label.to_string(),
        ty,
        sexcl: from_excl,
        eexcl: to_excl,
        hlo,
    }
}

fn other_row(fmtname: &str, ty: char, label: &str) -> CntlRow {
    CntlRow {
        fmtname: fmtname.to_string(),
        start: String::new(),
        end: String::new(),
        label: label.to_string(),
        ty,
        sexcl: false,
        eexcl: false,
        hlo: "O".to_string(),
    }
}

/// Encode an INVALUE result as CNTLOUT LABEL text — see module doc.
fn informat_value_to_label(iv: &InformatValue) -> String {
    match iv {
        InformatValue::Num(n) => format_num_bound(*n),
        InformatValue::Char(s) => s.clone(),
        InformatValue::Same => "_SAME_".to_string(),
        InformatValue::Missing(kind) => match kind.as_str() {
            "" | "." => ".".to_string(),
            "_" => "._".to_string(),
            s => format!(".{}", s.to_uppercase()),
        },
    }
}

/// Inverse of [`informat_value_to_label`] — see module doc for the (documented)
/// ambiguity with a genuine char label equal to one of these literal tokens.
fn label_to_informat_value(label: &str) -> InformatValue {
    if label == "_SAME_" {
        return InformatValue::Same;
    }
    if label == "." {
        return InformatValue::Missing(".".to_string());
    }
    if label == "._" {
        return InformatValue::Missing("_".to_string());
    }
    if label.len() == 2 && label.starts_with('.') {
        let c = label.as_bytes()[1];
        if c.is_ascii_uppercase() {
            return InformatValue::Missing((c as char).to_string());
        }
    }
    match label.trim().parse::<f64>() {
        Ok(n) => InformatValue::Num(n),
        Err(_) => InformatValue::Char(label.to_string()),
    }
}

/// Build the full, DETERMINISTICALLY ORDERED row set for `CNTLOUT=` from a
/// resolved catalog (see module doc — sort key is (FMTNAME, TYPE), PICTURE
/// entries are skipped).
fn build_rows(cat: &FormatCatalog) -> Vec<CntlRow> {
    let mut groups: Vec<(String, char, Vec<CntlRow>)> = Vec::new();

    for (key, uf) in cat.user_formats() {
        let fmtname = key.trim_start_matches('$');
        let ty = if uf.is_char { 'C' } else { 'N' };
        let mut rows: Vec<CntlRow> = uf
            .ranges
            .iter()
            .map(|r| {
                range_row(
                    fmtname,
                    ty,
                    &r.from,
                    &r.to,
                    r.from_exclusive,
                    r.to_exclusive,
                    &r.label,
                )
            })
            .collect();
        if let Some(other) = &uf.other {
            rows.push(other_row(fmtname, ty, other));
        }
        groups.push((fmtname.to_string(), ty, rows));
    }

    for (key, ui) in cat.user_informats() {
        let fmtname = key.trim_start_matches('$');
        let ty = if ui.is_char_result { 'J' } else { 'I' };
        let mut rows: Vec<CntlRow> = ui
            .ranges
            .iter()
            .map(|r| {
                range_row(
                    fmtname,
                    ty,
                    &r.from,
                    &r.to,
                    r.from_exclusive,
                    r.to_exclusive,
                    &informat_value_to_label(&r.result),
                )
            })
            .collect();
        if let Some(other) = &ui.other {
            rows.push(other_row(fmtname, ty, &informat_value_to_label(other)));
        }
        groups.push((fmtname.to_string(), ty, rows));
    }

    groups.sort_by(|a, b| (a.0.as_str(), a.1).cmp(&(b.0.as_str(), b.1)));
    groups.into_iter().flat_map(|(_, _, rows)| rows).collect()
}

/// Write the `CNTLOUT=` dataset from an already-resolved catalog (the
/// caller has already applied this step's own updates). Emits a NOTE for any
/// PICTURE formats present but not included (see module doc).
pub(super) fn write_cntlout(
    cntlout_ref: &DatasetRef,
    cat: &FormatCatalog,
    session: &mut Session,
) -> Result<()> {
    let n_pictures = cat.user_picture_count();
    if n_pictures > 0 {
        session.log.note(&format!(
            "CNTLOUT={}: {} PICTURE format(s) in the catalog were not included \
             — PICTURE formats are not supported by CNTLOUT=/CNTLIN= in this build.",
            cntlout_ref.display(),
            n_pictures
        ));
    }

    let rows = build_rows(cat);

    let fmtname_len = rows
        .iter()
        .map(|r| r.fmtname.len())
        .max()
        .unwrap_or(1)
        .max(1);
    let text_len = rows
        .iter()
        .flat_map(|r| [r.start.len(), r.end.len()])
        .max()
        .unwrap_or(1)
        .max(1);
    let label_len = rows.iter().map(|r| r.label.len()).max().unwrap_or(1).max(1);
    let hlo_len = rows.iter().map(|r| r.hlo.len()).max().unwrap_or(1).max(1);

    let fmtnames: Vec<Option<String>> = rows.iter().map(|r| Some(r.fmtname.clone())).collect();
    let starts: Vec<Option<String>> = rows.iter().map(|r| Some(r.start.clone())).collect();
    let ends: Vec<Option<String>> = rows.iter().map(|r| Some(r.end.clone())).collect();
    let labels: Vec<Option<String>> = rows.iter().map(|r| Some(r.label.clone())).collect();
    let types: Vec<Option<String>> = rows.iter().map(|r| Some(r.ty.to_string())).collect();
    let sexcls: Vec<Option<String>> = rows
        .iter()
        .map(|r| Some(if r.sexcl { "Y" } else { "N" }.to_string()))
        .collect();
    let eexcls: Vec<Option<String>> = rows
        .iter()
        .map(|r| Some(if r.eexcl { "Y" } else { "N" }.to_string()))
        .collect();
    let hlos: Vec<Option<String>> = rows.iter().map(|r| Some(r.hlo.clone())).collect();

    let columns: Vec<Column> = vec![
        Series::new("FMTNAME".into(), fmtnames).into(),
        Series::new("START".into(), starts).into(),
        Series::new("END".into(), ends).into(),
        Series::new("LABEL".into(), labels).into(),
        Series::new("TYPE".into(), types).into(),
        Series::new("SEXCL".into(), sexcls).into(),
        Series::new("EEXCL".into(), eexcls).into(),
        Series::new("HLO".into(), hlos).into(),
    ];
    let out_vars = vec![
        char_var_meta("FMTNAME", fmtname_len),
        char_var_meta("START", text_len),
        char_var_meta("END", text_len),
        char_var_meta("LABEL", label_len),
        char_var_meta("TYPE", 1),
        char_var_meta("SEXCL", 1),
        char_var_meta("EEXCL", 1),
        char_var_meta("HLO", hlo_len),
    ];
    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars: out_vars };

    let out_libref = cntlout_ref.libref_or_work();
    let out_table = cntlout_ref.name.to_uppercase();
    let display = format!("{out_libref}.{out_table}");
    let n_rows = rows.len();
    let n_vars = out_ds.vars.len();

    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(display.clone());
    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_rows, n_vars
    ));
    Ok(())
}

// ── CNTLIN= : dataset → catalogue ───────────────────────────────────────────

fn parse_num_bound(s: &str, cntlin_ref: &DatasetRef, fmtname: &str, which: &str) -> Result<f64> {
    s.trim().parse::<f64>().map_err(|_| {
        SasError::runtime(format!(
            "CNTLIN={}: invalid numeric {} value '{}' for format {}.",
            cntlin_ref.display(),
            which,
            s,
            fmtname
        ))
    })
}

/// Read a `CNTLIN=` control dataset and return the reconstructed VALUE and
/// INVALUE entries, in the dataset's own row order (an entry's ranges keep
/// the order they appear in `ds`, OTHER row or not). PICTURE rows
/// (`TYPE='P'`) are counted and skipped with a NOTE; unrecognised TYPE values
/// are silently skipped (tolerant read, mirrors the "ignore what you don't
/// understand" spirit of a hand-authored control dataset).
pub(super) fn read_cntlin(cntlin_ref: &DatasetRef, session: &mut Session) -> Result<CntlinEntries> {
    let ds = open_resolved(cntlin_ref, session)?;

    let col = |name: &str| -> Option<usize> {
        ds.vars
            .iter()
            .position(|v| v.name.eq_ignore_ascii_case(name))
    };

    let fmtname_i = col("FMTNAME").ok_or_else(|| {
        SasError::runtime(format!(
            "CNTLIN={}: the control dataset has no FMTNAME variable.",
            cntlin_ref.display()
        ))
    })?;
    let ty_i = col("TYPE").ok_or_else(|| {
        SasError::runtime(format!(
            "CNTLIN={}: the control dataset has no TYPE variable.",
            cntlin_ref.display()
        ))
    })?;

    let fmtnames = decode_column(&ds, fmtname_i)?;
    let types = decode_column(&ds, ty_i)?;
    let starts = col("START").map(|i| decode_column(&ds, i)).transpose()?;
    let ends = col("END").map(|i| decode_column(&ds, i)).transpose()?;
    let labels = col("LABEL").map(|i| decode_column(&ds, i)).transpose()?;
    let sexcls = col("SEXCL").map(|i| decode_column(&ds, i)).transpose()?;
    let eexcls = col("EEXCL").map(|i| decode_column(&ds, i)).transpose()?;
    let hlos = col("HLO").map(|i| decode_column(&ds, i)).transpose()?;

    let get_str = |col: &Option<Vec<Value>>, i: usize| -> String {
        match col {
            Some(v) => match &v[i] {
                Value::Char(s) => s.clone(),
                _ => String::new(),
            },
            None => String::new(),
        }
    };

    let mut value_order: Vec<String> = Vec::new();
    let mut value_map: HashMap<String, UserFormat> = HashMap::new();
    let mut informat_order: Vec<String> = Vec::new();
    let mut informat_map: HashMap<String, UserInformat> = HashMap::new();
    let mut n_pictures_skipped = 0usize;

    for i in 0..ds.n_obs() {
        let fmtname = match &fmtnames[i] {
            Value::Char(s) => s.trim().to_string(),
            _ => String::new(),
        };
        if fmtname.is_empty() {
            continue;
        }
        let ty = match &types[i] {
            Value::Char(s) => s.trim().to_uppercase().chars().next().unwrap_or(' '),
            _ => ' ',
        };

        let start = get_str(&starts, i);
        let end = get_str(&ends, i);
        let label = get_str(&labels, i);
        let sexcl = get_str(&sexcls, i).trim().eq_ignore_ascii_case("Y");
        let eexcl = get_str(&eexcls, i).trim().eq_ignore_ascii_case("Y");
        let hlo = get_str(&hlos, i).trim().to_uppercase();
        let is_other = hlo.contains('O');
        let is_low = hlo.contains('L');
        let is_high = hlo.contains('H');

        match ty {
            'N' | 'C' => {
                let is_char = ty == 'C';
                let key = if is_char {
                    format!("${fmtname}")
                } else {
                    fmtname.clone()
                };
                let is_new = !value_map.contains_key(&key);
                let entry = value_map.entry(key.clone()).or_insert_with(|| UserFormat {
                    is_char,
                    ranges: Vec::new(),
                    other: None,
                });
                if is_new {
                    value_order.push(key.clone());
                }
                if is_other {
                    entry.other = Some(label);
                } else {
                    let from = if is_low {
                        Bound::Low
                    } else if is_char {
                        Bound::Char(start)
                    } else {
                        Bound::Num(parse_num_bound(&start, cntlin_ref, &fmtname, "START")?)
                    };
                    let to = if is_high {
                        Bound::High
                    } else if is_char {
                        Bound::Char(end)
                    } else {
                        Bound::Num(parse_num_bound(&end, cntlin_ref, &fmtname, "END")?)
                    };
                    entry.ranges.push(Range {
                        from,
                        to,
                        from_exclusive: sexcl,
                        to_exclusive: eexcl,
                        label,
                    });
                }
            }
            'I' | 'J' => {
                let is_char_result = ty == 'J';
                let key = if is_char_result {
                    format!("${fmtname}")
                } else {
                    fmtname.clone()
                };
                let is_new = !informat_map.contains_key(&key);
                let entry = informat_map
                    .entry(key.clone())
                    .or_insert_with(|| UserInformat {
                        is_char_result,
                        ranges: Vec::new(),
                        other: None,
                    });
                if is_new {
                    informat_order.push(key.clone());
                }
                let value = label_to_informat_value(&label);
                if is_other {
                    entry.other = Some(value);
                } else {
                    let from = if is_low {
                        Bound::Low
                    } else {
                        Bound::Char(start)
                    };
                    let to = if is_high {
                        Bound::High
                    } else {
                        Bound::Char(end)
                    };
                    entry.ranges.push(InformatRange {
                        from,
                        to,
                        from_exclusive: sexcl,
                        to_exclusive: eexcl,
                        result: value,
                    });
                }
            }
            'P' => n_pictures_skipped += 1,
            _ => {
                // Unrecognised TYPE: tolerated, skipped (see doc comment).
            }
        }
    }

    if n_pictures_skipped > 0 {
        session.log.note(&format!(
            "CNTLIN={}: {} PICTURE format observation(s) (TYPE='P') were skipped \
             — PICTURE formats are not reconstructed by CNTLIN= in this build.",
            cntlin_ref.display(),
            n_pictures_skipped
        ));
    }

    let values = value_order
        .into_iter()
        .map(|k| {
            let v = value_map.remove(&k).expect("just inserted");
            (k, v)
        })
        .collect();
    let invalues = informat_order
        .into_iter()
        .map(|k| {
            let v = informat_map.remove(&k).expect("just inserted");
            (k, v)
        })
        .collect();

    Ok((values, invalues))
}
