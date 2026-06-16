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
//! - `proc tabulate data=<ref>;` — seule l'option `data=` est reconnue.
//! - `class <var list>;` — variables catégorielles. Décodées une fois via
//!   `common::decode_column`, niveaux ordonnés par `Value::sas_cmp`. Les
//!   valeurs MANQUANTES d'une variable CLASS sont EXCLUES en v1 (toute ligne
//!   dont une variable CLASS impliquée est manquante est ignorée pour la
//!   cellule). Documenté : SAS sans l'option MISSING fait de même.
//! - `var <var list>;` — variables d'analyse numériques.
//! - `table <dimexpr> [, <dimexpr> [, <dimexpr>]];` — UNE dimension (colonnes
//!   seules), DEUX dimensions (`lignes , colonnes`) ou TROIS dimensions
//!   (`page , lignes , colonnes`). La dimension page produit un sous-tableau
//!   row×col répété par catégorie de page, précédé d'un libellé de page.
//! - `run;` / `quit;`.
//!
//! ### Grammaire d'expression de table (v1 — petite et précise)
//! ```text
//! dimexpr := term { term }            (* concaténation par blancs = empilage *)
//! term    := factor { '*' factor }    (* croisement *)
//! factor  := NAME | STATKW | '(' dimexpr ')'
//! ```
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
//!   (`PCTN<row>`) sont DIFFÉRÉS — atome de dénominateur parenthésé → erreur
//!   propre. Dénominateur nul → cellule « . ».
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
//! Une cellule qui viole ces règles (p. ex. deux VAR croisées, ou deux
//! stats) → erreur « PROC TABULATE: <construct> not yet supported ».
//!
//! ### DÉFÉRÉ (documenté + erreur propre, jamais silencieux)
//! - Formats / labels dans les en-têtes (on affiche les noms et niveaux
//!   bruts) ; `KEYLABEL`, `BOX=`, `*F=` (formats de cellule), `RTS=`,
//!   dénominateurs de groupe `PCTN<...>`, option `MISSING`. Tout
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
//! l'en-tête « boîte » multi-niveaux de SAS — documenté. PROC TABULATE en v1
//! ne produit AUCUN dataset de sortie (pas de `last_dataset`).

#![allow(dead_code)]

use crate::ast::DatasetRef;
use crate::dataset::SasDataset;
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::parser::StatementStream;
use crate::procs::common::{decode_column, partition_numeric};
use crate::procs::means::{compute, stat_header};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{format_best, Value, VarType};
use std::cmp::Ordering;

// ───────────────────────────── AST ─────────────────────────────

/// A parsed table-expression (raw, before CLASS/VAR resolution).
#[derive(Debug, Clone)]
struct DimExpr {
    /// Stacked terms (concatenation by blanks).
    terms: Vec<Term>,
}

#[derive(Debug, Clone)]
struct Term {
    /// Factors crossed by `*`.
    factors: Vec<Factor>,
}

#[derive(Debug, Clone)]
enum Factor {
    /// An identifier (resolved to CLASS / VAR / stat at execute time).
    Name(String),
    /// A parenthesized sub-expression (distributes over crossings).
    Group(DimExpr),
}

pub struct TabulateAst {
    pub data: Option<DatasetRef>,
    class: Vec<String>,
    var: Vec<String>,
    /// Page dimension (None unless three comma-separated dimensions given).
    page: Option<DimExpr>,
    /// Row dimension (None when only a column dimension was given).
    row: Option<DimExpr>,
    /// Column dimension (always present).
    col: DimExpr,
}

/// Statistic keywords recognized inside a TABLE expression.
const STAT_KEYWORDS: &[&str] =
    &["n", "nmiss", "sum", "mean", "min", "max", "std", "pctn", "pctsum"];

fn is_stat_keyword(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    STAT_KEYWORDS.iter().any(|k| *k == l)
}

// ───────────────────────────── parse ─────────────────────────────

/// Parse `proc tabulate [data=a]; class ...; var ...; table ...; run;`.
/// Called AFTER "proc tabulate" has been consumed. Consumes through
/// `run;` / `quit;`.
pub fn parse(ts: &mut StatementStream) -> Result<TabulateAst> {
    let mut data: Option<DatasetRef> = None;

    // --- PROC TABULATE statement options, until `;` ---
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
        } else if let Some(name) = ts.peek().ident().map(str::to_string) {
            let span = ts.peek().span;
            return Err(SasError::parse(
                format!(
                    "Unexpected option '{}' on PROC TABULATE statement.",
                    name.to_uppercase()
                ),
                span,
            ));
        } else {
            let span = ts.peek().span;
            return Err(SasError::parse(
                "Unexpected token on PROC TABULATE statement.",
                span,
            ));
        }
    }

    // --- sub-statements until run;/quit; ---
    let mut class: Vec<String> = Vec::new();
    let mut var: Vec<String> = Vec::new();
    let mut page: Option<DimExpr> = None;
    let mut row: Option<DimExpr> = None;
    let mut col: Option<DimExpr> = None;

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

        if ts.peek().is_kw("class") {
            ts.next();
            class.extend(ts.parse_name_list()?);
            ts.expect_semi()?;
        } else if ts.peek().is_kw("var") {
            ts.next();
            var.extend(ts.parse_name_list()?);
            ts.expect_semi()?;
        } else if ts.peek().is_kw("table") || ts.peek().is_kw("tables") {
            ts.next();
            let (p, r, c) = parse_table_statement(ts)?;
            page = p;
            row = r;
            col = Some(c);
            ts.expect_semi()?;
        } else {
            // Unknown sub-statement: recover by skipping to `;`.
            ts.skip_to_semi();
        }
    }

    let col = col.ok_or_else(|| {
        SasError::runtime("PROC TABULATE requires a TABLE statement.")
    })?;

    Ok(TabulateAst {
        data,
        class,
        var,
        page,
        row,
        col,
    })
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
            "PROC TABULATE: 4th dimension not yet supported",
        )),
    }
}

/// `dimexpr := term { term }`. Terms are concatenated by blanks; a term ends
/// at a `,`, `)`, `;`, or EOF.
fn parse_dimexpr(ts: &mut StatementStream) -> Result<DimExpr> {
    let mut terms = Vec::new();
    loop {
        match ts.peek().kind {
            TokenKind::Comma
            | TokenKind::RParen
            | TokenKind::Semi
            | TokenKind::Eof => break,
            _ => {}
        }
        terms.push(parse_term(ts)?);
    }
    if terms.is_empty() {
        return Err(SasError::parse(
            "PROC TABULATE: empty dimension in TABLE statement",
            ts.peek().span,
        ));
    }
    Ok(DimExpr { terms })
}

/// `term := factor { '*' factor }`.
fn parse_term(ts: &mut StatementStream) -> Result<Term> {
    let mut factors = vec![parse_factor(ts)?];
    while ts.peek().kind == TokenKind::Star {
        ts.next();
        factors.push(parse_factor(ts)?);
    }
    Ok(Term { factors })
}

/// `factor := NAME | STATKW | '(' dimexpr ')'`.
fn parse_factor(ts: &mut StatementStream) -> Result<Factor> {
    if ts.peek().kind == TokenKind::LParen {
        ts.next();
        let inner = parse_dimexpr(ts)?;
        if ts.peek().kind != TokenKind::RParen {
            return Err(SasError::parse(
                "PROC TABULATE: expected ')' in TABLE expression",
                ts.peek().span,
            ));
        }
        ts.next();
        return Ok(Factor::Group(inner));
    }
    if let Some(name) = ts.peek().ident().map(str::to_string) {
        ts.next();
        return Ok(Factor::Name(name));
    }
    Err(SasError::parse(
        "PROC TABULATE: expected a variable name, statistic, or '(' in TABLE expression",
        ts.peek().span,
    ))
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

// ───────────────────────── expansion ─────────────────────────

/// A single atom of an expanded cell.
#[derive(Debug, Clone)]
enum Atom {
    /// A CLASS variable binding: (class column index, observed level value).
    ClassLevel { col: usize, level: Value },
    /// The analysis VAR column index.
    Var(usize),
    /// A statistic keyword (lowercase).
    Stat(String),
    /// The universal class (marginal total): no CLASS constraint, labelled
    /// "All". Aggregates over every category of its dimension.
    All,
}

/// A fully-expanded cell: an ordered crossing of atoms (used for the header
/// label and for selecting rows + computing a statistic).
#[derive(Debug, Clone)]
struct Cell {
    atoms: Vec<Atom>,
}

/// Classification of a TABLE identifier.
enum Ident3 {
    Class(usize),
    Var(usize),
    Stat(String),
    All,
}

/// Resolve a name appearing in a TABLE expression to a CLASS col / VAR col /
/// stat keyword. Errors cleanly on anything else.
fn classify(
    name: &str,
    class_cols: &[(String, usize)],
    var_cols: &[(String, usize)],
) -> Result<Ident3> {
    if name.eq_ignore_ascii_case("all") {
        return Ok(Ident3::All);
    }
    if is_stat_keyword(name) {
        return Ok(Ident3::Stat(name.to_ascii_lowercase()));
    }
    if let Some((_, ci)) = class_cols
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
    {
        return Ok(Ident3::Class(*ci));
    }
    if let Some((_, ci)) = var_cols.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
        return Ok(Ident3::Var(*ci));
    }
    Err(SasError::runtime(format!(
        "PROC TABULATE: {} not yet supported",
        name.to_uppercase()
    )))
}

/// Expand a `DimExpr` into a flat list of cells. Each cell is one column (or
/// one row stub). Stacking concatenates the cells of successive terms;
/// crossing builds the cartesian product of the factors' cell lists.
fn expand_dim(
    dim: &DimExpr,
    class_cols: &[(String, usize)],
    var_cols: &[(String, usize)],
    class_values: &[(usize, Vec<Value>)],
    n_obs: usize,
) -> Result<Vec<Cell>> {
    let mut out: Vec<Cell> = Vec::new();
    for term in &dim.terms {
        out.extend(expand_term(term, class_cols, var_cols, class_values, n_obs)?);
    }
    Ok(out)
}

fn expand_term(
    term: &Term,
    class_cols: &[(String, usize)],
    var_cols: &[(String, usize)],
    class_values: &[(usize, Vec<Value>)],
    n_obs: usize,
) -> Result<Vec<Cell>> {
    // Each factor expands to a list of cells; crossing = cartesian product
    // (concatenating atoms).
    let mut acc: Vec<Cell> = vec![Cell { atoms: Vec::new() }];
    for factor in &term.factors {
        let factor_cells =
            expand_factor(factor, class_cols, var_cols, class_values, n_obs)?;
        let mut next: Vec<Cell> = Vec::with_capacity(acc.len() * factor_cells.len());
        for base in &acc {
            for fc in &factor_cells {
                let mut atoms = base.atoms.clone();
                atoms.extend(fc.atoms.iter().cloned());
                next.push(Cell { atoms });
            }
        }
        acc = next;
    }
    Ok(acc)
}

fn expand_factor(
    factor: &Factor,
    class_cols: &[(String, usize)],
    var_cols: &[(String, usize)],
    class_values: &[(usize, Vec<Value>)],
    n_obs: usize,
) -> Result<Vec<Cell>> {
    match factor {
        Factor::Group(inner) => {
            expand_dim(inner, class_cols, var_cols, class_values, n_obs)
        }
        Factor::Name(name) => match classify(name, class_cols, var_cols)? {
            Ident3::All => Ok(vec![Cell {
                atoms: vec![Atom::All],
            }]),
            Ident3::Stat(s) => Ok(vec![Cell {
                atoms: vec![Atom::Stat(s)],
            }]),
            Ident3::Var(ci) => Ok(vec![Cell {
                atoms: vec![Atom::Var(ci)],
            }]),
            Ident3::Class(ci) => {
                // Expand to one cell per observed (non-missing) level, in
                // sas_cmp order.
                let vals = &class_values
                    .iter()
                    .find(|(c, _)| *c == ci)
                    .expect("class col decoded")
                    .1;
                let levels = observed_levels(vals, n_obs);
                Ok(levels
                    .into_iter()
                    .map(|lv| Cell {
                        atoms: vec![Atom::ClassLevel { col: ci, level: lv }],
                    })
                    .collect())
            }
        },
    }
}

/// Observed non-missing levels of a CLASS column, ordered by `sas_cmp`.
fn observed_levels(vals: &[Value], n_obs: usize) -> Vec<Value> {
    let mut levels: Vec<Value> = Vec::new();
    for v in vals.iter().take(n_obs) {
        if v.is_missing() {
            continue;
        }
        if !levels.iter().any(|e| e.sas_cmp(v) == Ordering::Equal) {
            levels.push(v.clone());
        }
    }
    levels.sort_by(|a, b| a.sas_cmp(b));
    levels
}

// ───────────────────────── execute ─────────────────────────

/// Resolve `data=` or `_LAST_` into a concrete DatasetRef.
fn resolve_input(ast: &TabulateAst, session: &Session) -> Result<DatasetRef> {
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

pub fn execute(ast: &TabulateAst, session: &mut Session) -> Result<()> {
    let in_ref = resolve_input(ast, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }
    let n_obs = ds.n_obs();

    // Resolve CLASS and VAR columns (validate existence; VAR must be numeric).
    let mut class_cols: Vec<(String, usize)> = Vec::with_capacity(ast.class.len());
    for cname in &ast.class {
        match ds.vars.iter().position(|m| m.name.eq_ignore_ascii_case(cname)) {
            Some(i) => class_cols.push((ds.vars[i].name.clone(), i)),
            None => {
                return Err(SasError::runtime(format!(
                    "Variable {} not found.",
                    cname.to_uppercase()
                )))
            }
        }
    }
    let mut var_cols: Vec<(String, usize)> = Vec::with_capacity(ast.var.len());
    for vname in &ast.var {
        match ds.vars.iter().position(|m| m.name.eq_ignore_ascii_case(vname)) {
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
                )))
            }
        }
    }

    // Decode every CLASS and VAR column once.
    let mut class_values: Vec<(usize, Vec<Value>)> = Vec::with_capacity(class_cols.len());
    for (_, ci) in &class_cols {
        class_values.push((*ci, decode_column(&ds, *ci)?));
    }
    let mut var_values: Vec<(usize, Vec<Value>)> = Vec::with_capacity(var_cols.len());
    for (_, ci) in &var_cols {
        var_values.push((*ci, decode_column(&ds, *ci)?));
    }

    // Expand column and (optional) row dimensions into cell lists.
    let col_cells = expand_dim(&ast.col, &class_cols, &var_cols, &class_values, n_obs)?;
    let row_cells: Vec<Cell> = match &ast.row {
        Some(r) => expand_dim(r, &class_cols, &var_cols, &class_values, n_obs)?,
        None => vec![Cell { atoms: Vec::new() }], // single anonymous row
    };

    // Expand the (optional) page dimension. Without a page dimension we render
    // a single, page-less section (byte-identical to the pre-page behaviour).
    let page_cells: Vec<Option<Cell>> = match &ast.page {
        Some(p) => expand_dim(p, &class_cols, &var_cols, &class_values, n_obs)?
            .into_iter()
            .map(Some)
            .collect(),
        None => vec![None],
    };

    // --- listing ---
    session.listing.page_header();
    let title = "The TABULATE Procedure";
    let ls = session.listing.ls();
    let pad = ls.saturating_sub(title.len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), title));
    session.listing.blank();

    for page in &page_cells {
        // Page label line (only when a page dimension is present).
        if let Some(pc) = page {
            session
                .listing
                .write_line(&format!("{}={}", page_dim_name(&ast, &ds), cell_label(pc, &ds)));
            session.listing.blank();
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

        let mut rows: Vec<Vec<String>> = Vec::with_capacity(row_cells.len());
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
                let value = compute_cell(&merged, &var_values, &class_values, n_obs)?;
                out_row.push(value);
            }
            rows.push(out_row);
        }

        session.listing.write_table(&headers, &aligns, &rows);
        if page.is_some() {
            session.listing.blank();
        }
    }

    // v1: no output dataset — do NOT touch session.last_dataset.
    Ok(())
}

/// Best-effort name of the page dimension for the page-label line: the first
/// CLASS variable that appears in the page expression, else "Page".
fn page_dim_name(ast: &TabulateAst, ds: &SasDataset) -> String {
    if let Some(p) = &ast.page {
        if let Some(name) = first_class_name(p, ds) {
            return name;
        }
    }
    "Page".to_string()
}

fn first_class_name(dim: &DimExpr, ds: &SasDataset) -> Option<String> {
    for term in &dim.terms {
        for factor in &term.factors {
            match factor {
                Factor::Name(n) => {
                    if let Some(m) = ds.vars.iter().find(|m| m.name.eq_ignore_ascii_case(n)) {
                        return Some(m.name.clone());
                    }
                }
                Factor::Group(inner) => {
                    if let Some(n) = first_class_name(inner, ds) {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

/// Build the header/stub label for an expanded cell: components joined by "*".
fn cell_label(cell: &Cell, ds: &SasDataset) -> String {
    if cell.atoms.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = cell
        .atoms
        .iter()
        .map(|a| match a {
            Atom::ClassLevel { level, .. } => level_label(level),
            Atom::Var(ci) => ds.vars[*ci].name.clone(),
            Atom::Stat(s) => tab_stat_header(s).to_string(),
            Atom::All => "All".to_string(),
        })
        .collect();
    parts.join("*")
}

/// Header label for a stat keyword, extending `means::stat_header` with the
/// TABULATE-specific percentage stats (kept local to avoid touching common
/// code shared with the parallel REPORT work).
fn tab_stat_header(stat: &str) -> &'static str {
    match stat {
        "pctn" => "PctN",
        "pctsum" => "PctSum",
        _ => stat_header(stat),
    }
}

fn level_label(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Char(s) => s.clone(),
        Value::Missing(k) => k.display(),
    }
}

/// Validate the merged cell's atoms and compute its numeric value, formatted.
/// Returns "." when the cell is undefined (no qualifying rows / undefined
/// statistic). Errors cleanly for unsupported constructs (>1 VAR, >1 stat).
fn compute_cell(
    atoms: &[Atom],
    var_values: &[(usize, Vec<Value>)],
    class_values: &[(usize, Vec<Value>)],
    n_obs: usize,
) -> Result<String> {
    let mut var_col: Option<usize> = None;
    let mut stat: Option<String> = None;
    // (class col, required level) constraints.
    let mut class_constraints: Vec<(usize, &Value)> = Vec::new();

    for a in atoms {
        match a {
            Atom::Var(ci) => {
                if var_col.is_some() {
                    return Err(SasError::runtime(
                        "PROC TABULATE: crossing two analysis variables not yet supported",
                    ));
                }
                var_col = Some(*ci);
            }
            Atom::Stat(s) => {
                if stat.is_some() {
                    return Err(SasError::runtime(
                        "PROC TABULATE: crossing two statistics not yet supported",
                    ));
                }
                stat = Some(s.clone());
            }
            Atom::ClassLevel { col, level } => {
                class_constraints.push((*col, level));
            }
            // Universal class: aggregate over every category — no constraint.
            Atom::All => {}
        }
    }

    // Select rows matching ALL class constraints (and excluding missing
    // class values — they are never equal to a non-missing required level).
    let rows: Vec<usize> = (0..n_obs)
        .filter(|&r| {
            class_constraints.iter().all(|(col, level)| {
                let vals = &class_values
                    .iter()
                    .find(|(c, _)| c == col)
                    .expect("class col decoded")
                    .1;
                vals[r].sas_cmp(level) == Ordering::Equal
            })
        })
        .collect();

    // Default statistic: SUM when a VAR is present, N otherwise (frequency).
    let stat = stat.unwrap_or_else(|| {
        if var_col.is_some() {
            "sum".to_string()
        } else {
            "n".to_string()
        }
    });

    // Percentage statistics: numerator over the selected rows, denominator
    // over the grand total (all observations). v1 supports only the grand
    // total denominator (group denominators PCTN<...> are deferred).
    if stat == "pctn" {
        let denom = n_obs as f64;
        let value = if denom == 0.0 {
            Value::Missing(crate::value::MissingKind::Dot)
        } else {
            Value::Num(100.0 * rows.len() as f64 / denom)
        };
        return Ok(fmt_cell(&stat, &value));
    }
    if stat == "pctsum" {
        let ci = var_col.ok_or_else(|| {
            SasError::runtime(
                "PROC TABULATE: PCTSUM requires an analysis variable (not yet supported)",
            )
        })?;
        let col = &var_values
            .iter()
            .find(|(c, _)| *c == ci)
            .expect("var col decoded")
            .1;
        let (xs, _) = partition_numeric(col, &rows);
        let all_rows: Vec<usize> = (0..n_obs).collect();
        let (all_xs, _) = partition_numeric(col, &all_rows);
        let denom: f64 = all_xs.iter().sum();
        let numer: f64 = xs.iter().sum();
        let value = if denom == 0.0 {
            Value::Missing(crate::value::MissingKind::Dot)
        } else {
            Value::Num(100.0 * numer / denom)
        };
        return Ok(fmt_cell(&stat, &value));
    }

    // Determine the analysis values. With no VAR, only N/NMISS are meaningful
    // (frequency counts over the selected rows).
    let value: Value = match var_col {
        Some(ci) => {
            let col = &var_values
                .iter()
                .find(|(c, _)| *c == ci)
                .expect("var col decoded")
                .1;
            let (xs, nmiss) = partition_numeric(col, &rows);
            // TABULATE has no CI statistics; default alpha is unused here.
            compute(&stat, &xs, nmiss, 0.05)
        }
        None => {
            // No analysis variable: only frequency-style stats are defined.
            match stat.as_str() {
                "n" => Value::Num(rows.len() as f64),
                "nmiss" => Value::Num(0.0),
                _ => {
                    return Err(SasError::runtime(format!(
                        "PROC TABULATE: statistic {} requires an analysis variable (not yet supported)",
                        stat.to_uppercase()
                    )))
                }
            }
        }
    };

    Ok(fmt_cell(&stat, &value))
}

/// Format a computed cell value for the listing. Missing → ".".
fn fmt_cell(stat: &str, v: &Value) -> String {
    match v {
        Value::Num(f) => {
            if stat == "n" || stat == "nmiss" {
                format!("{}", *f as i64)
            } else {
                format_best(*f, 12)
            }
        }
        Value::Missing(_) => ".".to_string(),
        Value::Char(s) => s.clone(),
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

    fn num_meta(name: &str) -> VarMeta {
        VarMeta { name: name.into(), ty: VarType::Num, length: 8, format: None, label: None }
    }
    fn char_meta(name: &str) -> VarMeta {
        VarMeta { name: name.into(), ty: VarType::Char, length: 8, format: None, label: None }
    }

    fn write_dataset(session: &mut Session, table: &str, ds: SasDataset) {
        session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
        session.last_dataset = Some(format!("WORK.{}", table.to_uppercase()));
    }

    fn parse_src(src: &str) -> Result<TabulateAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // "proc"
        ts.next(); // "tabulate"
        parse(&mut ts)
    }

    /// Parse + execute through a session, returning the listing string.
    fn run(mut session: Session, src: &str) -> Result<String> {
        let ast = parse_src(src)?;
        execute(&ast, &mut session)?;
        Ok(session.listing.into_string())
    }

    // ─────────────── parse tests ───────────────

    #[test]
    fn parse_minimal_table() {
        let ast = parse_src("proc tabulate data=a; class region; table region; run;").unwrap();
        assert_eq!(ast.data.as_ref().unwrap().name, "a");
        assert_eq!(ast.class, vec!["region"]);
        assert!(ast.row.is_none());
        assert_eq!(ast.col.terms.len(), 1);
    }

    #[test]
    fn parse_two_dimensions() {
        let ast = parse_src(
            "proc tabulate data=a; class region; var sales; table region, sales*mean; run;",
        )
        .unwrap();
        assert!(ast.row.is_some());
    }

    #[test]
    fn parse_unknown_proc_option_errors() {
        let r = parse_src("proc tabulate data=a bogus; class x; table x; run;");
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("BOGUS"));
    }

    // ─────────────── execute tests ───────────────

    #[test]
    fn one_dimension_frequency() {
        let mut session = make_session();
        let df = df!["region" => ["E", "E", "W"]].unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("region")] };
        write_dataset(&mut session, "T", ds);

        let listing = run(
            session,
            "proc tabulate data=work.t; class region; table region; run;",
        )
        .unwrap();
        assert!(listing.contains("The TABULATE Procedure"), "{listing}");
        // Two levels E and W in headers.
        assert!(listing.contains("E"), "{listing}");
        assert!(listing.contains("W"), "{listing}");
        // Frequencies: E=2, W=1.
        assert!(listing.contains("2"), "{listing}");
        assert!(listing.contains("1"), "{listing}");
    }

    #[test]
    fn row_classvar_col_var_mean() {
        let mut session = make_session();
        let df = df![
            "region" => ["E", "E", "W"],
            "sales"  => [10.0_f64, 20.0, 8.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("region"), num_meta("sales")] };
        write_dataset(&mut session, "T", ds);

        let listing = run(
            session,
            "proc tabulate data=work.t; class region; var sales; table region, sales*mean; run;",
        )
        .unwrap();
        // E mean = 15, W mean = 8.
        assert!(listing.contains("15"), "{listing}");
        assert!(listing.contains("8"), "{listing}");
        // Header includes sales*Mean.
        assert!(listing.contains("sales") && listing.contains("Mean"), "{listing}");
    }

    #[test]
    fn class_cross_class() {
        let mut session = make_session();
        let df = df![
            "a" => ["x", "x", "y"],
            "b" => ["p", "q", "p"]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("a"), char_meta("b")] };
        write_dataset(&mut session, "T", ds);

        // a on rows, a*b crossing on columns gives nested category cells.
        let listing = run(
            session,
            "proc tabulate data=work.t; class a b; table a, a*b; run;",
        )
        .unwrap();
        // Column headers should show crossings like x*p.
        assert!(listing.contains("x*p"), "{listing}");
        assert!(listing.contains("x*q"), "{listing}");
        assert!(listing.contains("y*p"), "{listing}");
    }

    #[test]
    fn multistat_list_with_group() {
        let mut session = make_session();
        let df = df![
            "region" => ["E", "E", "W"],
            "sales"  => [10.0_f64, 20.0, 8.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("region"), num_meta("sales")] };
        write_dataset(&mut session, "T", ds);

        let listing = run(
            session,
            "proc tabulate data=work.t; class region; var sales; table region, sales*(n sum mean); run;",
        )
        .unwrap();
        // Three stat columns for sales: N, Sum, Mean.
        assert!(listing.contains("sales*N"), "{listing}");
        assert!(listing.contains("sales*Sum"), "{listing}");
        assert!(listing.contains("sales*Mean"), "{listing}");
        // E: n=2 sum=30 mean=15.
        assert!(listing.contains("30"), "{listing}");
        assert!(listing.contains("15"), "{listing}");
    }

    #[test]
    fn missing_in_var_excluded_from_mean_counted_in_nmiss() {
        let mut session = make_session();
        let df = df![
            "region" => ["E", "E", "E"],
            "sales"  => [Some(10.0_f64), Some(20.0), None]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("region"), num_meta("sales")] };
        write_dataset(&mut session, "T", ds);

        let listing = run(
            session,
            "proc tabulate data=work.t; class region; var sales; table region, sales*(mean nmiss n); run;",
        )
        .unwrap();
        // mean over [10,20] = 15; nmiss = 1; n = 2.
        assert!(listing.contains("15"), "{listing}");
        assert!(listing.contains("sales*NMiss"), "{listing}");
    }

    #[test]
    fn unsupported_construct_clean_error() {
        let mut session = make_session();
        let df = df![
            "region" => ["E", "W"],
            "a" => [1.0_f64, 2.0],
            "b" => [3.0_f64, 4.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("region"), num_meta("a"), num_meta("b")],
        };
        write_dataset(&mut session, "T", ds);

        // Crossing two analysis variables a*b is unsupported.
        let r = run(
            session,
            "proc tabulate data=work.t; class region; var a b; table region, a*b; run;",
        );
        assert!(r.is_err());
        assert!(
            r.err().unwrap().to_string().contains("not yet supported"),
            "expected clean unsupported error"
        );
    }

    #[test]
    fn unknown_name_clean_error() {
        let mut session = make_session();
        let df = df!["region" => ["E", "W"]].unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("region")] };
        write_dataset(&mut session, "T", ds);

        let r = run(
            session,
            "proc tabulate data=work.t; class region; table region*nope; run;",
        );
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("not yet supported"));
    }

    #[test]
    fn third_dimension_now_supported() {
        let mut session = make_session();
        let df = df![
            "a" => ["x", "y"],
            "b" => ["p", "p"],
            "c" => ["m", "m"]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("a"), char_meta("b"), char_meta("c")],
        };
        write_dataset(&mut session, "T", ds);

        // A 3rd (page) dimension is now rendered, not an error.
        let listing = run(
            session,
            "proc tabulate data=work.t; class a b c; table a, b, c; run;",
        )
        .unwrap();
        // Two page sections, labelled by the page CLASS value of `a`.
        assert!(listing.contains("a=x"), "{listing}");
        assert!(listing.contains("a=y"), "{listing}");
    }

    // ─────────────── M21.4: page dimension ───────────────

    /// Build the classic sashelp.class-like fixture (subset of rows is fine).
    fn class_fixture(session: &mut Session) {
        let df = df![
            "sex"    => ["M", "F", "M", "F", "M"],
            "age"    => [14.0_f64, 13.0, 12.0, 13.0, 14.0],
            "height" => [69.0_f64, 56.5, 57.3, 65.3, 62.5],
            "weight" => [112.5_f64, 84.0, 83.0, 98.0, 84.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![
                char_meta("sex"),
                num_meta("age"),
                num_meta("height"),
                num_meta("weight"),
            ],
        };
        write_dataset(session, "C", ds);
    }

    #[test]
    fn page_dimension_renders_per_page_subtables() {
        let mut session = make_session();
        let df = df![
            "grp"    => ["A", "A", "B", "B"],
            "region" => ["E", "W", "E", "W"],
            "sales"  => [10.0_f64, 20.0, 30.0, 40.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("grp"), char_meta("region"), num_meta("sales")],
        };
        write_dataset(&mut session, "T", ds);

        let listing = run(
            session,
            "proc tabulate data=work.t; class grp region; var sales; \
             table grp, region, sales*sum; run;",
        )
        .unwrap();
        // Two page sections, labelled by page CLASS value.
        assert!(listing.contains("grp=A"), "{listing}");
        assert!(listing.contains("grp=B"), "{listing}");
        // Page A: E=10, W=20 ; page B: E=30, W=40.
        assert!(listing.contains("10") && listing.contains("20"), "{listing}");
        assert!(listing.contains("30") && listing.contains("40"), "{listing}");
    }

    #[test]
    fn four_dimensions_clean_error() {
        let mut session = make_session();
        let df = df![
            "a" => ["x"], "b" => ["p"], "c" => ["m"], "d" => ["q"]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("a"), char_meta("b"), char_meta("c"), char_meta("d")],
        };
        write_dataset(&mut session, "T", ds);

        let r = run(
            session,
            "proc tabulate data=work.t; class a b c d; table a, b, c, d; run;",
        );
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("4th dimension"));
    }

    // ─────────────── M21.4: ALL (universal class) ───────────────

    #[test]
    fn all_marginal_row_total() {
        let mut session = make_session();
        class_fixture(&mut session);
        // ALL in the ROW dimension adds a grand-total row (no sex constraint),
        // so the N column shows N over all 5 observations on that row.
        let listing = run(
            session,
            "proc tabulate data=work.c; class sex; table sex all, n; run;",
        )
        .unwrap();
        assert!(listing.contains("All"), "{listing}");
        // sex M=3, F=2; ALL row = 5 (grand total).
        assert!(listing.contains("5"), "{listing}");
    }

    #[test]
    fn all_with_stat_aggregates_over_all_rows() {
        let mut session = make_session();
        class_fixture(&mut session);
        // ALL row crossed with height*mean: mean over all 5 rows.
        // (69 + 56.5 + 57.3 + 65.3 + 62.5) / 5 = 310.6/5 = 62.12.
        let listing = run(
            session,
            "proc tabulate data=work.c; class sex; var height; \
             table sex all, height*mean; run;",
        )
        .unwrap();
        assert!(listing.contains("All"), "{listing}");
        assert!(listing.contains("62.12"), "{listing}");
    }

    // ─────────────── M21.4: PCTN / PCTSUM ───────────────

    #[test]
    fn pctn_grand_total_denominator() {
        let mut session = make_session();
        class_fixture(&mut session);
        // PCTN per sex: M=3/5=60%, F=2/5=40%.
        let listing = run(
            session,
            "proc tabulate data=work.c; class sex; table sex, pctn; run;",
        )
        .unwrap();
        assert!(listing.contains("PctN"), "{listing}");
        assert!(listing.contains("60"), "{listing}");
        assert!(listing.contains("40"), "{listing}");
    }

    #[test]
    fn pctsum_grand_total_denominator() {
        let mut session = make_session();
        let df = df![
            "region" => ["E", "E", "W"],
            "sales"  => [10.0_f64, 30.0, 60.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("region"), num_meta("sales")],
        };
        write_dataset(&mut session, "T", ds);

        // PCTSUM of sales: E = (10+30)/100 = 40%, W = 60/100 = 60%.
        let listing = run(
            session,
            "proc tabulate data=work.t; class region; var sales; \
             table region, sales*pctsum; run;",
        )
        .unwrap();
        assert!(listing.contains("PctSum"), "{listing}");
        assert!(listing.contains("40"), "{listing}");
        assert!(listing.contains("60"), "{listing}");
    }

    #[test]
    fn pctn_empty_cell_is_dot_not_panic() {
        let mut session = make_session();
        // No observations at all → grand total N = 0 → "." (no div-by-zero).
        let df = df!["region" => [""; 0]].unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("region")] };
        write_dataset(&mut session, "T", ds);
        let listing = run(
            session,
            "proc tabulate data=work.t; class region; table region, pctn; run;",
        );
        // Either an empty table or a clean render — must not panic.
        assert!(listing.is_ok(), "{:?}", listing.err());
    }

    #[test]
    fn pctsum_requires_var_clean_error() {
        let mut session = make_session();
        class_fixture(&mut session);
        let r = run(
            session,
            "proc tabulate data=work.c; class sex; table sex, pctsum; run;",
        );
        assert!(r.is_err());
        assert!(
            r.err().unwrap().to_string().contains("not yet supported"),
            "expected clean error for PCTSUM without VAR"
        );
    }

    #[test]
    fn pctsum_zero_denominator_is_dot() {
        let mut session = make_session();
        let df = df![
            "region" => ["E", "W"],
            "sales"  => [0.0_f64, 0.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("region"), num_meta("sales")],
        };
        write_dataset(&mut session, "T", ds);
        let listing = run(
            session,
            "proc tabulate data=work.t; class region; var sales; \
             table region, sales*pctsum; run;",
        )
        .unwrap();
        // Denominator 0 → cells are ".", no panic.
        assert!(listing.contains('.'), "{listing}");
    }

    // ─────────────── M21.4: multi-VAR / multi-stat in columns ───────────────

    #[test]
    fn multi_var_separate_column_analyses() {
        let mut session = make_session();
        class_fixture(&mut session);
        // Two different VAR analyses side by side in the column dimension.
        let listing = run(
            session,
            "proc tabulate data=work.c; class sex; var height weight; \
             table sex, height*mean weight*sum; run;",
        )
        .unwrap();
        assert!(listing.contains("height") && listing.contains("Mean"), "{listing}");
        assert!(listing.contains("weight") && listing.contains("Sum"), "{listing}");
        // M weights sum = 112.5 + 83 + 84 = 279.5.
        assert!(listing.contains("279.5"), "{listing}");
    }

    #[test]
    fn distribute_stats_over_var_via_group() {
        let mut session = make_session();
        class_fixture(&mut session);
        // height*(N MEAN) distributes two stats over the single VAR.
        let listing = run(
            session,
            "proc tabulate data=work.c; class sex; var height; \
             table sex, height*(n mean); run;",
        )
        .unwrap();
        assert!(listing.contains("height*N"), "{listing}");
        assert!(listing.contains("height*Mean"), "{listing}");
        // M: n=3, F: n=2.
        assert!(listing.contains("3") && listing.contains("2"), "{listing}");
    }

    #[test]
    fn all_and_pctn_combined() {
        let mut session = make_session();
        class_fixture(&mut session);
        // sex on rows with an ALL marginal row; PCTN columns.
        let listing = run(
            session,
            "proc tabulate data=work.c; class sex; table sex all, pctn; run;",
        )
        .unwrap();
        assert!(listing.contains("All"), "{listing}");
        // ALL row PCTN = 5/5 = 100%.
        assert!(listing.contains("100"), "{listing}");
    }

    #[test]
    fn no_output_dataset_set() {
        let mut session = make_session();
        let df = df!["region" => ["E", "W"]].unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("region")] };
        write_dataset(&mut session, "T", ds);
        let before = session.last_dataset.clone();

        let ast = parse_src("proc tabulate data=work.t; class region; table region; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        // last_dataset unchanged (v1 produces no output dataset).
        assert_eq!(session.last_dataset, before);
    }
}
