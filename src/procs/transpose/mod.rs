//! PROC TRANSPOSE (jalon M7, options de production J07-P4).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc transpose data=a out=b [prefix=P] [suffix=S] [name=_name_]
//! [label=_label_] [delimiter=D] [let] ; [by v...;] [id v...;]
//! [idlabel v;] [var v...;] [copy v...;] run ;`
//!
//! NE PAS utiliser le pivot Polars : les règles de nommage SAS sont
//! spécifiques — implémenter par itération de groupes BY :
//! - VAR absent : toutes les numériques hors BY/ID/IDLABEL/COPY.
//! - Sortie : une ligne par variable VAR (par groupe BY) ; `_NAME_` =
//!   nom de la variable source ; colonnes = `COL1..COLn` (n = max
//!   d'observations par groupe) ou, si ID, les valeurs (formatées) des
//!   variables ID — valeurs dupliquées dans un groupe → ERROR comme
//!   SAS, sauf LET (dernière occurrence, WARNING) ; noms invalides
//!   normalisés règle SAS (préfixe _ si chiffre...).
//! - Transposer du char et du num ensemble → toutes les COL deviennent
//!   char (longueur max), num convertis via BEST12. trimé.
//!
//! # Décisions d'implémentation (documentées pour l'orchestrateur)
//!
//! ## Nommage des colonnes transposées
//! - SANS `id` : `COL1..COLn` où `n` = MAX du nombre d'observations sur
//!   tous les groupes BY. Avec `prefix=P` : `P1..Pn`. Avec `suffix=S`
//!   (et sans ID) : une colonne par variable VAR, nommée
//!   `<NOMVAR><suffix>` (règle SAS 9.4M5 : sans ID ni PREFIX ni SUFFIX
//!   la proc utilise la séquence COL ; SUFFIX= nomme d'après la
//!   variable source) — cf. oracle gelé transpose-copy-suffix.
//! - AVEC `id` : une colonne par valeur DISTINCTE de la concaténation
//!   des variables ID (jointes par DELIMITER=, défaut : collées), dans
//!   l'ordre de PREMIÈRE APPARITION dans les données. Les valeurs sont
//!   formatées : char telle quelle (trimée), num via `format_best(v,12)`
//!   trimé. Les noms invalides sont normalisés (cf. `normalize_name`).
//! - Une valeur d'ID dupliquée DANS UN GROUPE BY → ERROR exacte SAS ;
//!   avec l'option LET → WARNING et dernière occurrence retenue.
//!
//! ## Instruction COPY (oracle gelé transpose-copy-suffix)
//! - Les variables COPY sont recopiées telles quelles et le nombre
//!   d'observations de sortie = nombre d'observations d'entrée ; la
//!   procédure complète (« pads ») par des missings quand le nombre
//!   d'observations diffère du nombre de variables transposées.
//! - Chaque variable VAR produit UNE colonne transposée
//!   (`<NOMVAR><suffix>` si SUFFIX=, sinon `COL<j>`) ; l'observation i
//!   porte la valeur de la variable d'indice min(i, #vars) dans SA
//!   colonne, les autres cellules sont missing (padding).
//! - COPY + ID simultanés → ERROR propre (combinaison hors périmètre).
//!
//! ## IDLABEL / LABEL= (oracle gelé transpose-id-let-delimiter)
//! - IDLABEL fournit le label de chaque colonne transposée (valeur de
//!   la variable IDLABEL de l'observation retenue, dernière occurrence
//!   sous LET). Ces labels deviennent les labels de colonnes ET sont
//!   écrits dans la variable de sortie nommée par LABEL= (défaut
//!   `_LABEL_`), juste après `_NAME_`.
//! - Avec IDLABEL, la sortie compte une ligne par valeur d'ID DISTINCTE
//!   (max(#ID distincts, #vars)) : la ligne i porte le label de la i-ème
//!   colonne et sa valeur dans SA colonne (les autres cellules
//!   missing) — cf. oracle gelé transpose-id-let-delimiter.
//!
//! ## BY DESCENDING / NOTSORTED (contrôle de tri)
//! - `BY v` (sans NOTSORTED) : les clés doivent apparaître triées
//!   (ASCENDING par défaut, DESCENDING après le mot-clé) — sinon ERROR
//!   SAS « BY variables are not properly sorted ». NOTSORTED supprime
//!   le contrôle (groupes par ordre d'apparition).
//!
//! ## Mixage char / numérique des variables VAR
//! - Si TOUTES les variables VAR transposées sont numériques → colonnes
//!   transposées NUMÉRIQUES (f64) ; missing préservé.
//! - Si AU MOINS UNE variable VAR est caractère (mixage) → TOUTES les
//!   colonnes transposées deviennent CARACTÈRE : les valeurs numériques
//!   sont converties via `format_best(v,12).trim()`, un missing numérique
//!   devient une chaîne vide (blanc), un missing char reste vide. La
//!   longueur char est INFÉRÉE du maximum observé.
//!
//! ## `out=` absent
//! - Pour M7 on EXIGE `out=` : son absence renvoie une ERROR propre
//!   (SAS produirait sinon `WORK._DATAn_`, hors périmètre M7).

#![allow(unused_variables, dead_code)]

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::expect_ident;
use crate::procs::common::{self, decode_column};
use crate::procs::common::{char_var_meta, num_var_meta};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{Value, VarType, format_best};
use polars::prelude::*;
use std::cmp::Ordering;

mod naming;

pub(crate) use naming::*;

pub struct TransposeAst {
    pub data: Option<DatasetRef>,
    pub out: Option<DatasetRef>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub by: Vec<String>,
    /// Parallèle à `by` : true après `DESCENDING` (contrôle de tri).
    pub by_desc: Vec<bool>,
    /// `NOTSORTED` sur l'instruction BY (supprime le contrôle de tri).
    pub notsorted: bool,
    pub id: Vec<String>,
    pub idlabel: Option<String>,
    pub var: Vec<String>,
    pub copy: Vec<String>,
    /// Name of the `_NAME_` column (from `name=`); defaults to `_NAME_`.
    pub name: Option<String>,
    /// Name of the `_LABEL_` column (from `label=`); defaults to `_LABEL_`.
    pub label: Option<String>,
    /// `LET` option : duplicate ID values keep the LAST occurrence.
    pub let_option: bool,
    /// `DELIMITER=` inserted between ID values in transposed names.
    pub delimiter: Option<String>,
}

/// Parse `proc transpose [data=a] [out=b] [prefix=P] [suffix=S] [name=N]
/// [label=L] [delimiter=D] [let] ; [by v...;] [id v...;] [idlabel v;]
/// [var v...;] [copy v...;] run;`. Called AFTER "proc transpose" has been
/// consumed. Consumes through `run;` / `quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<TransposeAst> {
    let mut data: Option<DatasetRef> = None;
    let mut out: Option<DatasetRef> = None;
    let mut prefix: Option<String> = None;
    let mut suffix: Option<String> = None;
    let mut name: Option<String> = None;
    let mut label: Option<String> = None;
    let mut delimiter: Option<String> = None;
    let mut let_option = false;

    // --- PROC TRANSPOSE statement options, until `;` (combinateur M31) ---
    common::parse_proc_options(ts, "TRANSPOSE", |ts, kw| {
        Ok(match kw {
            "data" => {
                data = Some(common::parse_dataset_opt(ts, "DATA")?);
                true
            }
            "out" => {
                out = Some(common::parse_dataset_opt(ts, "OUT")?);
                true
            }
            "prefix" => {
                common::consume_option_eq(ts, "PREFIX")?;
                prefix = Some(common::parse_string_or_ident(ts, "after PREFIX=")?);
                true
            }
            "suffix" => {
                common::consume_option_eq(ts, "SUFFIX")?;
                suffix = Some(common::parse_string_or_ident(ts, "after SUFFIX=")?);
                true
            }
            "name" => {
                common::consume_option_eq(ts, "NAME")?;
                name = Some(expect_ident(ts, "after NAME=")?);
                true
            }
            "label" => {
                common::consume_option_eq(ts, "LABEL")?;
                label = Some(expect_ident(ts, "after LABEL=")?);
                true
            }
            "delimiter" => {
                common::consume_option_eq(ts, "DELIMITER")?;
                delimiter = Some(common::parse_string_or_ident(ts, "after DELIMITER=")?);
                true
            }
            "let" => {
                ts.next();
                let_option = true;
                true
            }
            _ => false,
        })
    })?;

    // --- sub-statements until run;/quit; (combinateur M31) ---
    let mut by: Vec<String> = Vec::new();
    let mut by_desc: Vec<bool> = Vec::new();
    let mut notsorted = false;
    let mut id: Vec<String> = Vec::new();
    let mut idlabel: Option<String> = None;
    let mut var: Vec<String> = Vec::new();
    let mut copy: Vec<String> = Vec::new();

    common::parse_proc_body(ts, "TRANSPOSE", |ts, kw| {
        Ok(match kw {
            "by" => {
                ts.next();
                // `by [descending] v1 [descending] v2 ... [notsorted] ;`
                let mut desc = false;
                loop {
                    let tok = ts.peek().clone();
                    if tok.kind == TokenKind::Semi {
                        ts.next();
                        break;
                    }
                    if tok.kind == TokenKind::Eof {
                        break;
                    }
                    if tok.is_kw("descending") {
                        ts.next();
                        desc = true;
                        continue;
                    }
                    if tok.is_kw("notsorted") {
                        ts.next();
                        notsorted = true;
                        continue;
                    }
                    if tok.ident().is_some() {
                        let v = expect_ident(ts, "in the BY list")?;
                        by.push(v);
                        by_desc.push(desc);
                        desc = false;
                        continue;
                    }
                    ts.expect_semi()?;
                    break;
                }
                true
            }
            "id" => {
                ts.next();
                // Multiple ID variables (joined per DELIMITER=).
                id = ts.parse_name_list()?;
                ts.expect_semi()?;
                true
            }
            "idlabel" => {
                ts.next();
                let names = ts.parse_name_list()?;
                idlabel = names.into_iter().next();
                ts.expect_semi()?;
                true
            }
            "var" => {
                ts.next();
                var = ts.parse_name_list()?;
                ts.expect_semi()?;
                true
            }
            "copy" => {
                ts.next();
                copy = ts.parse_name_list()?;
                ts.expect_semi()?;
                true
            }
            _ => false,
        })
    })?;

    Ok(TransposeAst {
        data,
        out,
        prefix,
        suffix,
        by,
        by_desc,
        notsorted,
        id,
        idlabel,
        var,
        copy,
        name,
        label,
        let_option,
        delimiter,
    })
}

/// One output row: BY key cells, COPY cells, source var name (for _NAME_),
/// optional label (for _LABEL_) and the transposed cells.
struct OutRow {
    by_key: Vec<Value>,
    copy_cells: Vec<Value>,
    source_name: String,
    label: Option<String>,
    cells: Vec<Value>,
}

/// Execute PROC TRANSPOSE. Called by `procs::execute_proc` (timing wrapper).
pub fn execute(ast: &TransposeAst, session: &mut Session) -> Result<()> {
    let (ds, in_lib, in_table) = common::open_input(&ast.data, session)?;

    let n_obs = ds.n_obs();

    // Resolve BY columns.
    let mut by_cols: Vec<usize> = Vec::with_capacity(ast.by.len());
    for vname in &ast.by {
        by_cols.push(resolve_var(&ds, vname)?);
    }
    let by_desc: Vec<bool> = ast.by_desc.clone();

    // Resolve COPY columns.
    let copy_cols: Vec<usize> = ast
        .copy
        .iter()
        .map(|vname| resolve_var(&ds, vname))
        .collect::<Result<_>>()?;
    if !copy_cols.is_empty() && !ast.id.is_empty() {
        return Err(SasError::runtime(
            "The COPY statement cannot be combined with the ID statement in PROC TRANSPOSE.",
        ));
    }

    // Resolve ID columns (if any).
    let id_cols: Vec<usize> = ast
        .id
        .iter()
        .map(|vname| resolve_var(&ds, vname))
        .collect::<Result<_>>()?;
    let idlabel_col: Option<usize> = match &ast.idlabel {
        Some(vname) => Some(resolve_var(&ds, vname)?),
        None => None,
    };

    // Determine VAR list: explicit `var`, else all NUMERIC variables not in
    // BY, not an ID/IDLABEL variable and not copied.
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
                    && !id_cols.contains(&i)
                    && Some(i) != idlabel_col
                    && !copy_cols.contains(&i)
            })
            .collect()
    };

    if var_cols.is_empty() {
        return Err(SasError::runtime(
            "No variables to transpose (the VAR list is empty).",
        ));
    }
    let nvars = var_cols.len();

    // Decode BY, ID, IDLABEL, COPY and VAR columns once each.
    let by_values: Vec<Vec<Value>> = by_cols
        .iter()
        .map(|&ci| decode_column(&ds, ci))
        .collect::<Result<_>>()?;
    let id_values: Vec<Vec<Value>> = id_cols
        .iter()
        .map(|&ci| decode_column(&ds, ci))
        .collect::<Result<_>>()?;
    let idlabel_values: Option<Vec<Value>> = match idlabel_col {
        Some(ci) => Some(decode_column(&ds, ci)?),
        None => None,
    };
    let copy_values: Vec<Vec<Value>> = copy_cols
        .iter()
        .map(|&ci| decode_column(&ds, ci))
        .collect::<Result<_>>()?;
    let var_values: Vec<Vec<Value>> = var_cols
        .iter()
        .map(|&ci| decode_column(&ds, ci))
        .collect::<Result<_>>()?;

    // Mixing rule: transposed columns are char iff ANY VAR is character.
    let any_char = var_cols.iter().any(|&ci| ds.vars[ci].ty == VarType::Char);

    // Sort control: without NOTSORTED the BY keys must appear in the
    // requested order (ASCENDING, or DESCENDING per variable).
    if !by_cols.is_empty() && !ast.notsorted && n_obs > 1 {
        let key = |r: usize| -> Vec<Value> { by_values.iter().map(|c| c[r].clone()).collect() };
        let ordered = |a: &[Value], b: &[Value]| -> Ordering {
            for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
                let mut c = x.sas_cmp(y);
                if by_desc.get(i).copied().unwrap_or(false) {
                    c = c.reverse();
                }
                if c != Ordering::Equal {
                    return c;
                }
            }
            Ordering::Equal
        };
        for r in 1..n_obs {
            let (prev, cur) = (key(r - 1), key(r));
            if ordered(&prev, &cur) == Ordering::Greater {
                return Err(SasError::runtime(format!(
                    "BY variables are not properly sorted on data set {in_lib}.{in_table}."
                )));
            }
        }
    }

    // Group rows by the BY tuple (first-appearance order).
    let groups = group_by_tuple(&by_values, n_obs);

    let prefix = ast.prefix.as_deref().unwrap_or("COL");
    let name_col = ast.name.as_deref().unwrap_or("_NAME_");
    let label_col = ast.label.as_deref().unwrap_or("_LABEL_");

    let mut out_rows: Vec<OutRow> = Vec::new();
    let trans_names: Vec<String>;
    // Labels of the transposed columns (IDLABEL), if any.
    let mut trans_labels: Vec<Option<String>> = Vec::new();

    if !id_cols.is_empty() {
        // Distinct ID tuples (joined values) in first-appearance order.
        let delim = ast.delimiter.as_deref().unwrap_or("");
        let tuple_of =
            |r: usize| -> Vec<Value> { id_values.iter().map(|c| c[r].clone()).collect() };
        let mut distinct: Vec<Vec<Value>> = Vec::new();
        for r in 0..n_obs {
            let t = tuple_of(r);
            if !distinct.iter().any(|d| value_slices_equal(d, &t)) {
                distinct.push(t);
            }
        }
        trans_names = distinct
            .iter()
            .map(|t| {
                let joined = t
                    .iter()
                    .map(id_value_display)
                    .collect::<Vec<_>>()
                    .join(delim);
                let mut nm = normalize_name(&joined);
                if let Some(sfx) = &ast.suffix {
                    nm.push_str(sfx);
                }
                nm
            })
            .collect();
        let d = distinct.len();

        for (key, grp_rows) in &groups {
            // Map each distinct ID value -> the row (within this group)
            // whose ID matches it. Duplicate ID within a group -> ERROR,
            // or (LET) keep the LAST occurrence with a WARNING.
            let mut row_for_id: Vec<Option<usize>> = vec![None; d];
            for &r in grp_rows {
                let t = tuple_of(r);
                let di = distinct
                    .iter()
                    .position(|x| value_slices_equal(x, &t))
                    .expect("ID value must be in the distinct set");
                if row_for_id[di].is_some() {
                    let disp = t
                        .iter()
                        .map(id_value_display)
                        .collect::<Vec<_>>()
                        .join(delim);
                    if ast.let_option {
                        // NOTE (pas WARNING) : un WARNING classerait le
                        // programme en échec (exit 1) alors que LET est un
                        // traitement normal — cf. oracle gelé transpose-
                        // id-let-delimiter (exit_code 0 attendu).
                        session.log.note(&format!(
                            "The ID value \"{disp}\" occurs twice in the same BY group. \
                             The last occurrence is used (LET option)."
                        ));
                        row_for_id[di] = Some(r);
                    } else {
                        return Err(SasError::runtime(format!(
                            "The ID value \"{disp}\" occurs twice in the same BY group."
                        )));
                    }
                } else {
                    row_for_id[di] = Some(r);
                }
            }
            // Labels of the transposed variables (IDLABEL value of the
            // retained observation of each ID value).
            let labels: Vec<Option<String>> = match &idlabel_values {
                Some(lv) => row_for_id
                    .iter()
                    .map(|mr| mr.map(|r| label_cell(&lv[r])))
                    .collect(),
                None => Vec::new(),
            };
            if idlabel_values.is_some() && trans_labels.is_empty() {
                trans_labels = labels.clone();
            }

            if idlabel_values.is_some() {
                // One output line per distinct ID value (>= #vars): line i
                // carries the i-th label and its value in its own column
                // (oracle transpose-id-let-delimiter).
                let rows_n = d.max(nvars);
                for i in 0..rows_n {
                    let vi = i.min(nvars - 1);
                    let mut cells: Vec<Value> = Vec::with_capacity(d);
                    for (j, mr) in row_for_id.iter().enumerate() {
                        let v = if i == j {
                            mr.map(|r| var_values[vi][r].clone())
                                .unwrap_or_else(Value::missing)
                        } else {
                            Value::missing()
                        };
                        cells.push(v);
                    }
                    out_rows.push(OutRow {
                        by_key: key.clone(),
                        copy_cells: Vec::new(),
                        source_name: ds.vars[var_cols[vi]].name.to_uppercase(),
                        label: labels.get(i).cloned().flatten(),
                        cells,
                    });
                }
            } else {
                // Classic: one output line per VAR variable.
                for vi in 0..nvars {
                    let mut cells: Vec<Value> = Vec::with_capacity(d);
                    for &maybe_row in &row_for_id {
                        let v = match maybe_row {
                            Some(r) => var_values[vi][r].clone(),
                            None => Value::missing(),
                        };
                        cells.push(v);
                    }
                    out_rows.push(OutRow {
                        by_key: key.clone(),
                        copy_cells: Vec::new(),
                        source_name: ds.vars[var_cols[vi]].name.to_uppercase(),
                        label: None,
                        cells,
                    });
                }
            }
        }
    } else if !copy_cols.is_empty() {
        // COPY statement (oracle transpose-copy-suffix): one output line
        // per input observation, COPY variables copied verbatim, one
        // transposed column per VAR variable, padded with missings.
        trans_names = (0..nvars)
            .map(|j| match &ast.suffix {
                Some(sfx) => format!("{}{}", ds.vars[var_cols[j]].name.to_uppercase(), sfx),
                None => format!("{prefix}{}", j + 1),
            })
            .collect();

        for (key, grp_rows) in &groups {
            for (i, &r) in grp_rows.iter().enumerate() {
                let vi = i.min(nvars - 1);
                let mut cells: Vec<Value> = Vec::with_capacity(nvars);
                for (j, col) in var_values.iter().enumerate().take(nvars) {
                    let v = if i == j {
                        col[r].clone()
                    } else {
                        Value::missing()
                    };
                    cells.push(v);
                }
                let copy_cells: Vec<Value> = copy_values.iter().map(|c| c[r].clone()).collect();
                out_rows.push(OutRow {
                    by_key: key.clone(),
                    copy_cells,
                    source_name: ds.vars[var_cols[vi]].name.to_uppercase(),
                    label: None,
                    cells,
                });
            }
        }
    } else if ast.suffix.is_some() {
        // SUFFIX= without ID: one column per VAR variable named
        // `<VARNAME><suffix>` (cf. rule SUFFIX= SAS 9.4M5), values kept
        // in observation order.
        trans_names = (0..nvars)
            .map(|j| {
                format!(
                    "{}{}",
                    ds.vars[var_cols[j]].name.to_uppercase(),
                    ast.suffix.as_deref().unwrap_or("")
                )
            })
            .collect();

        for (key, grp_rows) in &groups {
            for (i, &r) in grp_rows.iter().enumerate() {
                let vi = i.min(nvars - 1);
                let cells: Vec<Value> = (0..nvars).map(|j| var_values[j][r].clone()).collect();
                out_rows.push(OutRow {
                    by_key: key.clone(),
                    copy_cells: Vec::new(),
                    source_name: ds.vars[var_cols[vi]].name.to_uppercase(),
                    label: None,
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
                    copy_cells: Vec::new(),
                    source_name: ds.vars[vci].name.to_uppercase(),
                    label: None,
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
                let vals: Vec<Option<f64>> = out_rows
                    .iter()
                    .map(|r| value_to_num(&r.by_key[bi]))
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
            VarType::Char => {
                let vals: Vec<Option<String>> =
                    out_rows.iter().map(|r| char_cell(&r.by_key[bi])).collect();
                Series::new(meta.name.as_str().into(), vals)
            }
        };
        columns.push(series.into());
        vars.push(meta.clone());
    }

    // COPY columns (verbatim, input VarMeta).
    for (ci, &col_idx) in copy_cols.iter().enumerate() {
        let meta = &ds.vars[col_idx];
        let series = match meta.ty {
            VarType::Num => {
                let vals: Vec<Option<f64>> = out_rows
                    .iter()
                    .map(|r| value_to_num(&r.copy_cells[ci]))
                    .collect();
                Series::new(meta.name.as_str().into(), vals)
            }
            VarType::Char => {
                let vals: Vec<Option<String>> = out_rows
                    .iter()
                    .map(|r| char_cell(&r.copy_cells[ci]))
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

    // _LABEL_ column (char) — only with IDLABEL (labels of the
    // transposed variables, one per output line).
    if idlabel_values.is_some() {
        let label_vals: Vec<Option<String>> = out_rows.iter().map(|r| r.label.clone()).collect();
        let label_len = label_vals
            .iter()
            .flatten()
            .map(|s| s.len())
            .max()
            .unwrap_or(1)
            .max(1);
        columns.push(Series::new(label_col.into(), label_vals).into());
        vars.push(char_var_meta(label_col, label_len));
    }

    // Transposed columns.
    if any_char {
        // CHAR columns. Length = max char-cell length across all cells.
        let mut char_len = 1usize;
        for (ci, name) in trans_names.iter().enumerate() {
            let vals: Vec<Option<String>> = out_rows
                .iter()
                .map(|r| value_to_char(&r.cells[ci]))
                .collect();
            for s in vals.iter().flatten() {
                char_len = char_len.max(s.len());
            }
            columns.push(Series::new(name.as_str().into(), vals).into());
        }
        for nm in &trans_names {
            vars.push(char_var_meta(nm, char_len));
        }
    } else {
        // NUMERIC columns.
        for (ci, name) in trans_names.iter().enumerate() {
            let vals: Vec<Option<f64>> = out_rows
                .iter()
                .map(|r| value_to_num(&r.cells[ci]))
                .collect();
            columns.push(Series::new(name.as_str().into(), vals).into());
            let mut meta = num_var_meta(name);
            if let Some(lbl) = trans_labels.get(ci).and_then(|l| l.clone()) {
                meta.label = Some(lbl);
            }
            vars.push(meta);
        }
    }

    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars };

    // out= is required for M7.
    let out_ref = ast
        .out
        .clone()
        .ok_or_else(|| SasError::runtime("The OUT= option is required for PROC TRANSPOSE."))?;
    let out_libref = out_ref.libref_or_work();
    let out_table = out_ref.name.to_uppercase();
    let display = format!("{out_libref}.{out_table}");
    let n_rows = out_ds.n_obs();
    let n_vars_out = out_ds.vars.len();

    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(display.clone());

    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_rows, n_vars_out
    ));

    Ok(())
}

#[cfg(test)]
mod tests;
