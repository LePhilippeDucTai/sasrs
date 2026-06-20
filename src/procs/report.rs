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
//!                                  → "Unexpected option 'XXX' on PROC REPORT
//!                                     statement."

use crate::ast::{DatasetRef, Expr};
use crate::error::{Result, SasError};
use crate::listing::Align;
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column, group_by_keys, partition_numeric};
use crate::procs::means;
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::{format_best, Value, VarType};
use polars::prelude::{Column, DataFrame, NamedFrom, Series};
use std::cmp::Ordering;

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
    let mut where_: Option<Expr> = None;
    let mut out: Option<DatasetRef> = None;
    let mut breaks: Vec<Break> = Vec::new();
    let mut rbreak: Option<Break> = None;
    let mut computes: Vec<Compute> = Vec::new();

    // --- PROC REPORT statement options, until `;` (combinateur partagé M31) ---
    common::parse_proc_options(ts, "REPORT", |ts, kw| {
        Ok(match kw {
            "data" => {
                data = Some(common::parse_dataset_opt(ts, "DATA")?);
                true
            }
            "out" => {
                out = Some(common::parse_out_opt(ts)?);
                true
            }
            "nowd" | "nowindow" => {
                // No-op: we never open an interactive window.
                ts.next();
                true
            }
            "noheader" => {
                ts.next();
                noheader = true;
                true
            }
            "headline" | "headskip" => {
                // No-op cosmetic options (rule line / skip line under headers).
                ts.next();
                true
            }
            _ => false,
        })
    })?;

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
            ts.next();
            computes.push(parse_compute(ts)?);
        } else if ts.peek().is_kw("break") {
            ts.next();
            breaks.push(parse_break(ts, false)?);
        } else if ts.peek().is_kw("rbreak") {
            ts.next();
            rbreak = Some(parse_break(ts, true)?);
        } else if ts.peek().is_kw("where") {
            ts.next();
            where_ = Some(crate::parser::expr::parse_expr(ts)?);
            ts.expect_semi()?;
        } else if is_global_stmt_kw(ts.peek().ident()) {
            // TITLE/FOOTNOTE (and numbered variants) are global statements that
            // SAS accepts anywhere, including inside a PROC step. We don't act on
            // them here (global title/footnote state is owned by the executor and
            // is set by the global statements placed before the step) — we just
            // skip them gracefully rather than aborting the whole REPORT, matching
            // the leniency of PROC PRINT and others.
            ts.skip_to_semi();
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
        where_,
        out,
        breaks,
        rbreak,
        computes,
    })
}

/// True if `ident` names a global statement (TITLE/FOOTNOTE, plain or numbered
/// e.g. TITLE2/FOOTNOTE3) that SAS allows inside a PROC step. Used to skip such
/// statements gracefully in the REPORT sub-statement loop.
fn is_global_stmt_kw(ident: Option<&str>) -> bool {
    let Some(w) = ident else { return false };
    let lw = w.to_ascii_lowercase();
    let stem = lw.trim_end_matches(|c: char| c.is_ascii_digit());
    matches!(stem, "title" | "footnote")
}

/// Parse a `break` / `rbreak` statement, after the keyword was consumed.
/// `break after <var> [/ summarize ...];`  |  `rbreak after [/ summarize ...];`
fn parse_break(ts: &mut StatementStream, is_rbreak: bool) -> Result<Break> {
    // Optional `after` / `before` location keyword. v1 treats both as the
    // summary line placed AFTER the range (the meaningful case); `before` is
    // accepted and documented as placed-after.
    if ts.peek().is_kw("after") || ts.peek().is_kw("before") {
        ts.next();
    }

    // For BREAK, a group variable name follows (absent for RBREAK).
    let var = if !is_rbreak {
        match ts.peek().ident().map(str::to_string) {
            Some(v) if ts.peek().kind != TokenKind::Slash => {
                ts.next();
                Some(v)
            }
            _ => {
                return Err(SasError::parse(
                    "expected a variable name after BREAK",
                    ts.peek().span,
                ));
            }
        }
    } else {
        None
    };

    // Optional `/ <options>`. v1 understands SUMMARIZE; OL/DOL/SKIP/PAGE and
    // similar cosmetic options are accepted and ignored (documented).
    let mut summarize = false;
    if ts.peek().kind == TokenKind::Slash {
        ts.next();
        loop {
            match &ts.peek().kind {
                TokenKind::Semi | TokenKind::Eof => break,
                TokenKind::Ident(raw) => {
                    if raw.eq_ignore_ascii_case("summarize") {
                        summarize = true;
                    }
                    // Other options (ol, dol, skip, page, suppress, ...) are
                    // cosmetic presentation flags → accepted, no-op in v1.
                    ts.next();
                }
                _ => {
                    ts.next();
                }
            }
        }
    }
    ts.expect_semi()?;
    Ok(Break { var, summarize })
}

/// Parse a `compute <target>; ... endcomp;` block, after `compute` consumed.
fn parse_compute(ts: &mut StatementStream) -> Result<Compute> {
    // Target: a column name or `after`/`before`.
    let target = match ts.peek().ident().map(str::to_string) {
        Some(t) => {
            ts.next();
            t
        }
        None => {
            return Err(SasError::parse(
                "expected a target after COMPUTE",
                ts.peek().span,
            ));
        }
    };
    // Skip any trailing options on the compute statement (e.g. `/ character`)
    // up to the `;`.
    while !matches!(ts.peek().kind, TokenKind::Semi | TokenKind::Eof) {
        ts.next();
    }
    ts.expect_semi()?;

    let mut stmts: Vec<ComputeStmt> = Vec::new();
    loop {
        while ts.peek().kind == TokenKind::Semi {
            ts.next();
        }
        if ts.peek().kind == TokenKind::Eof {
            return Err(SasError::parse(
                "expected ENDCOMP to close COMPUTE block",
                ts.peek().span,
            ));
        }
        if ts.peek().is_kw("endcomp") {
            ts.next();
            if ts.peek().kind == TokenKind::Semi {
                ts.next();
            }
            break;
        }
        if ts.peek().is_kw("line") {
            ts.next();
            stmts.push(ComputeStmt::Line(parse_line_items(ts)?));
            ts.expect_semi()?;
        } else if let Some(col) = ts.peek().ident().map(str::to_string) {
            // Expect `<col> = <expr>;`. Anything else inside a COMPUTE is
            // deferred CLEANLY (no panic): error with a clear message.
            ts.next();
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::runtime(format!(
                    "PROC REPORT v1 supports only simple '<col> = <expr>;' \
                     assignments and LINE statements inside COMPUTE (got '{}').",
                    col.to_uppercase()
                )));
            }
            ts.next(); // '='
            let expr = crate::parser::expr::parse_expr(ts)?;
            ts.expect_semi()?;
            stmts.push(ComputeStmt::Assign { col, expr });
        } else {
            return Err(SasError::parse(
                "unexpected token inside COMPUTE block",
                ts.peek().span,
            ));
        }
    }
    Ok(Compute { target, stmts })
}

/// Parse the items of a `line` statement up to (but not consuming) the `;`.
/// Supports string literals, `@<col>` pointers (rendered as padding to that
/// column), bare expressions (column references / numbers), and an optional
/// trailing SAS format on an expression (`line @5 total best8.;`, M33.5).
fn parse_line_items(ts: &mut StatementStream) -> Result<Vec<LineItem>> {
    let mut items = Vec::new();
    loop {
        match &ts.peek().kind {
            TokenKind::Semi | TokenKind::Eof => break,
            TokenKind::Str { value, .. } => {
                items.push(LineItem::Literal(value.clone()));
                ts.next();
            }
            TokenKind::At => {
                // `@<col>` column pointer: pad the line out to column `col`.
                ts.next();
                if let TokenKind::Num(n) = ts.peek().kind {
                    ts.next();
                    items.push(LineItem::Pointer(n.max(1.0) as usize));
                }
                // A bare `@` without a column is ignored (lenient).
            }
            _ => {
                // Parse a bare expression (column reference, number, ...).
                let e = crate::parser::expr::parse_expr(ts)?;
                // Optional trailing SAS format token (e.g. `best8.`): a format
                // is recognized only when the next token starts a format whose
                // text contains a '.' (so plain identifiers stay expressions).
                let fmt = if peek_is_line_format(ts) {
                    Some(crate::parser::expr::read_format_token(ts)?)
                } else {
                    None
                };
                items.push(LineItem::Expr(e, fmt));
            }
        }
    }
    Ok(items)
}

/// True when the next token begins a SAS format used as a LINE item suffix.
/// We only accept tokens whose joined format text contains a '.', so bare
/// identifiers (another expression item) are not mistaken for a format.
fn peek_is_line_format(ts: &StatementStream) -> bool {
    // A format suffix begins with an identifier (e.g. `best8.`, `dollar8.2`)
    // or `$`; a leading bare number like `8.2` is also a format. We confirm by
    // requiring the following token to be a Dot or a Num adjacent to it (the
    // shape of `best8.` / `8.2`). Two-token lookahead suffices.
    match &ts.peek().kind {
        TokenKind::Ident(_) => {
            // e.g. `best8.` → ident "best8" then Dot, or ident "best" then num.
            matches!(ts.peek2().kind, TokenKind::Dot)
                || matches!(ts.peek2().kind, TokenKind::Num(_))
        }
        TokenKind::Dollar => true,
        _ => false,
    }
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
    let mut format: Option<String> = None;
    let mut width: Option<usize> = None;
    let mut spacing: Option<usize> = None;

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
                            usage = Some(Usage::Across);
                            ts.next();
                        }
                        "computed" => {
                            usage = Some(Usage::Computed);
                            ts.next();
                        }
                        // `format=<fmt>` — SAS format / `w.d` for displayed
                        // values (M33.5). Read the raw format token verbatim.
                        "format" => {
                            ts.next();
                            if ts.peek().kind != TokenKind::Eq {
                                return Err(SasError::parse(
                                    "expected '=' after FORMAT in DEFINE statement",
                                    ts.peek().span,
                                ));
                            }
                            ts.next(); // '='
                            format = Some(crate::parser::expr::read_format_token(ts)?);
                        }
                        // `width=<n>` — column display width (M33.5).
                        "width" => {
                            ts.next();
                            if ts.peek().kind != TokenKind::Eq {
                                return Err(SasError::parse(
                                    "expected '=' after WIDTH in DEFINE statement",
                                    ts.peek().span,
                                ));
                            }
                            ts.next(); // '='
                            width = Some(parse_usize_opt(ts, "WIDTH")?);
                        }
                        // `spacing=<n>` — blank spaces before the column (M33.5).
                        "spacing" => {
                            ts.next();
                            if ts.peek().kind != TokenKind::Eq {
                                return Err(SasError::parse(
                                    "expected '=' after SPACING in DEFINE statement",
                                    ts.peek().span,
                                ));
                            }
                            ts.next(); // '='
                            spacing = Some(parse_usize_opt(ts, "SPACING")?);
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
        format,
        width,
        spacing,
    })
}

/// Parse a non-negative integer option value (e.g. `width=8`, `spacing=4`).
fn parse_usize_opt(ts: &mut StatementStream, opt: &str) -> Result<usize> {
    match ts.peek().kind {
        TokenKind::Num(n) if n >= 0.0 && n.fract() == 0.0 => {
            ts.next();
            Ok(n as usize)
        }
        _ => Err(SasError::parse(
            format!("expected a non-negative integer after {opt}= in DEFINE statement"),
            ts.peek().span,
        )),
    }
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

/// Resolved per-column plan entry: index into the dataset, effective usage,
/// order direction, and the header text to display.
struct ColPlan {
    idx: usize,
    usage: Usage,
    dir: OrderDir,
    header: String,
    /// `format=<fmt>` for displayed values (M33.5); `None` → default rendering.
    format: Option<String>,
    /// `width=<n>` display width (M33.5); `None` → aligner-derived width.
    width: Option<usize>,
    /// `spacing=<n>` blank spaces before this column (M33.5); `None` → default.
    spacing: Option<usize>,
}

/// Render a Value into a listing cell (numeric via format_best, missing → ".").
fn fmt_cell(v: &Value) -> String {
    match v {
        Value::Num(f) => format_best(*f, 12),
        Value::Missing(k) => k.display(),
        Value::Char(s) => s.clone(),
    }
}

/// Render a Value into a listing cell, honoring an optional `format=<fmt>`
/// DEFINE option (M33.5). With no format, this is byte-identical to `fmt_cell`.
/// With a format, the value routes through the SAS format engine and the
/// leading pad is trimmed so the listing aligner controls width (mirrors
/// TABULATE's M33.4 cell formatting).
fn fmt_cell_fmt(
    v: &Value,
    format: Option<&str>,
    catalog: &crate::formats::FormatCatalog,
) -> String {
    if let Some(spec) = format.and_then(crate::formats::FormatSpec::parse) {
        return catalog.format(v, &spec).trim_start().to_string();
    }
    fmt_cell(v)
}

/// Execute PROC REPORT. Called by `procs::execute_proc`.
pub fn execute(ast: &ReportAst, session: &mut Session) -> Result<()> {
    let in_ref = common::resolve_last_dataset(&ast.data, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();
    let display_name = in_ref.display();

    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }
    let n_obs_total = ds.n_obs();

    // --- Resolve the column list (display order). ---
    let col_names: Vec<String> = match &ast.columns {
        Some(list) => list.clone(),
        None => ds.vars.iter().map(|m| m.name.clone()).collect(),
    };

    // --- Build the per-column plan, applying DEFINEs and type defaults. ---
    let mut plan: Vec<ColPlan> = Vec::with_capacity(col_names.len());
    for cname in &col_names {
        let def = ast
            .defines
            .iter()
            .find(|d| d.var.eq_ignore_ascii_case(cname));

        // COMPUTED columns have no underlying dataset variable.
        if matches!(def.map(|d| &d.usage), Some(Usage::Computed)) {
            plan.push(ColPlan {
                idx: usize::MAX,
                usage: Usage::Computed,
                dir: def.map(|d| d.order).unwrap_or(OrderDir::Ascending),
                header: def
                    .and_then(|d| d.label.clone())
                    .unwrap_or_else(|| cname.clone()),
                format: def.and_then(|d| d.format.clone()),
                width: def.and_then(|d| d.width),
                spacing: def.and_then(|d| d.spacing),
            });
            continue;
        }

        let idx = ds
            .vars
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(cname))
            .ok_or_else(|| {
                SasError::runtime(format!("Variable {} not found.", cname.to_uppercase()))
            })?;

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
            format: def.and_then(|d| d.format.clone()),
            width: def.and_then(|d| d.width),
            spacing: def.and_then(|d| d.spacing),
        });
    }

    // Decode every planned column once (COMPUTED columns decode to all-missing).
    let decoded_all: Vec<Vec<Value>> = plan
        .iter()
        .map(|c| {
            if c.idx == usize::MAX {
                Ok(vec![Value::missing(); n_obs_total])
            } else {
                decode_column(&ds, c.idx)
            }
        })
        .collect::<Result<_>>()?;

    // --- WHERE: build the surviving-rows index. ---
    let live_rows: Vec<usize> = if let Some(cond) = &ast.where_ {
        // Build a name→decoded-column lookup over ALL dataset variables (not
        // just the planned columns) so the predicate can reference any var.
        let where_cols = decode_named_columns(&ds)?;
        (0..n_obs_total)
            .filter(|&r| {
                let v = eval_row_expr(cond, &where_cols, r);
                v.truthy()
            })
            .collect()
    } else {
        (0..n_obs_total).collect()
    };
    let n_obs = live_rows.len();

    // Project the decoded columns onto the surviving rows so downstream code
    // indexes 0..n_obs contiguously.
    let decoded: Vec<Vec<Value>> = decoded_all
        .iter()
        .map(|col| live_rows.iter().map(|&r| col[r].clone()).collect())
        .collect();

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
    let headers: Vec<String> = plan.iter().map(|c| c.header.clone()).collect();
    let aligns: Vec<Align> = plan
        .iter()
        .map(|c| match c.usage {
            Usage::Analysis(_) => Align::Right,
            Usage::Computed => Align::Right,
            _ if c.idx == usize::MAX => Align::Left,
            _ => match ds.vars[c.idx].ty {
                VarType::Num => Align::Right,
                VarType::Char => Align::Left,
            },
        })
        .collect();

    // Output value rows (typed) — used both for the listing and for OUT=.
    // Each entry is (kind, values), where `kind` distinguishes detail/group
    // rows from BREAK/RBREAK summary rows (RBREAK is not written to OUT=).
    let mut value_rows: Vec<RowOut> = Vec::new();

    if !is_summary {
        // ── Detail report: one listing row per (surviving) observation. ──
        for r in 0..n_obs {
            let vals: Vec<Value> = (0..plan.len()).map(|ci| decoded[ci][r].clone()).collect();
            value_rows.push(RowOut {
                kind: RowKind::Detail,
                vals,
            });
        }
    } else {
        // ── Summary report: group by GROUP+ORDER key columns. ──
        let key_refs: Vec<&Vec<Value>> = group_positions.iter().map(|&p| &decoded[p]).collect();
        let mut groups = group_by_keys(&key_refs, n_obs);

        // Apply DESCENDING direction lexicographically over the key tuple.
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

        // Which group var(s) trigger a BREAK? Map a break's var to its position
        // in `group_positions` (the deepest matching group level).
        let break_after: Vec<(usize, &Break)> = ast
            .breaks
            .iter()
            .filter_map(|b| {
                let vn = b.var.as_ref()?;
                group_positions
                    .iter()
                    .position(|&p| {
                        plan[p].idx != usize::MAX
                            && ds.vars[plan[p].idx].name.eq_ignore_ascii_case(vn)
                    })
                    .map(|pos| (pos, b))
            })
            .collect();

        for (gi, (key, grp_rows)) in groups.iter().enumerate() {
            let vals = summary_row_values(&plan, &decoded, grp_rows);
            value_rows.push(RowOut {
                kind: RowKind::Group,
                vals,
            });

            // BREAK AFTER <var>: emit a sub-total line when the key value for
            // that level changes (or at the last group).
            for &(level_pos, brk) in &break_after {
                let is_last = gi + 1 == groups.len();
                let changes = is_last
                    || groups[gi + 1].0.get(level_pos).map(|nv| {
                        key[level_pos].sas_cmp(nv) != Ordering::Equal
                    }) != Some(false);
                if changes && brk.summarize {
                    // Range = all original rows whose key matches up to and
                    // including `level_pos`. Collect across the contiguous run.
                    let range = break_range_rows(&groups, gi, level_pos, key);
                    let bvals = break_row_values(&plan, &decoded, &range, level_pos);
                    value_rows.push(RowOut {
                        kind: RowKind::Break,
                        vals: bvals,
                    });
                }
            }
        }

        // RBREAK AFTER / SUMMARIZE: grand-total line over all surviving rows.
        if let Some(rb) = &ast.rbreak {
            if rb.summarize {
                let all: Vec<usize> = (0..n_obs).collect();
                let rvals = break_row_values(&plan, &decoded, &all, usize::MAX);
                value_rows.push(RowOut {
                    kind: RowKind::Rbreak,
                    vals: rvals,
                });
            }
        }
    }

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
    let has_layout = plan.iter().any(|c| c.width.is_some() || c.spacing.is_some());

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

/// A produced report row (typed values) and what kind of row it is.
struct RowOut {
    kind: RowKind,
    vals: Vec<Value>,
}

#[derive(Clone, Copy, PartialEq)]
enum RowKind {
    Detail,
    Group,
    Break,
    Rbreak,
}

/// Compute the typed cell values of a summary (group) row.
fn summary_row_values(plan: &[ColPlan], decoded: &[Vec<Value>], grp_rows: &[usize]) -> Vec<Value> {
    let mut vals = Vec::with_capacity(plan.len());
    for (ci, c) in plan.iter().enumerate() {
        let v = match &c.usage {
            Usage::Group | Usage::Order => decoded[ci][grp_rows[0]].clone(),
            Usage::Analysis(stat) => {
                let (xs, nmiss) = partition_numeric(&decoded[ci], grp_rows);
                means::compute(stat, &xs, nmiss, 0.05)
            }
            Usage::Display => {
                let first = &decoded[ci][grp_rows[0]];
                let constant = grp_rows
                    .iter()
                    .all(|&r| decoded[ci][r].sas_cmp(first) == Ordering::Equal);
                if constant {
                    first.clone()
                } else {
                    Value::Char(String::new())
                }
            }
            // COMPUTED / ACROSS columns are filled later / handled elsewhere.
            _ => Value::missing(),
        };
        vals.push(v);
    }
    vals
}

/// Compute the typed cell values of a BREAK/RBREAK summary row. The break key
/// columns up to and including `level_pos` keep their value; deeper group
/// columns are blanked; ANALYSIS columns are recomputed over `range`.
fn break_row_values(
    plan: &[ColPlan],
    decoded: &[Vec<Value>],
    range: &[usize],
    level_pos_excl: usize,
) -> Vec<Value> {
    // Translate the group-level cutoff (an index into group_positions) into a
    // plan-column comparison: we keep GROUP/ORDER cells whose own group level
    // is <= level_pos_excl; here we simply keep the first matching value for
    // key columns and blank the rest, marking the first key column with a tag.
    let mut group_seen = 0usize;
    let mut vals = Vec::with_capacity(plan.len());
    let mut first_key_done = false;
    for (ci, c) in plan.iter().enumerate() {
        let v = match &c.usage {
            Usage::Group | Usage::Order => {
                let keep = group_seen <= level_pos_excl;
                group_seen += 1;
                if keep && !range.is_empty() {
                    if !first_key_done && level_pos_excl == usize::MAX {
                        // RBREAK: label the leading key column.
                        first_key_done = true;
                        Value::Char(String::new())
                    } else {
                        decoded[ci][range[0]].clone()
                    }
                } else {
                    Value::Char(String::new())
                }
            }
            Usage::Analysis(stat) => {
                let (xs, nmiss) = partition_numeric(&decoded[ci], range);
                means::compute(stat, &xs, nmiss, 0.05)
            }
            _ => Value::Char(String::new()),
        };
        vals.push(v);
    }
    vals
}

/// Collect the original (projected) row indices belonging to the contiguous run
/// of groups that share the same key prefix up to `level_pos` ending at `gi`.
fn break_range_rows(
    groups: &[(Vec<Value>, Vec<usize>)],
    gi: usize,
    level_pos: usize,
    key: &[Value],
) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    // Walk backwards while the prefix matches, then forward — but since groups
    // are sorted, the run sharing this prefix is contiguous and ends at gi.
    let prefix_eq = |k: &[Value]| -> bool {
        (0..=level_pos).all(|p| key[p].sas_cmp(&k[p]) == Ordering::Equal)
    };
    let mut start = gi;
    while start > 0 && prefix_eq(&groups[start - 1].0) {
        start -= 1;
    }
    for g in &groups[start..=gi] {
        out.extend_from_slice(&g.1);
    }
    out
}

// ───────────────────────── WHERE evaluation ─────────────────────────

/// Decode every dataset variable into a name→Values map (lowercased names) so a
/// WHERE predicate can reference any variable. Returns a Vec of (name, column)
/// to preserve simple lookup without pulling in a HashMap.
fn decode_named_columns(ds: &crate::dataset::SasDataset) -> Result<Vec<(String, Vec<Value>)>> {
    let mut out = Vec::with_capacity(ds.vars.len());
    for (i, m) in ds.vars.iter().enumerate() {
        out.push((m.name.to_ascii_lowercase(), decode_column(ds, i)?));
    }
    Ok(out)
}

/// Look up a column's value for row `r` by (case-insensitive) name.
fn lookup_var<'a>(cols: &'a [(String, Vec<Value>)], name: &str, r: usize) -> Option<&'a Value> {
    let lname = name.to_ascii_lowercase();
    cols.iter()
        .find(|(n, _)| *n == lname)
        .map(|(_, col)| &col[r])
}

/// Self-contained, faithful-SAS evaluation of a WHERE/COMPUTE expression for a
/// single row, over decoded columns. Comparisons go through `Value::sas_cmp`
/// (so `. = .` is true and char compares ignore trailing blanks); logical ops
/// use SAS truthiness (missing/0 = false). Unsupported constructs (function
/// calls, arrays, hash methods) evaluate to a guard missing rather than panic.
fn eval_row_expr(expr: &Expr, cols: &[(String, Vec<Value>)], r: usize) -> Value {
    use crate::ast::UnaryOp;
    match expr {
        Expr::Num(n) => Value::Num(*n),
        Expr::Str(s) => Value::Char(s.clone()),
        Expr::Missing(k) => Value::Missing(*k),
        Expr::Var(name) => lookup_var(cols, name, r).cloned().unwrap_or(Value::missing()),
        Expr::Unary { op, expr } => {
            let v = eval_row_expr(expr, cols, r);
            match op {
                UnaryOp::Not => Value::Num(if v.truthy() { 0.0 } else { 1.0 }),
                UnaryOp::Plus => match value_to_num(&v) {
                    Some(f) => Value::Num(f),
                    None => Value::missing(),
                },
                UnaryOp::Minus => match value_to_num(&v) {
                    Some(f) => Value::Num(-f),
                    None => Value::missing(),
                },
            }
        }
        Expr::Binary { op, left, right } => {
            let l = eval_row_expr(left, cols, r);
            let rr = eval_row_expr(right, cols, r);
            eval_row_binary(*op, &l, &rr)
        }
        Expr::In { expr, list } => {
            let v = eval_row_expr(expr, cols, r);
            let found = list.iter().any(|e| {
                let item = eval_row_expr(e, cols, r);
                v.sas_cmp(&item) == Ordering::Equal
            });
            Value::Num(if found { 1.0 } else { 0.0 })
        }
        // Unsupported in this lightweight evaluator (documented): guard missing.
        _ => Value::missing(),
    }
}

fn eval_row_binary(op: crate::ast::BinaryOp, l: &Value, r: &Value) -> Value {
    use crate::ast::BinaryOp::*;
    match op {
        Lt | Le | Gt | Ge | Eq | Ne => {
            let ord = l.sas_cmp(r);
            let res = match op {
                Eq => ord == Ordering::Equal,
                Ne => ord != Ordering::Equal,
                Lt => ord == Ordering::Less,
                Le => ord != Ordering::Greater,
                Gt => ord == Ordering::Greater,
                Ge => ord != Ordering::Less,
                _ => unreachable!(),
            };
            Value::Num(if res { 1.0 } else { 0.0 })
        }
        And => Value::Num(if l.truthy() && r.truthy() { 1.0 } else { 0.0 }),
        Or => Value::Num(if l.truthy() || r.truthy() { 1.0 } else { 0.0 }),
        Concat => {
            let ls = value_to_disp(l);
            let rs = value_to_disp(r);
            Value::Char(format!("{ls}{rs}"))
        }
        Add | Sub | Mul | Div | Power => {
            match (value_to_num(l), value_to_num(r)) {
                (Some(a), Some(b)) => {
                    let v = match op {
                        Add => a + b,
                        Sub => a - b,
                        Mul => a * b,
                        Div => {
                            if b == 0.0 {
                                return Value::missing();
                            }
                            a / b
                        }
                        Power => a.powf(b),
                        _ => unreachable!(),
                    };
                    Value::Num(v)
                }
                _ => Value::missing(),
            }
        }
    }
}

/// Plain string rendering of a Value for concatenation / LINE output.
fn value_to_disp(v: &Value) -> String {
    match v {
        Value::Char(s) => s.trim_end().to_string(),
        Value::Num(f) => format_best(*f, 12),
        Value::Missing(_) => String::new(),
    }
}

// ───────────────────────── ACROSS report ─────────────────────────

/// Render an ACROSS report: GROUP/ORDER vars in rows, the distinct values of
/// the ACROSS var in columns, each cell = the statistic of the ANALYSIS var.
/// v1 supports exactly one ACROSS var and one ANALYSIS var; the two-level
/// header (across value over statistic) is flattened into a single header line
/// "value stat" (documented simplification, since the listing has no spanner).
fn execute_across(
    ast: &ReportAst,
    session: &mut Session,
    ds: &crate::dataset::SasDataset,
    plan: &[ColPlan],
    decoded: &[Vec<Value>],
    n_obs: usize,
    display_name: &str,
) -> Result<()> {
    // Identify the across, group/order, and analysis columns.
    let across_pos = plan
        .iter()
        .position(|c| matches!(c.usage, Usage::Across))
        .expect("execute_across called without an ACROSS column");
    let group_positions: Vec<usize> = plan
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c.usage, Usage::Group | Usage::Order))
        .map(|(i, _)| i)
        .collect();
    let analysis_pos = plan.iter().position(|c| matches!(c.usage, Usage::Analysis(_)));
    let (apos, stat) = match analysis_pos {
        Some(p) => match &plan[p].usage {
            Usage::Analysis(s) => (p, s.clone()),
            _ => unreachable!(),
        },
        None => {
            return Err(SasError::runtime(
                "PROC REPORT ACROSS in v1 requires exactly one ANALYSIS variable.",
            ));
        }
    };

    // Distinct across values (sorted via sas_cmp, honoring direction).
    let across_dir = plan[across_pos].dir;
    let mut across_vals: Vec<Value> = Vec::new();
    for r in 0..n_obs {
        let v = &decoded[across_pos][r];
        if !across_vals.iter().any(|e| e.sas_cmp(v) == Ordering::Equal) {
            across_vals.push(v.clone());
        }
    }
    across_vals.sort_by(|a, b| {
        let c = a.sas_cmp(b);
        if across_dir == OrderDir::Descending {
            c.reverse()
        } else {
            c
        }
    });

    // Group the rows by the GROUP/ORDER key tuple.
    let key_refs: Vec<&Vec<Value>> = group_positions.iter().map(|&p| &decoded[p]).collect();
    let mut groups = group_by_keys(&key_refs, n_obs);
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

    // Headers: the GROUP/ORDER columns, then one column per across value.
    let mut headers: Vec<String> = group_positions.iter().map(|&p| plan[p].header.clone()).collect();
    let stat_label = stat.to_uppercase();
    for av in &across_vals {
        headers.push(format!("{} {}", value_to_disp(av), stat_label));
    }

    let mut aligns: Vec<Align> = group_positions
        .iter()
        .map(|&p| match ds.vars[plan[p].idx].ty {
            VarType::Num => Align::Right,
            VarType::Char => Align::Left,
        })
        .collect();
    aligns.extend(std::iter::repeat(Align::Right).take(across_vals.len()));

    let mut rows: Vec<Vec<String>> = Vec::with_capacity(groups.len());
    for (_key, grp_rows) in &groups {
        let mut row: Vec<String> = Vec::new();
        for &gp in &group_positions {
            row.push(fmt_cell(&decoded[gp][grp_rows[0]]));
        }
        for av in &across_vals {
            // Sub-select the group rows whose across value equals `av`.
            let sub: Vec<usize> = grp_rows
                .iter()
                .copied()
                .filter(|&r| decoded[across_pos][r].sas_cmp(av) == Ordering::Equal)
                .collect();
            let (xs, nmiss) = partition_numeric(&decoded[apos], &sub);
            let v = means::compute(&stat, &xs, nmiss, 0.05);
            row.push(fmt_cell(&v));
        }
        rows.push(row);
    }

    session.listing.page_header();
    if ast.noheader {
        write_table_noheader(session, &aligns, &rows);
    } else {
        session.listing.write_table(&headers, &aligns, &rows);
    }

    session.log.note(&format!(
        "There were {} observations read from the data set {}.",
        n_obs, display_name
    ));
    // OUT= for ACROSS is deferred CLEANLY (no panic): note and skip.
    if ast.out.is_some() {
        session.log.note(
            "PROC REPORT v1 does not write an OUT= data set for ACROSS reports; OUT= ignored.",
        );
    }
    Ok(())
}

// ───────────────────────── COMPUTE / LINE ─────────────────────────

/// Apply simple `compute <col>; <col> = <expr>; endcomp;` assignments to each
/// produced row. The expression may reference any report column by name (its
/// per-row value). Computes targeting `after`/`before` are handled separately
/// (LINE rendering); non-column targets are skipped here.
fn apply_row_computes(ast: &ReportAst, plan: &[ColPlan], rows: &mut [RowOut]) {
    for comp in &ast.computes {
        // Only column-targeted computes assign into a cell.
        let target_ci = plan
            .iter()
            .position(|c| c.header.eq_ignore_ascii_case(&comp.target));
        // Build the per-row column context lazily inside the loop.
        for ro in rows.iter_mut() {
            // Context: each plan column referenced by its header AND by the
            // positional alias `_Cn_` (1-based COLUMN index, M33.5).
            let cols = compute_row_context(plan, &ro.vals);
            for st in &comp.stmts {
                if let ComputeStmt::Assign { col, expr } = st {
                    let v = eval_row_expr(expr, &cols, 0);
                    // Assign into the named column if it matches a plan column,
                    // else into the compute target column.
                    let dest = plan
                        .iter()
                        .position(|c| c.header.eq_ignore_ascii_case(col))
                        .or(target_ci);
                    if let Some(d) = dest {
                        ro.vals[d] = v;
                    }
                }
            }
        }
    }
}

/// Build the per-row COMPUTE/LINE evaluation context: each plan column is
/// addressable by its (lowercased) header AND by the positional alias `_Cn_`
/// (1-based COLUMN index), matching SAS's `_C1_`/`_C2_` report-column refs
/// (M33.5). Each column holds a single value (the current report row).
fn compute_row_context(plan: &[ColPlan], vals: &[Value]) -> Vec<(String, Vec<Value>)> {
    let mut cols: Vec<(String, Vec<Value>)> = Vec::with_capacity(plan.len() * 2);
    for (ci, c) in plan.iter().enumerate() {
        cols.push((c.header.to_ascii_lowercase(), vec![vals[ci].clone()]));
        cols.push((format!("_c{}_", ci + 1), vec![vals[ci].clone()]));
    }
    cols
}

/// Render `compute after; line ...; endcomp;` free-text lines below the report.
/// LINE items are concatenated: string literals verbatim, `@<col>` pointers pad
/// to a column, and expressions are resolved over the grand-total context (with
/// `_Cn_` aliases) and rendered with an optional trailing format (M33.5).
fn render_after_lines(
    ast: &ReportAst,
    session: &mut Session,
    plan: &[ColPlan],
    rows: &[RowOut],
    catalog: &crate::formats::FormatCatalog,
) {
    for comp in &ast.computes {
        if !comp.target.eq_ignore_ascii_case("after") {
            continue;
        }
        // Context for LINE expressions: the grand-total (RBREAK) row if present,
        // else the last row, else empty.
        let ctx_row = rows
            .iter()
            .rev()
            .find(|r| r.kind == RowKind::Rbreak)
            .or_else(|| rows.last());
        let ctx_cols: Option<Vec<(String, Vec<Value>)>> =
            ctx_row.map(|ro| compute_row_context(plan, &ro.vals));
        for st in &comp.stmts {
            if let ComputeStmt::Line(items) = st {
                let mut line = String::new();
                for item in items {
                    match item {
                        LineItem::Literal(s) => line.push_str(s),
                        LineItem::Pointer(col) => {
                            // Pad the line out to (1-based) column `col`.
                            if *col > line.len() {
                                line.push_str(&" ".repeat(*col - line.len()));
                            }
                        }
                        LineItem::Expr(e, fmt) => {
                            let v = match &ctx_cols {
                                Some(cols) => eval_row_expr(e, cols, 0),
                                None => Value::missing(),
                            };
                            match fmt.as_deref().and_then(crate::formats::FormatSpec::parse) {
                                Some(spec) => {
                                    line.push_str(catalog.format(&v, &spec).trim_start())
                                }
                                None => line.push_str(&value_to_disp(&v)),
                            }
                        }
                    }
                }
                session.listing.write_line(line.trim_end());
            }
        }
    }
}

// ───────────────────────── OUT= dataset ─────────────────────────

/// Render a Value as an optional char cell for OUT= (trailing blanks trimmed,
/// blanks/missing → null).
fn value_to_char_cell(v: &Value) -> Option<String> {
    match v {
        Value::Char(s) => {
            let t = s.trim_end();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Value::Num(f) => Some(format_best(*f, 12).trim().to_string()),
        Value::Missing(_) => None,
    }
}

/// Write the report's detail/group/break rows (excluding the RBREAK grand
/// total) as an OUT= data set. One output column per COLUMN entry; the SAS type
/// follows the input variable (ANALYSIS/GROUP/ORDER numeric vars stay numeric,
/// char DISPLAY vars stay char). COMPUTED columns are numeric.
fn write_out_dataset(
    session: &mut Session,
    out_ref: &DatasetRef,
    plan: &[ColPlan],
    ds: &crate::dataset::SasDataset,
    rows: &[RowOut],
) -> Result<()> {
    use crate::dataset::{SasDataset, VarMeta};

    // OUT= captures the report body rows (detail, group, and BREAK sub-totals);
    // the RBREAK grand total is a presentation-only line and is excluded.
    let body: Vec<&RowOut> = rows.iter().filter(|r| r.kind != RowKind::Rbreak).collect();

    let mut columns: Vec<Column> = Vec::with_capacity(plan.len());
    let mut vars: Vec<VarMeta> = Vec::with_capacity(plan.len());

    for (ci, c) in plan.iter().enumerate() {
        // Decide the output type for this column.
        let is_char = match &c.usage {
            Usage::Analysis(_) | Usage::Computed => false,
            _ => c.idx != usize::MAX && ds.vars[c.idx].ty == VarType::Char,
        };
        let name = if c.idx == usize::MAX {
            c.header.clone()
        } else {
            ds.vars[c.idx].name.clone()
        };
        if is_char {
            let vals: Vec<Option<String>> =
                body.iter().map(|r| value_to_char_cell(&r.vals[ci])).collect();
            let len = vals
                .iter()
                .flatten()
                .map(|s| s.len())
                .max()
                .unwrap_or(8)
                .max(1);
            columns.push(Series::new(name.as_str().into(), vals).into());
            vars.push(VarMeta {
                name,
                ty: VarType::Char,
                length: len,
                format: None,
                label: None,
            });
        } else {
            let vals: Vec<Option<f64>> =
                body.iter().map(|r| value_to_num(&r.vals[ci])).collect();
            columns.push(Series::new(name.as_str().into(), vals).into());
            vars.push(VarMeta {
                name,
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            });
        }
    }

    let df = DataFrame::new(columns)?;
    let out_ds = SasDataset { df, vars };

    let out_libref = out_ref.libref_or_work();
    let out_table = out_ref.name.to_uppercase();
    let display = format!("{out_libref}.{out_table}");
    let n_rows = out_ds.n_obs();
    let n_vars = out_ds.vars.len();

    session.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
    session.last_dataset = Some(display.clone());

    session.log.note(&format!(
        "The data set {} has {} observations and {} variables.",
        display, n_rows, n_vars
    ));
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

/// Render the report table honoring per-column `WIDTH=`/`SPACING=` DEFINE
/// options (M33.5). Used only when at least one column carries WIDTH=/SPACING=;
/// the default path stays on `ListingWriter::write_table`.
///
/// Semantics (faithful to SAS LISTING):
///   - A column's width is its `WIDTH=` if given, else the max of the header
///     and cell lengths (the auto width).
///   - Cells/header are truncated or padded to the width; numeric (Right-
///     aligned) columns right-justify, character (Left-aligned) columns
///     left-justify.
///   - `SPACING=<n>` sets the number of blank spaces BEFORE the column
///     (default 2). The leading column's spacing is rendered as left padding
///     too (SAS indents the first column by its spacing).
fn write_table_layout(
    session: &mut Session,
    headers: &[String],
    aligns: &[Align],
    rows: &[Vec<String>],
    plan: &[ColPlan],
    noheader: bool,
) {
    let ncol = headers.len();
    if ncol == 0 {
        return;
    }

    // Resolve each column's effective width.
    let mut widths = vec![0usize; ncol];
    for i in 0..ncol {
        match plan[i].width {
            Some(w) => widths[i] = w,
            None => {
                let mut w = headers[i].len();
                for row in rows {
                    if let Some(cell) = row.get(i) {
                        w = w.max(cell.len());
                    }
                }
                widths[i] = w;
            }
        }
    }

    // Spacing before each column (default 2; the leading column's spacing is
    // emitted as left indentation).
    let spacing: Vec<usize> = plan.iter().map(|c| c.spacing.unwrap_or(2)).collect();

    let pad_cell = |cell: &str, w: usize, align: Align| -> String {
        let mut s = cell.to_string();
        if s.len() > w {
            s.truncate(w);
        }
        let pad = w.saturating_sub(s.len());
        match align {
            Align::Right => format!("{}{}", " ".repeat(pad), s),
            Align::Left => format!("{}{}", s, " ".repeat(pad)),
        }
    };

    let render = |cells: &dyn Fn(usize) -> String| -> String {
        let mut line = String::new();
        for i in 0..ncol {
            line.push_str(&" ".repeat(spacing[i]));
            let align = aligns.get(i).copied().unwrap_or(Align::Left);
            line.push_str(&pad_cell(&cells(i), widths[i], align));
        }
        line
    };

    if !noheader {
        let header_line = render(&|i| headers[i].clone());
        session.listing.write_line(header_line.trim_end());
        session.listing.blank();
    }
    for row in rows {
        let line = render(&|i| row.get(i).cloned().unwrap_or_default());
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

    /// Defaults for the advanced (M21.4) ReportAst fields, used with Rust's
    /// struct-update syntax (`..report_defaults()`) in the execute tests.
    fn report_defaults() -> ReportAst {
        ReportAst {
            data: None,
            noheader: false,
            columns: None,
            defines: vec![],
            where_: None,
            out: None,
            breaks: vec![],
            rbreak: None,
            computes: vec![],
        }
    }

    fn work_ref(name: &str) -> DatasetRef {
        DatasetRef {
            libref: Some("WORK".into()),
            name: name.into(),
        }
    }

    /// Parse a standalone expression (e.g. a WHERE condition) for tests. The
    /// SourceFile is owned within this scope; the returned Expr is owned.
    fn parse_test_expr(src: &str) -> Expr {
        let source = crate::source::SourceFile::new(src);
        let mut ts = crate::parser::StatementStream::new(&source).unwrap();
        crate::parser::expr::parse_expr(&mut ts).unwrap()
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
    fn parse_across_usage_now_parses() {
        let ast = parse_report("proc report data=a; define x / across; run;").unwrap();
        assert_eq!(ast.defines[0].usage, Usage::Across);
    }

    #[test]
    fn parse_compute_block_now_parses() {
        let ast =
            parse_report("proc report data=a; compute after; line 'hi'; endcomp; run;").unwrap();
        assert_eq!(ast.computes.len(), 1);
        assert_eq!(ast.computes[0].target, "after");
        assert!(matches!(ast.computes[0].stmts[0], ComputeStmt::Line(_)));
    }

    #[test]
    fn parse_compute_assignment() {
        let ast =
            parse_report("proc report data=a; compute pct; pct = sales * 2; endcomp; run;").unwrap();
        match &ast.computes[0].stmts[0] {
            ComputeStmt::Assign { col, .. } => assert_eq!(col, "pct"),
            _ => panic!("expected assignment"),
        }
    }

    #[test]
    fn parse_break_now_parses() {
        let ast =
            parse_report("proc report data=a; break after region / summarize; run;").unwrap();
        assert_eq!(ast.breaks.len(), 1);
        assert_eq!(ast.breaks[0].var.as_deref(), Some("region"));
        assert!(ast.breaks[0].summarize);
    }

    #[test]
    fn parse_rbreak_now_parses() {
        let ast = parse_report("proc report data=a; rbreak after / summarize; run;").unwrap();
        assert!(ast.rbreak.is_some());
        assert!(ast.rbreak.as_ref().unwrap().var.is_none());
        assert!(ast.rbreak.as_ref().unwrap().summarize);
    }

    #[test]
    fn parse_where_statement() {
        let ast = parse_report("proc report data=a; where age > 12; run;").unwrap();
        assert!(ast.where_.is_some());
    }

    #[test]
    fn parse_out_option() {
        let ast = parse_report("proc report data=a out=work.b nowd; run;").unwrap();
        assert_eq!(ast.out.as_ref().unwrap().name, "b");
    }

    #[test]
    fn parse_computed_define() {
        let ast = parse_report("proc report data=a; define c / computed; run;").unwrap();
        assert_eq!(ast.defines[0].usage, Usage::Computed);
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
            ..report_defaults()
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
                    format: None,
                    width: None,
                    spacing: None,
                },
                Define {
                    var: "sales".into(),
                    usage: Usage::Analysis("sum".into()),
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
            ],
            ..report_defaults()
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
                    format: None,
                    width: None,
                    spacing: None,
                },
                Define {
                    var: "x".into(),
                    usage: Usage::Analysis("mean".into()),
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
            ],
            ..report_defaults()
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
                    format: None,
                    width: None,
                    spacing: None,
                },
                Define {
                    var: "v".into(),
                    usage: Usage::Analysis("sum".into()),
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
            ],
            ..report_defaults()
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
                format: None,
                width: None,
                spacing: None,
            }],
            ..report_defaults()
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
                format: None,
                width: None,
                spacing: None,
            }],
            ..report_defaults()
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
            ..report_defaults()
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
                    format: None,
                    width: None,
                    spacing: None,
                },
                Define {
                    var: "x".into(),
                    usage: Usage::Analysis("mean".into()),
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
            ],
            ..report_defaults()
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
            ..report_defaults()
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
            ..report_defaults()
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
            ..report_defaults()
        };
        execute(&ast, &mut session).unwrap();
        // REPORT must not change last_dataset.
        assert_eq!(session.last_dataset, before);
    }

    // ─────────────────── M21.4 advanced feature tests ───────────────────

    /// sashelp.class-like sex/age fixture: M/F with weights.
    fn class_like(session: &mut Session) {
        // 3 F (ages 11,12,13 → sum 36) and 2 M (ages 14,15 → sum 29).
        let df = df![
            "sex" => ["F", "F", "F", "M", "M"],
            "age" => [11.0_f64, 12.0, 13.0, 14.0, 15.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("sex"), num_meta("age")],
        };
        write_dataset(session, "C", ds);
    }

    #[test]
    fn where_filters_observations() {
        let mut session = make_session();
        class_like(&mut session);
        // where age > 12 → keep ages 13,14,15. Group by sex: F sum=13, M sum=29.
        let ast = ReportAst {
            data: Some(work_ref("C")),
            columns: Some(vec!["sex".into(), "age".into()]),
            defines: vec![
                Define {
                    var: "sex".into(),
                    usage: Usage::Group,
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
                Define {
                    var: "age".into(),
                    usage: Usage::Analysis("sum".into()),
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
            ],
            where_: Some(parse_test_expr("age > 12;")),
            ..report_defaults()
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // F filtered sum = 13, M = 29.
        assert!(listing.contains("13"), "F sum 13: {listing}");
        assert!(listing.contains("29"), "M sum 29: {listing}");
        // The filtered-out values 11/12 must not appear as an F total of 36.
        assert!(!listing.contains("36"), "36 should be filtered out: {listing}");
    }

    #[test]
    fn where_char_equality_sas_cmp() {
        let mut session = make_session();
        class_like(&mut session);
        // where sex = 'M' → only M rows; detail report shows 14 and 15, not 11.
        let ast = ReportAst {
            data: Some(work_ref("C")),
            columns: Some(vec!["sex".into(), "age".into()]),
            defines: vec![Define {
                var: "age".into(),
                usage: Usage::Display,
                order: OrderDir::Ascending,
                label: None,
                format: None,
                width: None,
                spacing: None,
            }],
            where_: Some(
                parse_test_expr("sex = 'M';"),
            ),
            ..report_defaults()
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("14"), "{listing}");
        assert!(listing.contains("15"), "{listing}");
        assert!(!listing.contains("11"), "11 filtered: {listing}");
    }

    #[test]
    fn out_dataset_written_and_typed() {
        let mut session = make_session();
        class_like(&mut session);
        let ast = ReportAst {
            data: Some(work_ref("C")),
            columns: Some(vec!["sex".into(), "age".into()]),
            defines: vec![
                Define {
                    var: "sex".into(),
                    usage: Usage::Group,
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
                Define {
                    var: "age".into(),
                    usage: Usage::Analysis("sum".into()),
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
            ],
            out: Some(work_ref("R")),
            ..report_defaults()
        };
        execute(&ast, &mut session).unwrap();
        // OUT= sets last_dataset and writes 2 rows (F, M).
        assert_eq!(session.last_dataset.as_deref(), Some("WORK.R"));
        let (out, _) = session.libs.get("WORK").unwrap().read("R").unwrap();
        assert_eq!(out.n_obs(), 2);
        // sex stays char, age stays numeric.
        assert_eq!(out.vars[0].name.to_lowercase(), "sex");
        assert_eq!(out.vars[0].ty, VarType::Char);
        assert_eq!(out.vars[1].ty, VarType::Num);
        let age = decode_column(&out, 1).unwrap();
        // F sum 36, M sum 29.
        assert_eq!(age[0], Value::Num(36.0));
        assert_eq!(age[1], Value::Num(29.0));
    }

    #[test]
    fn across_makes_columns_from_distinct_values() {
        let mut session = make_session();
        // region × sex crosstab of sales sum.
        let df = df![
            "region" => ["E", "E", "W", "W"],
            "sex" => ["F", "M", "F", "M"],
            "sales" => [10.0_f64, 20.0, 30.0, 40.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("region"), char_meta("sex"), num_meta("sales")],
        };
        write_dataset(&mut session, "X", ds);
        let ast = ReportAst {
            data: Some(work_ref("X")),
            columns: Some(vec!["region".into(), "sex".into(), "sales".into()]),
            defines: vec![
                Define {
                    var: "region".into(),
                    usage: Usage::Group,
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
                Define {
                    var: "sex".into(),
                    usage: Usage::Across,
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
                Define {
                    var: "sales".into(),
                    usage: Usage::Analysis("sum".into()),
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
            ],
            ..report_defaults()
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // Two across columns "F Sum" and "M Sum"; E row → F 10, M 20; W → 30,40.
        assert!(listing.contains("F SUM"), "across header F: {listing}");
        assert!(listing.contains("M SUM"), "across header M: {listing}");
        assert!(listing.contains("10"), "{listing}");
        assert!(listing.contains("20"), "{listing}");
        assert!(listing.contains("30"), "{listing}");
        assert!(listing.contains("40"), "{listing}");
    }

    #[test]
    fn break_after_group_summary_line() {
        let mut session = make_session();
        // Two-level group region/sub; break after region summarizes.
        let df = df![
            "region" => ["E", "E", "W"],
            "sub" => ["a", "b", "c"],
            "sales" => [10.0_f64, 30.0, 100.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("region"), char_meta("sub"), num_meta("sales")],
        };
        write_dataset(&mut session, "B", ds);
        let ast = ReportAst {
            data: Some(work_ref("B")),
            columns: Some(vec!["region".into(), "sub".into(), "sales".into()]),
            defines: vec![
                Define {
                    var: "region".into(),
                    usage: Usage::Group,
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
                Define {
                    var: "sub".into(),
                    usage: Usage::Group,
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
                Define {
                    var: "sales".into(),
                    usage: Usage::Analysis("sum".into()),
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
            ],
            breaks: vec![Break {
                var: Some("region".into()),
                summarize: true,
            }],
            ..report_defaults()
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // E subtotal = 10+30 = 40 appears after the E rows.
        assert!(listing.contains("40"), "E subtotal 40: {listing}");
        // W subtotal = 100.
        assert!(listing.contains("100"), "W subtotal 100: {listing}");
    }

    #[test]
    fn rbreak_grand_total_line() {
        let mut session = make_session();
        class_like(&mut session);
        let ast = ReportAst {
            data: Some(work_ref("C")),
            columns: Some(vec!["sex".into(), "age".into()]),
            defines: vec![
                Define {
                    var: "sex".into(),
                    usage: Usage::Group,
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
                Define {
                    var: "age".into(),
                    usage: Usage::Analysis("sum".into()),
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
            ],
            rbreak: Some(Break {
                var: None,
                summarize: true,
            }),
            ..report_defaults()
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // Grand total of ages = 11+12+13+14+15 = 65.
        assert!(listing.contains("65"), "grand total 65: {listing}");
    }

    #[test]
    fn rbreak_excluded_from_out_dataset() {
        let mut session = make_session();
        class_like(&mut session);
        let ast = ReportAst {
            data: Some(work_ref("C")),
            columns: Some(vec!["sex".into(), "age".into()]),
            defines: vec![
                Define {
                    var: "sex".into(),
                    usage: Usage::Group,
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
                Define {
                    var: "age".into(),
                    usage: Usage::Analysis("sum".into()),
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
            ],
            rbreak: Some(Break {
                var: None,
                summarize: true,
            }),
            out: Some(work_ref("RB")),
            ..report_defaults()
        };
        execute(&ast, &mut session).unwrap();
        let (out, _) = session.libs.get("WORK").unwrap().read("RB").unwrap();
        // Only the 2 group rows; the RBREAK grand total is not written.
        assert_eq!(out.n_obs(), 2);
    }

    #[test]
    fn compute_simple_assignment() {
        let mut session = make_session();
        class_like(&mut session);
        // computed column `dbl` = age * 2 in a detail report.
        let ast = ReportAst {
            data: Some(work_ref("C")),
            columns: Some(vec!["sex".into(), "age".into(), "dbl".into()]),
            defines: vec![
                Define {
                    var: "age".into(),
                    usage: Usage::Display,
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
                Define {
                    var: "dbl".into(),
                    usage: Usage::Computed,
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
            ],
            computes: vec![Compute {
                target: "dbl".into(),
                stmts: vec![ComputeStmt::Assign {
                    col: "dbl".into(),
                    expr: parse_test_expr("age * 2;"),
                }],
            }],
            ..report_defaults()
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // First row age 11 → dbl 22; age 15 → dbl 30.
        assert!(listing.contains("22"), "11*2=22: {listing}");
        assert!(listing.contains("30"), "15*2=30: {listing}");
    }

    #[test]
    fn compute_after_line_text() {
        let mut session = make_session();
        class_like(&mut session);
        let ast = ReportAst {
            data: Some(work_ref("C")),
            columns: Some(vec!["sex".into(), "age".into()]),
            defines: vec![
                Define {
                    var: "sex".into(),
                    usage: Usage::Group,
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
                Define {
                    var: "age".into(),
                    usage: Usage::Analysis("sum".into()),
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
            ],
            computes: vec![Compute {
                target: "after".into(),
                stmts: vec![ComputeStmt::Line(vec![LineItem::Literal(
                    "End of report".into(),
                )])],
            }],
            ..report_defaults()
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("End of report"), "line text: {listing}");
    }

    #[test]
    fn where_missing_semantics_dot_equals_dot() {
        let mut session = make_session();
        let df = df![
            "g" => ["a", "b", "c"],
            "x" => [Some(1.0_f64), None, Some(3.0)]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("g"), num_meta("x")],
        };
        write_dataset(&mut session, "M", ds);
        // where x = . → only the row with missing x survives (g='b').
        let ast = ReportAst {
            data: Some(work_ref("M")),
            columns: Some(vec!["g".into(), "x".into()]),
            defines: vec![Define {
                var: "x".into(),
                usage: Usage::Display,
                order: OrderDir::Ascending,
                label: None,
                format: None,
                width: None,
                spacing: None,
            }],
            where_: Some(
                parse_test_expr("x = .;"),
            ),
            ..report_defaults()
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains('b'), "missing row kept: {listing}");
        assert!(!listing.contains('a'), "non-missing filtered: {listing}");
    }

    #[test]
    fn across_with_descending_direction() {
        let mut session = make_session();
        let df = df![
            "g" => ["x", "x"],
            "k" => [1.0_f64, 2.0],
            "v" => [5.0_f64, 7.0]
        ]
        .unwrap();
        let ds = SasDataset {
            df,
            vars: vec![char_meta("g"), num_meta("k"), num_meta("v")],
        };
        write_dataset(&mut session, "AD", ds);
        let ast = ReportAst {
            data: Some(work_ref("AD")),
            columns: Some(vec!["g".into(), "k".into(), "v".into()]),
            defines: vec![
                Define {
                    var: "g".into(),
                    usage: Usage::Group,
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
                Define {
                    var: "k".into(),
                    usage: Usage::Across,
                    order: OrderDir::Descending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
                Define {
                    var: "v".into(),
                    usage: Usage::Analysis("sum".into()),
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
            ],
            ..report_defaults()
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // Descending across order → header for k=2 appears before k=1.
        let i2 = listing.find("2 SUM").unwrap();
        let i1 = listing.find("1 SUM").unwrap();
        assert!(i2 < i1, "descending across: {listing}");
    }

    #[test]
    fn break_without_summarize_emits_no_subtotal() {
        let mut session = make_session();
        class_like(&mut session);
        let ast = ReportAst {
            data: Some(work_ref("C")),
            columns: Some(vec!["sex".into(), "age".into()]),
            defines: vec![
                Define {
                    var: "sex".into(),
                    usage: Usage::Group,
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
                Define {
                    var: "age".into(),
                    usage: Usage::Analysis("n".into()),
                    order: OrderDir::Ascending,
                    label: None,
                    format: None,
                    width: None,
                    spacing: None,
                },
            ],
            breaks: vec![Break {
                var: Some("sex".into()),
                summarize: false,
            }],
            ..report_defaults()
        };
        // Should not panic; n for F=3, M=2.
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains('3'), "F n=3: {listing}");
        assert!(listing.contains('2'), "M n=2: {listing}");
    }

    // ─────────────────── M33.5 deferred-option tests ───────────────────

    /// Build a DEFINE with optional format/width/spacing (M33.5 test helper).
    fn def(
        var: &str,
        usage: Usage,
        label: Option<&str>,
        format: Option<&str>,
        width: Option<usize>,
        spacing: Option<usize>,
    ) -> Define {
        Define {
            var: var.into(),
            usage,
            order: OrderDir::Ascending,
            label: label.map(|s| s.to_string()),
            format: format.map(|s| s.to_string()),
            width,
            spacing,
        }
    }

    #[test]
    fn parse_define_format_width_spacing() {
        let ast = parse_report(
            "proc report data=a; \
             define x / analysis sum format=dollar8.2 width=10 spacing=4; run;",
        )
        .unwrap();
        let d = &ast.defines[0];
        assert_eq!(d.format.as_deref(), Some("dollar8.2"));
        assert_eq!(d.width, Some(10));
        assert_eq!(d.spacing, Some(4));
    }

    #[test]
    fn parse_define_flow_still_errors() {
        // FLOW is genuinely deferred → clean error at parse.
        let r = parse_report("proc report data=a; define x / display flow; run;");
        assert!(r.err().unwrap().to_string().contains("FLOW"));
    }

    #[test]
    fn format_applies_to_displayed_numeric() {
        // DEFINE / FORMAT=5.1 on a detail numeric column. Oracle: 11 → "11.0".
        let mut session = make_session();
        let df = df!["age" => [11.0_f64, 12.0]].unwrap();
        let ds = SasDataset { df, vars: vec![num_meta("age")] };
        write_dataset(&mut session, "T", ds);
        let ast = ReportAst {
            data: Some(work_ref("T")),
            columns: Some(vec!["age".into()]),
            defines: vec![def("age", Usage::Display, None, Some("5.1"), None, None)],
            ..report_defaults()
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("11.0"), "formatted 11.0: {listing}");
        assert!(listing.contains("12.0"), "formatted 12.0: {listing}");
    }

    #[test]
    fn width_truncates_and_pads_column() {
        // WIDTH=3 on a char column truncates long values to 3 chars.
        let mut session = make_session();
        let df = df!["name" => ["Alfred", "Bo"]].unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("name")] };
        write_dataset(&mut session, "T", ds);
        let ast = ReportAst {
            data: Some(work_ref("T")),
            columns: Some(vec!["name".into()]),
            defines: vec![def("name", Usage::Display, None, None, Some(3), None)],
            ..report_defaults()
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // "Alfred" truncated to "Alf"; full name must NOT appear.
        assert!(listing.contains("Alf"), "truncated to Alf: {listing}");
        assert!(!listing.contains("Alfred"), "full name truncated away: {listing}");
    }

    #[test]
    fn spacing_changes_intercolumn_gap() {
        // SPACING=6 before the second column → at least 6 spaces precede it.
        let mut session = make_session();
        let df = df![
            "a" => ["x", "y"],
            "b" => ["p", "q"]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("a"), char_meta("b")] };
        write_dataset(&mut session, "T", ds);
        let ast = ReportAst {
            data: Some(work_ref("T")),
            noheader: true,
            columns: Some(vec!["a".into(), "b".into()]),
            defines: vec![
                def("a", Usage::Display, None, None, None, None),
                def("b", Usage::Display, None, None, None, Some(6)),
            ],
            ..report_defaults()
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // Row "x" then 6 spaces (spacing) then "p". Default leading spacing is 2
        // on column a. So a data line should contain "x      p" (1+6 = the gap).
        assert!(listing.contains("x      p"), "6-space gap: {listing:?}");
    }

    #[test]
    fn compute_reads_cn_positional_reference() {
        // _C2_ is the 2nd COLUMN (age); ratio column = _C2_ / 10. Detail report.
        let mut session = make_session();
        let df = df![
            "sex" => ["F", "M"],
            "age" => [20.0_f64, 30.0]
        ]
        .unwrap();
        let ds = SasDataset { df, vars: vec![char_meta("sex"), num_meta("age")] };
        write_dataset(&mut session, "T", ds);
        let ast = ReportAst {
            data: Some(work_ref("T")),
            columns: Some(vec!["sex".into(), "age".into(), "ratio".into()]),
            defines: vec![
                def("age", Usage::Display, None, None, None, None),
                def("ratio", Usage::Computed, None, None, None, None),
            ],
            computes: vec![Compute {
                target: "ratio".into(),
                stmts: vec![ComputeStmt::Assign {
                    col: "ratio".into(),
                    expr: parse_test_expr("_c2_ / 10;"),
                }],
            }],
            ..report_defaults()
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // age 20 → ratio 2; age 30 → ratio 3.
        assert!(listing.contains('2'), "_c2_/10 = 2: {listing}");
        assert!(listing.contains('3'), "_c2_/10 = 3: {listing}");
    }

    #[test]
    fn line_with_format_renders_via_format_engine() {
        // compute after; line 'Total: ' age best8.; → grand total formatted.
        let mut session = make_session();
        class_like(&mut session);
        let ast = ReportAst {
            data: Some(work_ref("C")),
            columns: Some(vec!["sex".into(), "age".into()]),
            defines: vec![
                def("sex", Usage::Group, None, None, None, None),
                def("age", Usage::Analysis("sum".into()), None, None, None, None),
            ],
            rbreak: Some(Break { var: None, summarize: true }),
            computes: vec![Compute {
                target: "after".into(),
                stmts: vec![ComputeStmt::Line(vec![
                    LineItem::Literal("Total age: ".into()),
                    LineItem::Expr(parse_test_expr("age;"), Some("best8.".into())),
                ])],
            }],
            ..report_defaults()
        };
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // Grand total of ages = 65, rendered by best8. as "65".
        assert!(listing.contains("Total age: 65"), "line with format: {listing}");
    }

    #[test]
    fn parse_line_with_pointer_and_format() {
        // `line @5 total best8.;` parses to Pointer + Expr-with-format.
        let ast = parse_report(
            "proc report data=a; compute after; line @5 age best8.; endcomp; run;",
        )
        .unwrap();
        match &ast.computes[0].stmts[0] {
            ComputeStmt::Line(items) => {
                assert!(matches!(items[0], LineItem::Pointer(5)));
                match &items[1] {
                    LineItem::Expr(_, fmt) => assert_eq!(fmt.as_deref(), Some("best8.")),
                    _ => panic!("expected Expr item"),
                }
            }
            _ => panic!("expected LINE"),
        }
    }
}
