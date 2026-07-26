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
//!     SUM MEAN MIN MAX N STD (défaut SUM).
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
//! ## FONCTIONNALITÉS AVANCÉES (M21.4) — désormais supportées :
//!   - usage `ACROSS` : les valeurs distinctes de la variable across deviennent
//!     des COLONNES ; cellule = stat de l'ANALYSIS var au croisement
//!     GROUP×ACROSS. v1 : exactement 1 across + 1 analysis ; en-tête à deux
//!     niveaux APLATI en une ligne "valeur STAT" (le listing n'a pas de
//!     spanner). OUT= sur un rapport ACROSS est différé proprement (note).
//!   - `WHERE <cond>;` : filtre les observations AVANT le rapport. Évaluateur
//!     d'expression local (ce fichier) fidèle SAS : comparaisons via
//!     `Value::sas_cmp` (`. = .` vrai, char insensible aux blancs finaux),
//!     logique sur la véracité SAS, `in (...)`. Appels de fonctions/arrays non
//!     gérés → missing de garde (pas de panic).
//!   - `BREAK AFTER <var> / summarize;` : ligne de sous-total recalculée
//!     (ANALYSIS via `means::compute`) après chaque changement du groupe.
//!   - `RBREAK AFTER / summarize;` : ligne de total général en bas. OL/DOL/
//!     SKIP/PAGE acceptés mais cosmétiques (no-op v1).
//!   - `COMPUTE <col>; <col> = <expr>; endcomp;` : affectation simple par ligne.
//!     `COMPUTE AFTER; line <items>; endcomp;` : ligne de texte libre. Les
//!     affectations et LINE peuvent référencer une colonne par son nom OU par
//!     l'alias positionnel `_Cn_` (M33.5). `line` accepte un pointeur `@<col>`
//!     et un format de fin (`line @5 total best8.;`).
//!   - `OUT=<ref>` : écrit les lignes du corps du rapport (détail/groupe +
//!     sous-totaux BREAK ; le total RBREAK est exclu) comme dataset, en
//!     respectant le type SAS de chaque colonne, et émet la NOTE de création.
//!
//! ## OPTIONS DEFINE AVANCÉES (M33.5) — désormais supportées :
//!   - `FORMAT=<fmt>` : applique un format SAS / `w.d` aux valeurs affichées de
//!     la colonne (numérique et char `$w.`), via `src/formats` (réutilise le
//!     moteur de M33.4/TABULATE). Sans format → rendu byte-identique.
//!   - `WIDTH=<n>` : largeur d'affichage de la colonne (troncature/padding de
//!     l'en-tête et des cellules ; numériques justifiés à droite, char à
//!     gauche).
//!   - `SPACING=<n>` : nombre d'espaces avant la colonne (défaut 2). Modifie le
//!     gap inter-colonnes du listing.
//!
//!   Le rendu width/spacing n'est activé que si AU MOINS un DEFINE porte
//!   WIDTH=/SPACING= ; sinon le chemin `ListingWriter::write_table` historique
//!   reste byte-identique.
//!
//! ## DEFERRALS RESTANTS (erreurs/notes PROPRES) — v1 ne supporte PAS :
//!   - `FLOW` (retour à la ligne des valeurs char longues) — interaction avec la
//!     hauteur de ligne ; différé PROPREMENT → "PROC REPORT v1 does not support
//!     the DEFINE option 'FLOW'." De même pour multi-label, etc.
//!   - COMPUTE non trivial AU-DELÀ de `_Cn_`/nom + LINE-avec-format : affectation
//!     back dans des colonnes calculées avec un riche jeu de fonctions n'est que
//!     partiellement couvert (l'évaluateur d'expression local gère les fonctions
//!     déjà disponibles ; le reste est différé).
//!   - options PROC autres que nowd/nowindow/noheader/headline/headskip/out=
//!     → "Unexpected option 'XXX' on PROC REPORT statement."

use crate::ast::{DatasetRef, Expr};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column, group_by_keys, partition_numeric};
use crate::procs::means;
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{Value, VarType, format_best};
use polars::prelude::{Column, DataFrame, NamedFrom, Series};
use std::cmp::Ordering;

mod compute;
mod output;
mod parse;
mod plan;
mod render;
mod rows;

pub use parse::parse;

use compute::*;
use output::*;
use plan::*;
use render::*;
use rows::*;

/// Usage of a column in the report.
#[derive(Debug, Clone, PartialEq)]
pub enum Usage {
    Display,
    Order,
    Group,
    /// ANALYSIS with a statistic keyword (sum/mean/min/max/n/std).
    Analysis(String),
    /// ACROSS: the distinct values of this variable become COLUMNS. The
    /// crossing of GROUP rows × ACROSS columns is filled with the statistic of
    /// the (single) ANALYSIS variable.
    Across,
    /// COMPUTED: a column produced by a `compute` block (`define x /
    /// computed;`); it has no underlying dataset variable.
    Computed,
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
    /// `format=<fmt>` — SAS format / `w.d` applied to this column's displayed
    /// values (M33.5). `None` keeps the byte-identical default rendering.
    pub format: Option<String>,
    /// `width=<n>` — display width of the column (M33.5). `None` lets the
    /// listing aligner derive the width from the data (default path).
    pub width: Option<usize>,
    /// `spacing=<n>` — number of blank spaces before the column (M33.5).
    /// `None` uses the default inter-column gap (2 spaces in SAS LISTING).
    pub spacing: Option<usize>,
}

pub struct ReportAst {
    pub data: Option<DatasetRef>,
    pub noheader: bool,
    /// COLUMN list (display order). `None` → all variables in dataset order.
    pub columns: Option<Vec<String>>,
    pub defines: Vec<Define>,
    /// `where <condition>;` — subsetting predicate applied before the report.
    pub where_: Option<Expr>,
    /// `out=<ref>` — write the report rows as a dataset.
    pub out: Option<DatasetRef>,
    /// `break after <var> / summarize;` — one summary line after each group.
    pub breaks: Vec<Break>,
    /// `rbreak after / summarize;` — a grand-total summary line at the bottom.
    pub rbreak: Option<Break>,
    /// `compute <target>; ... endcomp;` blocks.
    pub computes: Vec<Compute>,
}

/// A `break after <var> / summarize;` (BREAK) or `rbreak after / summarize;`
/// (RBREAK) statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Break {
    /// Group variable the break is taken after. `None` for RBREAK.
    pub var: Option<String>,
    /// `summarize`: recompute ANALYSIS stats over the break range.
    pub summarize: bool,
}

/// A `compute <target>; ... endcomp;` block. v1 supports `line` statements and
/// simple `<col> = <expr>;` assignments. The target identifies the location: a
/// column name, or `after`/`before` for report-level computes.
#[derive(Debug, Clone, PartialEq)]
pub struct Compute {
    /// `compute <target>;` — column name, or "after"/"before".
    pub target: String,
    /// Statements inside the block, in order.
    pub stmts: Vec<ComputeStmt>,
}

/// A statement inside a COMPUTE block.
#[derive(Debug, Clone, PartialEq)]
pub enum ComputeStmt {
    /// `<col> = <expr>;`
    Assign { col: String, expr: Expr },
    /// `line <item> [item ...];` — free-text line; items are literals or refs.
    Line(Vec<LineItem>),
}

/// An item in a `line` statement: a string literal or a bare expression
/// (typically a column reference resolved per group). An expression may carry
/// an optional trailing SAS format token (`line @5 total best8.;`, M33.5);
/// `None` keeps the default BESTw. rendering.
#[derive(Debug, Clone, PartialEq)]
pub enum LineItem {
    Literal(String),
    Expr(Expr, Option<String>),
    /// `@<col>` column pointer: pad the rendered line to (1-based) column.
    Pointer(usize),
}

/// Execute PROC REPORT. Called by `procs::execute_proc`.
pub fn execute(ast: &ReportAst, session: &mut Session) -> Result<()> {
    let (ds, display_name) = common::open_input_display(&ast.data, session)?;
    let n_obs_total = ds.n_obs();

    // --- Build the per-column plan, applying DEFINEs and type defaults. ---
    let plan = build_col_plan(ast, &ds)?;

    // --- Decode, apply WHERE, and project onto surviving rows. ---
    let (decoded, n_obs) = decode_and_filter(ast, &ds, &plan, n_obs_total)?;

    // --- ACROSS branch: distinct values of the across var become columns. ---
    let has_across = plan.iter().any(|c| matches!(c.usage, Usage::Across));
    if has_across {
        return execute_across(ast, session, &ds, &plan, &decoded, n_obs, &display_name);
    }

    // Determine whether this is a summary report.
    let group_positions: Vec<usize> = plan
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c.usage, Usage::Group | Usage::Order))
        .map(|(i, _)| i)
        .collect();
    let is_summary = !group_positions.is_empty();

    // --- Headers & alignments ---
    let (headers, aligns) = build_headers(&plan, &ds);

    // Output value rows (typed) — used both for the listing and for OUT=.
    // Each entry is (kind, values), where `kind` distinguishes detail/group
    // rows from BREAK/RBREAK summary rows (RBREAK is not written to OUT=).
    let mut value_rows: Vec<RowOut> = if !is_summary {
        build_detail_rows(&plan, &decoded, n_obs)
    } else {
        build_summary_rows(ast, &ds, &plan, &decoded, &group_positions, n_obs)
    };

    // --- COMPUTE: apply simple `<col> = <expr>;` assignments per row. ---
    apply_row_computes(ast, &plan, &mut value_rows);

    // --- Render the listing. ---
    // Clone the user-format catalog once so cell formatting (which borrows it)
    // does not clash with the mutable `session.listing` borrow below. Empty on
    // the default path → no behaviour change.
    let catalog = session.format_catalog.clone();
    let rows: Vec<Vec<String>> = value_rows
        .iter()
        .map(|ro| {
            ro.vals
                .iter()
                .enumerate()
                .map(|(ci, v)| fmt_cell_fmt(v, plan[ci].format.as_deref(), &catalog))
                .collect()
        })
        .collect();

    // Whether any DEFINE carried WIDTH=/SPACING= (M33.5). When none do, we keep
    // the exact historical rendering path (byte-identical default).
    let has_layout = plan
        .iter()
        .any(|c| c.width.is_some() || c.spacing.is_some());

    session.listing.page_header();
    if has_layout {
        write_table_layout(session, &headers, &aligns, &rows, &plan, ast.noheader);
    } else if ast.noheader {
        write_table_noheader(session, &aligns, &rows);
    } else {
        session.listing.write_table(&headers, &aligns, &rows);
    }

    // --- COMPUTE AFTER / LINE: free-text lines below the report. ---
    render_after_lines(ast, session, &plan, &value_rows, &catalog);

    // --- OUT=: write the report rows (excluding RBREAK grand total) as data. ---
    if let Some(out_ref) = &ast.out {
        write_out_dataset(session, out_ref, &plan, &ds, &value_rows)?;
    }

    // NOTE — observations read (plural invariable, as in PRINT). After a WHERE,
    // SAS reports the count actually read (the filtered count).
    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        n_obs, display_name
    ));

    Ok(())
}

#[cfg(test)]
mod tests;
