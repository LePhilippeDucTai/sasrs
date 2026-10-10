//! PROC TABULATE — bounded v1 (LISTING output only).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc tabulate data=<ref>; class <vars>; var <vars>;
//!  table <dimexpr> [, <dimexpr>]; run;`/`quit;`
//!
//! ## Périmètre v1 (STRICT — tout le reste est une erreur propre, jamais
//! un no-op silencieux).
//!
//! ### Statements
//! - `proc tabulate data=<ref> [out=<ref>] [format=<fmt>] [missing];`.
//! - `class <var list>;` — variables catégorielles. Décodées une fois via
//!   `common::decode_column` ; les niveaux sont les valeurs FORMATÉES par le
//!   format stocké de la variable (BEST12. / valeur brute sans format), rangés
//!   par leur plus petite valeur non formatée (J02-P7 : SAS regroupe sur la
//!   valeur formatée, ORDER=UNFORMATTED par défaut). Une observation dont UNE
//!   variable CLASS est manquante est exclue de toute la table (cellules, ALL,
//!   dénominateurs) ; avec l'option PROC `MISSING`, la valeur manquante est un
//!   niveau à part entière (doc SAS 9.4, CLASS statement, MISSING).
//! - `var <var list>;` — variables d'analyse numériques.
//! - `table <dimexpr> [, <dimexpr> [, <dimexpr>]];` — UNE dimension (colonnes
//!   seules), DEUX dimensions (`lignes , colonnes`) ou TROIS dimensions
//!   (`page , lignes , colonnes`). La dimension page produit un sous-tableau
//!   row×col répété par catégorie de page, précédé d'un libellé de page. UNE
//!   seule instruction TABLE par étape : une deuxième est une ERROR (J03-P2).
//! - `classlev`, `keylabel`, `keyword` : WARNING d'affichage (en-têtes).
//! - `run;` / `quit;`.
//!
//! ### Grammaire d'expression de table (v1 — petite et précise)
//! ```text
//! dimexpr := term { term }            (* concaténation par blancs = empilage *)
//! term    := factor { '*' factor }    (* croisement *)
//! factor  := atom | '(' dimexpr ')'
//! atom    := NAME | STATKW
//!            { '=' STRLIT }           (* libellé d'en-tête, M33.4 *)
//!            { '*' 'F' '=' FORMAT }   (* format de cellule, M33.4 *)
//! ```
//! - `='label'` après un NAME/STATKW remplace le texte rendu dans l'en-tête
//!   (`sex='Gender'`, `mean='Average'`). Sans libellé explicite, le LABEL
//!   stocké de la variable (VarMeta) sert de défaut. Sans ni l'un ni l'autre,
//!   le rendu reste byte-identique (nom/mot-clé brut).
//! - `*F=<fmt>` (collé sur un atome, p. ex. `mean*f=8.2`) fixe le format de la
//!   cellule numérique. Combiné à l'option `format=<fmt>` du statement PROC
//!   (défaut de table), via le moteur `src/formats`.
//! - Un NAME qui est une variable CLASS s'étend en ses niveaux observés.
//! - Un NAME qui est une variable VAR est une variable d'analyse.
//! - Un STATKW est un mot-clé statistique (voir plus bas).
//! - Les parenthèses groupent une sous-`dimexpr` ; l'empilage à l'intérieur
//!   produit des alternatives, et un croisement `A*( B C )` distribue sur
//!   chaque alternative → `A*B`, `A*C` (produit cartésien des facteurs).
//!
//! ### Mots-clés statistiques supportés (mappés sur `means::compute`)
//! `N`, `NMISS`, `SUM`, `MEAN`, `MIN`, `MAX`, `STD`.
//! - Statistique par défaut quand une VAR apparaît sans stat explicite :
//!   `SUM`.
//! - Cellule CLASS seule (sans VAR ni stat) : défaut `N` (effectif).
//! - `PCTN` / `PCTSUM` : pourcentages. `PCTN` = 100·n_cellule / N_dénominateur ;
//!   `PCTSUM` = 100·sum_cellule / SUM_dénominateur. En v1 le dénominateur est
//!   le TOTAL GÉNÉRAL (grand total : toutes les observations, resp. la somme de
//!   la VAR sur toutes les observations). Les dénominateurs de groupe
//!   (`PCTN<row>`) sont DIFFÉRÉS — `<` après un mot-clé → ERROR au parsing
//!   (J03-P2). `pctn(x)` n'est PAS un dénominateur : SAS lit la concaténation
//!   de PCTN et du groupe `(x)` (doc TABLE statement : opérateurs blanc,
//!   `*`, `()` et `<>`), comme sasrs. Dénominateur nul → cellule « . ».
//! - Les autres mots-clés de la doc (COLPCTN, MEDIAN, STDDEV…) → ERROR au
//!   parsing, avant toute sortie (`contract::UNSUPPORTED_STATS`).
//!
//! ### ALL — classe universelle (totaux marginaux)
//! Le mot-clé `ALL` dans une dimension ajoute une catégorie « total marginal » :
//! une ligne/colonne agrégée sur TOUTES les catégories de la dimension (aucune
//! contrainte CLASS). `ALL` peut être croisé avec une VAR et/ou une stat
//! (`ALL*MEAN`). Libellé affiché : « All ».
//!
//! ### Croisements supportés en v1
//! `class`, `var`, `stat` (seuls), `class*class`, `class*stat`, `var*stat`,
//! `class*var*stat`, et toute combinaison équivalente après distribution des
//! parenthèses. Contraintes vérifiées sur chaque cellule étendue :
//!   - AU PLUS une variable VAR (analyse).
//!   - AU PLUS une statistique explicite.
//!   - zéro ou plusieurs variables CLASS (croisées = catégories imbriquées).
//!
//! Une cellule qui viole ces règles (p. ex. deux VAR croisées, ou deux
//! stats) → erreur « PROC TABULATE: <construct> not yet supported ». Toutes
//! les cellules sont calculées AVANT d'écrire la moindre ligne du listing :
//! une erreur ne laisse ni titre ni en-tête de procédure orphelins (J02-P7).
//!
//! ### COUVERT en M33.4
//! - Libellés d'en-tête `='texte'` + LABEL stocké des variables (défaut).
//! - `format=<fmt>` (statement PROC) et `*F=<fmt>` (par cellule) via
//!   `src/formats`.
//! - `out=lib.ds` : dataset de cellules style SAS (voir plus bas).
//!
//! ### DÉFÉRÉ (documenté + erreur propre, jamais silencieux)
//! - `KEYLABEL` (WARNING), `BOX=`, `RTS=`, dénominateurs de groupe
//!   `PCTN<...>` (ERROR, J03-P2), plusieurs TABLE (ERROR, J03-P2). Tout
//!   mot-clé/atome non reconnu dans `table` → erreur
//!   « PROC TABULATE: <construct> not yet supported ». Toute option de
//!   statement inconnue (sur `proc tabulate` ou un sous-statement non géré)
//!   → erreur de parse.
//!
//! ### Calcul des cellules
//! Pour chaque (catégorie-ligne, catégorie-colonne) issue du croisement des
//! niveaux CLASS, on sélectionne les lignes du dataset où TOUTES les
//! variables CLASS de la cellule valent les niveaux requis (intersection),
//! puis on calcule la statistique demandée sur les valeurs NON manquantes de
//! la VAR (`common::partition_numeric`) — `N`/`NMISS` sont des comptes.
//! Cellule indéfinie / aucune ligne → `.`.
//!
//! ### Rendu (simplifié vs SAS)
//! On rend une table monospace via `ListingWriter::write_table` : une colonne
//! « stub » nomme la catégorie de ligne (ou « Table » s'il n'y a pas de
//! dimension ligne), puis une colonne par cellule de la dimension colonne.
//! L'en-tête de colonne concatène les composantes (niveaux CLASS, nom de VAR,
//! libellé de stat) séparées par « * ». C'est volontairement plus plat que
//! l'en-tête « boîte » multi-niveaux de SAS — documenté.
//!
//! ### OUT= dataset (M33.4) — convention de nommage choisie
//! `proc tabulate data=… out=lib.ds;` produit un dataset de cellules : UNE
//! observation par cellule (combinaison ligne×colonne, et page si présente).
//! Colonnes, dans l'ordre :
//!   - chaque variable CLASS impliquée dans la table (valeur du niveau de la
//!     cellule, ou MANQUANTE/blanc quand cette CLASS n'est pas active pour la
//!     cellule courante — comme MEANS) ;
//!   - `_TYPE_` (char) : motif `0`/`1` sur les variables CLASS (1 = active) ;
//!   - `_PAGE_` (num) : numéro de page (1 sans dimension page) ;
//!   - `_TABLE_` (num) : numéro de table (toujours 1 en v1) ;
//!   - une colonne numérique PAR STAT calculée : nom `<var>_<STAT>` quand une
//!     VAR d'analyse est présente (p. ex. `height_Mean`), sinon `<STAT>` seul
//!     pour les cellules de pure fréquence (p. ex. `N`, `PctN`). `<STAT>` est
//!     le libellé renvoyé par `tab_stat_header` (Mean, Sum, N, …).
//!
//! Approximation documentée vs SAS (docs/support-contract.md, « Approximations
//! documentées », NOTE à chaque exécution avec OUT=, jusqu'à J03-P2) : SAS
//! écrit UNE observation par combinaison de valeurs CLASS avec TOUTES ses
//! statistiques (noms `PctN_xx`/`PctSum_xx`) ; sasrs écrit une ligne par
//! cellule rendue et par statistique, les autres colonnes de stat manquantes.

#![allow(dead_code)]

use crate::ast::DatasetRef;
use crate::dataset::{SasDataset, VarMeta};
use crate::error::{Result, SasError};
use crate::formats::FormatSpec;
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column, partition_numeric};
use crate::procs::means::{compute, stat_header};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{Value, VarType, format_best};
use polars::prelude::*;
use std::cmp::Ordering;

mod cell;
mod contract;
mod model;
mod output;
mod parse;

pub use parse::parse;

use cell::*;
use model::*;
use output::*;
use parse::*;

pub struct TabulateAst {
    pub data: Option<DatasetRef>,
    /// J02-P7 — PROC option MISSING: missing CLASS values are valid levels.
    /// Without it, an observation with a missing value of any CLASS variable
    /// is excluded from the table (cells, ALL and denominators).
    missing: bool,
    class: Vec<String>,
    var: Vec<String>,
    /// J08-P2 — BY v1 [DESCENDING v2] ; : table par groupe (données triées
    /// par les variables BY, sinon ERROR — voir `common::by_groups`).
    by: Vec<(String, bool)>,
    /// Page dimension (None unless three comma-separated dimensions given).
    page: Option<DimExpr>,
    /// Row dimension (None when only a column dimension was given).
    row: Option<DimExpr>,
    /// Column dimension (always present).
    col: DimExpr,
    /// Table-level default cell format from `format=<fmt>` (M33.4). `None`
    /// keeps the byte-identical default rendering.
    format: Option<String>,
    /// `out=lib.ds` cell dataset target (M33.4). `None` → no output dataset.
    out: Option<DatasetRef>,
}

/// Parse the body of a TABLE statement (after `table` consumed), up to the
/// terminating `;` (NOT consumed). Returns (page, row, column) dimensions.
/// One dimension → columns only; two → rows, columns; three → page, rows,
/// columns. A fourth → clean error.
type ParsedTable = (Option<DimExpr>, Option<DimExpr>, DimExpr);

fn parse_table_statement(ts: &mut StatementStream) -> Result<ParsedTable> {
    // Parse comma-separated dimensions.
    let mut dims: Vec<DimExpr> = Vec::new();
    dims.push(parse_dimexpr(ts)?);
    while ts.peek().kind == TokenKind::Comma {
        ts.next();
        dims.push(parse_dimexpr(ts)?);
    }

    match dims.len() {
        1 => {
            let col = dims.pop().unwrap();
            Ok((None, None, col))
        }
        2 => {
            let col = dims.pop().unwrap();
            let row = dims.pop().unwrap();
            Ok((None, Some(row), col))
        }
        3 => {
            let col = dims.pop().unwrap();
            let row = dims.pop().unwrap();
            let page = dims.pop().unwrap();
            Ok((Some(page), Some(row), col))
        }
        _ => Err(SasError::runtime(
            "PROC TABULATE: a TABLE statement supports at most 3 dimensions",
        )),
    }
}

// ───────────────────────── execute ─────────────────────────

pub fn execute(ast: &TabulateAst, session: &mut Session) -> Result<()> {
    let (ds, in_libref, in_table) = common::open_input(&ast.data, session)?;
    let n_obs = ds.n_obs();

    // Resolve CLASS and VAR columns (validate existence; VAR must be numeric).
    let mut class_cols: Vec<(String, usize)> = Vec::with_capacity(ast.class.len());
    for cname in &ast.class {
        match ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(cname))
        {
            Some(i) => class_cols.push((ds.vars[i].name.clone(), i)),
            None => {
                return Err(SasError::runtime(format!(
                    "Variable {} not found.",
                    cname.to_uppercase()
                )));
            }
        }
    }
    let mut var_cols: Vec<(String, usize)> = Vec::with_capacity(ast.var.len());
    for vname in &ast.var {
        match ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(vname))
        {
            Some(i) => {
                if ds.vars[i].ty != VarType::Num {
                    return Err(SasError::runtime(format!(
                        "PROC TABULATE: analysis variable {} is not numeric (not yet supported)",
                        vname.to_uppercase()
                    )));
                }
                var_cols.push((ds.vars[i].name.clone(), i));
            }
            None => {
                return Err(SasError::runtime(format!(
                    "Variable {} not found.",
                    vname.to_uppercase()
                )));
            }
        }
    }

    // Clone the user-format catalog once so cell formatting (which borrows it)
    // does not clash with the mutable `session.listing` borrow below. Empty on
    // the default path → no behaviour change.
    let catalog = session.format_catalog.clone();

    // Decode every CLASS column (with its formatted levels, J02-P7) and every
    // VAR column once.
    let class_data: Vec<ClassData> = class_cols
        .iter()
        .map(|(_, ci)| ClassData::decode(&ds, *ci, &catalog))
        .collect::<Result<_>>()?;
    let mut var_values: Vec<(usize, Vec<Value>)> = Vec::with_capacity(var_cols.len());
    for (_, ci) in &var_cols {
        var_values.push((*ci, decode_column(&ds, *ci)?));
    }

    // --- BY processing (J08-P2) : resolve, verify sortedness, partition. ---
    // No BY → a single group spanning all rows (output byte-identical).
    let by_cols = common::resolve_by_cols(&ds, &ast.by)?;
    let by_values: Vec<Vec<Value>> = by_cols
        .iter()
        .map(|c| decode_column(&ds, c.col_idx))
        .collect::<Result<_>>()?;
    let by_groups_list: Vec<(Vec<Value>, Vec<usize>)> = if by_cols.is_empty() {
        vec![(Vec::new(), (0..n_obs).collect())]
    } else {
        let descending: Vec<bool> = by_cols.iter().map(|c| c.descending).collect();
        let by_names: Vec<String> = by_cols.iter().map(|c| c.name.clone()).collect();
        let in_display = format!("{in_libref}.{in_table}");
        common::by_groups(&by_values, &descending, n_obs, &by_names, &in_display)?
    };
    let by_names: Vec<String> = by_cols.iter().map(|c| c.name.clone()).collect();

    let table_format = ast.format.as_deref();

    // J02-P7 — the OUT= library is checked before any output: an unassigned
    // libref used to fail after the whole listing had been written.
    if let Some(out) = &ast.out {
        session.libs.get(&out.libref_or_work())?;
    }

    // --- OUT= cell dataset (M33.4, BY en J08-P2) : cellules calculées par
    // groupe, assemblées après la boucle de rendu. ---
    let want_out = ast.out.is_some();
    let mut out_groups: Vec<(Vec<Value>, Vec<OutCell>)> = Vec::new();

    // J02-P7 — every cell is computed before the first listing line: an
    // unsupported crossing used to stop the step after the titles and the
    // procedure heading had been written.
    let mut ops: Vec<ListingOp> = Vec::new();

    for (by_key, rows) in &by_groups_list {
        // BY group heading, SAS style (J08-P2) — skipped when no BY.
        if !by_names.is_empty() {
            let parts: Vec<String> = by_names
                .iter()
                .zip(by_key)
                .map(|(name, v)| format!("{}={}", name, by_heading_cell(v)))
                .collect();
            ops.push(ListingOp::Line(parts.join(" ")));
            ops.push(ListingOp::Blank);
        }

        // J02-P7 — an observation with a missing value of ANY CLASS variable
        // is excluded from the whole table — cells, ALL and the PCTN/PCTSUM
        // denominators — unless MISSING (SAS 9.4, CLASS statement, MISSING:
        // « If you omit the MISSING option, then PROC TABULATE excludes the
        // observations with any missing CLASS variable values »). It used to
        // be left out only of the cells that constrain that variable.
        let rows: Vec<usize> = rows
            .iter()
            .copied()
            .filter(|&r| ast.missing || class_data.iter().all(|cd| !cd.values[r].is_missing()))
            .collect();

        // Per-group data: CLASS/VAR columns filtered to the group's rows (row
        // indices become 0..n_g within the group) so the dimension expansion
        // and cell computation apply unchanged.
        let n_g = rows.len();
        let class_data_g: Vec<ClassData> = class_data.iter().map(|cd| cd.select(&rows)).collect();
        let var_values_g: Vec<(usize, Vec<Value>)> = var_values
            .iter()
            .map(|(c, v)| (*c, rows.iter().map(|&r| v[r].clone()).collect()))
            .collect();

        // Expand column and (optional) row dimensions into cell lists. With
        // BY, only the levels OBSERVED in the group appear (comportement SAS).
        let col_cells = expand_dim(&ast.col, &class_cols, &var_cols, &class_data_g, n_g)?;
        let row_cells: Vec<Cell> = match &ast.row {
            Some(r) => expand_dim(r, &class_cols, &var_cols, &class_data_g, n_g)?,
            None => vec![Cell { atoms: Vec::new() }], // single anonymous row
        };

        // Expand the (optional) page dimension. Without a page dimension we render
        // a single, page-less section (byte-identical to the pre-page behaviour).
        let page_cells: Vec<Option<Cell>> = match &ast.page {
            Some(p) => expand_dim(p, &class_cols, &var_cols, &class_data_g, n_g)?
                .into_iter()
                .map(Some)
                .collect(),
            None => vec![None],
        };

        for page in &page_cells {
            // Page label line (only when a page dimension is present).
            if let Some(pc) = page {
                ops.push(ListingOp::Line(format!(
                    "{}={}",
                    page_dim_name(ast, &ds),
                    cell_label(pc, &ds)
                )));
                ops.push(ListingOp::Blank);
            }
            let page_atoms: &[Atom] = match page {
                Some(pc) => &pc.atoms,
                None => &[],
            };

            // Build this section's listing table.
            let mut headers: Vec<String> = Vec::with_capacity(col_cells.len() + 1);
            let stub_title = match &ast.row {
                Some(_) => String::new(),
                None => "Table".to_string(),
            };
            headers.push(stub_title);
            for cc in &col_cells {
                headers.push(cell_label(cc, &ds));
            }
            let mut aligns: Vec<Align> = vec![Align::Left];
            aligns.extend(std::iter::repeat_n(Align::Right, col_cells.len()));

            let mut rows_out: Vec<Vec<String>> = Vec::with_capacity(row_cells.len());
            for rc in &row_cells {
                let stub = if rc.atoms.is_empty() {
                    String::new()
                } else {
                    cell_label(rc, &ds)
                };
                let mut out_row: Vec<String> = vec![stub];
                for cc in &col_cells {
                    // Merge page + row + column cell atoms.
                    let merged: Vec<Atom> = page_atoms
                        .iter()
                        .chain(rc.atoms.iter())
                        .chain(cc.atoms.iter())
                        .cloned()
                        .collect();
                    let value = compute_cell(
                        &merged,
                        &var_values_g,
                        &class_data_g,
                        n_g,
                        table_format,
                        &catalog,
                    )?;
                    out_row.push(value);
                }
                rows_out.push(out_row);
            }

            ops.push(ListingOp::Table {
                headers,
                aligns,
                rows: rows_out,
            });
            if page.is_some() {
                ops.push(ListingOp::Blank);
            }
        }

        if want_out {
            let cells = compute_out_cells(
                &ds,
                &class_cols,
                &var_values_g,
                &class_data_g,
                &page_cells,
                &row_cells,
                &col_cells,
                n_g,
            )?;
            out_groups.push((by_key.clone(), cells));
        }
    }

    // --- listing: written once every cell is known ---
    session.listing.page_header();
    let title = "The TABULATE Procedure";
    let ls = session.listing.ls();
    let pad = ls.saturating_sub(title.len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), title));
    session.listing.blank();
    for op in &ops {
        match op {
            ListingOp::Line(line) => session.listing.write_line(line),
            ListingOp::Blank => session.listing.blank(),
            ListingOp::Table {
                headers,
                aligns,
                rows,
            } => session.listing.write_table(headers, aligns, rows),
        }
    }

    // --- OUT= cell dataset (M33.4) ---
    if let Some(out) = &ast.out {
        // J02-P7 — documented approximation of the SAS OUT= layout, signalled
        // at every execution until J03-P2 (docs/support-contract.md).
        session.log.note(contract::OUT_APPROXIMATION_NOTE);
        write_out_dataset(session, &ds, &class_cols, &by_cols, &out_groups, out)?;
    } else {
        // No OUT= → do NOT touch session.last_dataset (byte-identical default).
    }
    Ok(())
}

/// One listing operation of the table, replayed once the whole table has been
/// computed (J02-P7: no partial output before an error).
enum ListingOp {
    Line(String),
    Blank,
    Table {
        headers: Vec<String>,
        aligns: Vec<Align>,
        rows: Vec<Vec<String>>,
    },
}

/// BY-key cell rendering for the group heading line (J08-P2) — same
/// convention as `common::by::by_cell` (char trimmed, BEST12. for nums).
fn by_heading_cell(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.trim_end().to_string(),
    }
}

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod tests;
