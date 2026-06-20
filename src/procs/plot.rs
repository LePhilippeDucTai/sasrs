//! PROC PLOT — historical ASCII scatter plot (M30.2).
//!
//! PROC PLOT is SAS's original line-printer graphics procedure: it draws scatter
//! plots in the LISTING using ASCII characters, with no image files. When ODS
//! GRAPHICS is active it instead delegates to the image infrastructure (like
//! PROC SGPLOT / GPLOT).
//!
//! # Execution model by state
//!
//! - `ods_graphics.enabled == false` → render an ASCII scatter into the listing.
//! - `enabled == true`, default build (no `--features graphics`) → NOTE
//!   « image deferred ».
//! - `enabled == true`, `--features graphics` → the image is materialized and a
//!   `Output '...' (WxH) written.` NOTE is emitted.
//!
//! # Plot statement syntax
//!
//! ```sas
//! plot y*x;            /* scatter y vs x */
//! plot y*x='*';        /* use '*' as the plotting symbol */
//! plot (y1 y2)*x;      /* several y on the same x */
//! plot y*x=group;      /* one symbol per group value */
//! ```
//!
//! v1 only RENDERS the simple `y*x` form (ASCII or image). The other syntactic
//! forms PARSE without error; multi-y / group rendering is deferred (a NOTE).

use crate::ast::DatasetRef;
use crate::error::{Result, SasError};
use crate::missing::value_to_num;
use crate::parser::StatementStream;
use crate::procs::common::{self, decode_column};
use crate::session::Session;
use crate::token::TokenKind;
use crate::value::VarType;

// ───────────────────────── AST ─────────────────────────

#[derive(Debug, Clone)]
pub struct PlotAst {
    /// `DATA=` ; `None` → `_LAST_`.
    pub data_ref: Option<DatasetRef>,
    /// PLOT statements. v1 renders each in turn.
    pub plots: Vec<PlotStmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlotStmt {
    Plot {
        /// One or more y variables (`plot (y1 y2)*x;`).
        y_vars: Vec<String>,
        /// The x variable.
        x_var: String,
        /// `plot y*x=group;` — one plotting symbol per group value (deferred).
        group_var: Option<String>,
        /// `plot y*x='*';` — explicit plotting symbol.
        symbol: Option<char>,
    },
}

// ───────────────────────── Parser helpers ─────────────────────────

fn expect_ident(ts: &mut StatementStream, ctx: &str) -> Result<String> {
    match ts.peek().ident().map(str::to_string) {
        Some(s) => {
            ts.next();
            Ok(s)
        }
        None => Err(SasError::parse(
            format!("expected an identifier {ctx}"),
            ts.peek().span,
        )),
    }
}

/// Parse the left-hand side of a PLOT request: either a single y identifier or
/// a parenthesized list `(y1 y2 ...)`.
fn parse_y_vars(ts: &mut StatementStream) -> Result<Vec<String>> {
    if ts.peek().kind == TokenKind::LParen {
        ts.next(); // (
        let mut ys = Vec::new();
        while ts.peek().kind != TokenKind::RParen && ts.peek().kind != TokenKind::Eof {
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                ys.push(name);
                ts.next();
            } else {
                ts.next(); // skip separators / unexpected tokens
            }
        }
        if ts.peek().kind == TokenKind::RParen {
            ts.next();
        }
        if ys.is_empty() {
            return Err(SasError::parse(
                "expected at least one y variable in PLOT",
                ts.peek().span,
            ));
        }
        Ok(ys)
    } else {
        Ok(vec![expect_ident(ts, "for y variable in PLOT")?])
    }
}

/// Parse one PLOT request after the `plot` keyword: `y*x[=group|='sym']`.
fn parse_plot_request(ts: &mut StatementStream) -> Result<PlotStmt> {
    let y_vars = parse_y_vars(ts)?;

    // The `*` separator between y and x.
    if ts.peek().kind != TokenKind::Star {
        return Err(SasError::parse(
            "expected '*' between y and x variables in PLOT",
            ts.peek().span,
        ));
    }
    ts.next(); // *

    let x_var = expect_ident(ts, "for x variable in PLOT")?;

    // Optional `=group` or `='symbol'`.
    let mut group_var: Option<String> = None;
    let mut symbol: Option<char> = None;
    if ts.peek().kind == TokenKind::Eq {
        ts.next(); // =
        match &ts.peek().kind {
            TokenKind::Str { value, .. } => {
                symbol = value.chars().next();
                ts.next();
            }
            TokenKind::Ident(_) => {
                group_var = Some(expect_ident(ts, "after '=' in PLOT")?);
            }
            _ => {
                return Err(SasError::parse(
                    "expected a group variable or quoted symbol after '=' in PLOT",
                    ts.peek().span,
                ));
            }
        }
    }

    // Skip any trailing `/ options` (BOX, HAXIS=, etc.) — parsed, ignored in v1.
    if ts.peek().kind == TokenKind::Slash {
        ts.skip_to_semi();
        return Ok(PlotStmt::Plot {
            y_vars,
            x_var,
            group_var,
            symbol,
        });
    }

    Ok(PlotStmt::Plot {
        y_vars,
        x_var,
        group_var,
        symbol,
    })
}

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC PLOT. Called AFTER `proc plot` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<PlotAst> {
    let mut data_ref: Option<DatasetRef> = None;

    // PROC PLOT statement options, until `;`.
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            common::expect_eq(ts, "DATA")?;
            data_ref = Some(ts.parse_dataset_ref()?);
        } else {
            ts.next(); // ignore unknown PROC-level options
        }
    }

    let mut plots: Vec<PlotStmt> = Vec::new();

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

        if ts.peek().is_kw("plot") {
            ts.next(); // plot
            let stmt = parse_plot_request(ts)?;
            ts.expect_semi()?;
            plots.push(stmt);
        } else {
            ts.skip_to_semi();
        }
    }

    Ok(PlotAst { data_ref, plots })
}

// ───────────────────────── Resolve DATA= ─────────────────────────

/// Extract a numeric column by name (proper error if absent / non-numeric).
fn numeric_column(ds: &crate::dataset::SasDataset, name: &str) -> Result<Vec<f64>> {
    let idx = ds
        .vars
        .iter()
        .position(|m| m.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| SasError::runtime(format!("Variable {} not found.", name.to_uppercase())))?;
    if ds.vars[idx].ty != VarType::Num {
        return Err(SasError::runtime(format!(
            "Variable {} must be numeric for PROC PLOT.",
            name.to_uppercase()
        )));
    }
    let col = decode_column(ds, idx)?;
    Ok(col
        .iter()
        .map(|v| value_to_num(v).unwrap_or(f64::NAN))
        .collect())
}

// ───────────────────────── ASCII rendering ─────────────────────────

const GRID_ROWS: usize = 20;
const GRID_COLS: usize = 60;
/// Left margin reserved for the Y-axis tick labels + the `|`/`+` axis column.
const Y_LABEL_WIDTH: usize = 8;

/// Format a tick value as a compact label (integers drop the `.0`).
fn tick_label(v: f64) -> String {
    if v.is_finite() && (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.2}")
    }
}

/// The plotting character for an overlapped count: 1 obs → 'A', 2 → 'B', …,
/// 26 → 'Z', then 'Z' for anything denser. If an explicit symbol was given it
/// overrides the letter (SAS uses the literal symbol for every point).
fn plot_char(count: usize, symbol: Option<char>) -> char {
    if let Some(s) = symbol {
        return s;
    }
    if count == 0 {
        ' '
    } else if count <= 26 {
        (b'A' + (count as u8 - 1)) as char
    } else {
        'Z'
    }
}

/// Render a single `y*x` scatter into the listing as ASCII art.
fn render_ascii(session: &mut Session, x_name: &str, y_name: &str, data: &[(f64, f64)]) {
    let x_disp = x_name.to_uppercase();
    let y_disp = y_name.to_uppercase();

    session.listing.page_header();
    session.listing.blank();
    let title = format!(
        "Plot of {y_disp}*{x_disp}.  Legend: A = 1 obs, B = 2 obs, etc."
    );
    let ls = session.listing.ls();
    let pad = ls.saturating_sub(title.len()) / 2;
    session
        .listing
        .write_line(&format!("{}{}", " ".repeat(pad), title));
    session.listing.blank();

    if data.is_empty() {
        session
            .listing
            .write_line("   (no observations to plot)");
        session.listing.blank();
        return;
    }

    // Ranges (guard against a degenerate single-valued axis).
    let (mut x_min, mut x_max) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut y_min, mut y_max) = (f64::INFINITY, f64::NEG_INFINITY);
    for (x, y) in data {
        x_min = x_min.min(*x);
        x_max = x_max.max(*x);
        y_min = y_min.min(*y);
        y_max = y_max.max(*y);
    }
    if (x_max - x_min).abs() < f64::EPSILON {
        x_min -= 1.0;
        x_max += 1.0;
    }
    if (y_max - y_min).abs() < f64::EPSILON {
        y_min -= 1.0;
        y_max += 1.0;
    }

    // Overlap counts per cell. grid[row][col], row 0 = top (max y).
    let mut counts = vec![vec![0usize; GRID_COLS]; GRID_ROWS];
    for (x, y) in data {
        let cx = (((x - x_min) / (x_max - x_min)) * (GRID_COLS as f64 - 1.0)).round() as isize;
        let cy = (((y - y_min) / (y_max - y_min)) * (GRID_ROWS as f64 - 1.0)).round() as isize;
        let col = cx.clamp(0, GRID_COLS as isize - 1) as usize;
        let row_from_bottom = cy.clamp(0, GRID_ROWS as isize - 1) as usize;
        let row = GRID_ROWS - 1 - row_from_bottom; // flip so top = max y
        counts[row][col] += 1;
    }

    // Y-axis label (variable name) on its own line, left-aligned over the grid.
    session
        .listing
        .write_line(&format!("{:>width$} |", y_disp, width = Y_LABEL_WIDTH));

    // Body: every row, with a tick label + value on a subset of rows.
    for (r, row_counts) in counts.iter().enumerate() {
        // Tick value for this row (row 0 = y_max, last row = y_min).
        let frac = (GRID_ROWS - 1 - r) as f64 / (GRID_ROWS as f64 - 1.0);
        let yval = y_min + frac * (y_max - y_min);
        // Place a "+" tick label on roughly every 3rd row (and the extremes).
        let show_tick = r % 3 == 0 || r == GRID_ROWS - 1;
        let (label, axis) = if show_tick {
            (tick_label(yval), '+')
        } else {
            (String::new(), '|')
        };
        let mut line = format!("{:>width$} {}", label, axis, width = Y_LABEL_WIDTH);
        for &c in row_counts.iter() {
            line.push(if c == 0 { ' ' } else { plot_char(c, None) });
        }
        session.listing.write_line(&line);
    }

    // X-axis: a baseline of `-`, ticks every 5 columns, then value labels.
    let mut axis_line = format!("{:>width$} ", "", width = Y_LABEL_WIDTH);
    for col in 0..GRID_COLS {
        axis_line.push(if col % 5 == 0 { '+' } else { '-' });
    }
    session.listing.write_line(&axis_line);

    // X tick value labels under each tick column.
    let mut label_line = format!("{:>width$} ", "", width = Y_LABEL_WIDTH);
    let mut col = 0;
    while col < GRID_COLS {
        let frac = col as f64 / (GRID_COLS as f64 - 1.0);
        let xval = x_min + frac * (x_max - x_min);
        let lbl = tick_label(xval);
        // Pad/truncate the label into the 5-column tick slot.
        while label_line.len() < Y_LABEL_WIDTH + 1 + col {
            label_line.push(' ');
        }
        label_line.push_str(&lbl);
        col += 5;
    }
    // Append the x variable name at the far right.
    label_line.push_str("  ");
    label_line.push_str(&x_disp);
    session.listing.write_line(&label_line);
    session.listing.blank();
}

// ───────────────────────── Execute ─────────────────────────

pub fn execute(ast: &PlotAst, session: &mut Session) -> Result<()> {
    if ast.plots.is_empty() {
        session
            .log
            .note("No PLOT statement found in PROC PLOT; nothing to plot.");
        return Ok(());
    }

    // Read the input dataset once.
    let in_ref = common::resolve_last_dataset(&ast.data_ref, session)?;
    let in_libref = in_ref.libref_or_work();
    let in_table = in_ref.name.to_uppercase();
    let provider = session.libs.get(&in_libref)?;
    let (ds, notes) = provider.read(&in_table)?;
    for note in notes {
        session.log.forward(&note);
    }

    for stmt in &ast.plots {
        let PlotStmt::Plot {
            y_vars,
            x_var,
            group_var,
            symbol,
        } = stmt;

        // v1: group rendering is deferred (a NOTE), the plot still draws.
        if group_var.is_some() {
            session
                .log
                .note("PROC PLOT v1: '=group' rendering deferred; plotting all observations with a single symbol.");
        }
        // v1: only the first y variable is rendered.
        if y_vars.len() > 1 {
            session.log.note(&format!(
                "PROC PLOT v1 renders only the first y variable ({}); {} additional variable(s) ignored.",
                y_vars[0].to_uppercase(),
                y_vars.len() - 1
            ));
        }

        let y_name = &y_vars[0];
        let xs = numeric_column(&ds, x_var)?;
        let ys = numeric_column(&ds, y_name)?;
        let data: Vec<(f64, f64)> = xs
            .iter()
            .zip(ys.iter())
            .filter(|(a, b)| a.is_finite() && b.is_finite())
            .map(|(a, b)| (*a, *b))
            .collect();

        if session.ods_graphics.enabled {
            render_image(session, x_var, y_name, &data, *symbol)?;
        } else {
            render_ascii(session, x_var, y_name, &data);
        }
    }

    Ok(())
}

/// Render (or defer) one `y*x` scatter to an image file when ODS GRAPHICS is on.
/// Default build: a deferral NOTE; `--features graphics`: a `plot_{N}` scatter.
fn render_image(
    session: &mut Session,
    x_name: &str,
    y_name: &str,
    data: &[(f64, f64)],
    symbol: Option<char>,
) -> Result<()> {
    #[cfg(not(feature = "graphics"))]
    {
        let _ = (x_name, y_name, data, symbol);
        session
            .log
            .note("ODS GRAPHICS: image deferred (compile with --features graphics).");
        Ok(())
    }

    #[cfg(feature = "graphics")]
    {
        use crate::graphics::render::{draw_to_file, DrawingSpec, PlotType};
        let _ = symbol; // marker shape is fixed in the image backend for v1.

        let spec = DrawingSpec {
            title: "The PLOT Procedure".to_string(),
            x_label: x_name.to_uppercase(),
            y_label: y_name.to_uppercase(),
            plot_type: PlotType::Scatter,
            data: data.to_vec(),
            x_categorical: vec![],
        };

        session.graphics_image_count += 1;
        let stem = session
            .ods_graphics
            .file_stem
            .clone()
            .unwrap_or_else(|| "plot".to_string());
        let fmt = session.ods_graphics.image_format;
        let name = format!(
            "{}_{}.{}",
            stem,
            session.graphics_image_count,
            fmt.extension()
        );
        let path = session.ods_graphics.output_dir.join(&name);
        let (w, h) = draw_to_file(
            &spec,
            &path,
            session.ods_graphics.width,
            session.ods_graphics.height,
            fmt,
        )?;
        session
            .log
            .note(&format!("Output '{}' ({}x{}) written.", name, w, h));
        Ok(())
    }
}

// ───────────────────────── Tests ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::source::SourceFile;
    use crate::value::VarType;
    use polars::df;

    fn make_session() -> Session {
        Session::new(None, std::env::temp_dir(), true).unwrap()
    }

    fn parse_plot(src: &str) -> Result<PlotAst> {
        let source = SourceFile::new(src);
        let mut ts = StatementStream::new(&source).unwrap();
        ts.next(); // proc
        ts.next(); // plot
        parse(&mut ts)
    }

    fn write_xy(session: &mut Session, table: &str) {
        let df = df![
            "x" => [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0],
            "y" => [10.0_f64, 20.0, 15.0, 30.0, 25.0, 40.0]
        ]
        .unwrap();
        let vars = vec![
            VarMeta {
                name: "x".into(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            },
            VarMeta {
                name: "y".into(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            },
        ];
        let ds = SasDataset { df, vars };
        session.libs.get("WORK").unwrap().write(table, &ds).unwrap();
        session.last_dataset = Some(format!("WORK.{table}"));
    }

    // ── Parse tests ──────────────────────────────────────────────────────

    #[test]
    fn parse_simple_plot() {
        let ast = parse_plot("proc plot data=a; plot y*x; run;").unwrap();
        assert_eq!(ast.plots.len(), 1);
        match &ast.plots[0] {
            PlotStmt::Plot {
                y_vars,
                x_var,
                group_var,
                symbol,
            } => {
                assert_eq!(y_vars, &vec!["y".to_string()]);
                assert_eq!(x_var, "x");
                assert!(group_var.is_none());
                assert!(symbol.is_none());
            }
        }
    }

    #[test]
    fn parse_plot_with_symbol() {
        let ast = parse_plot("proc plot data=a; plot y*x='*'; run;").unwrap();
        match &ast.plots[0] {
            PlotStmt::Plot { symbol, .. } => assert_eq!(*symbol, Some('*')),
        }
    }

    #[test]
    fn parse_plot_multiple_y() {
        let ast = parse_plot("proc plot data=a; plot (y1 y2)*x; run;").unwrap();
        match &ast.plots[0] {
            PlotStmt::Plot { y_vars, x_var, .. } => {
                assert_eq!(y_vars, &vec!["y1".to_string(), "y2".to_string()]);
                assert_eq!(x_var, "x");
            }
        }
    }

    #[test]
    fn parse_plot_with_group() {
        let ast = parse_plot("proc plot data=a; plot y*x=group; run;").unwrap();
        match &ast.plots[0] {
            PlotStmt::Plot { group_var, .. } => {
                assert_eq!(group_var.as_deref(), Some("group"));
            }
        }
    }

    // ── ASCII render tests (default behavior, ODS off) ────────────────────

    #[test]
    fn render_ascii_contains_title() {
        let mut session = make_session();
        write_xy(&mut session, "XY");
        let ast = parse_plot("proc plot data=work.xy; plot y*x; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        assert!(listing.contains("Plot of Y*X"), "listing: {listing}");
    }

    #[test]
    fn render_ascii_contains_min_max() {
        let mut session = make_session();
        write_xy(&mut session, "XY");
        let ast = parse_plot("proc plot data=work.xy; plot y*x; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        let listing = session.listing.into_string();
        // x ranges 1..6, y ranges 10..40 — extremes must appear as tick labels.
        assert!(listing.contains('1'), "listing: {listing}");
        assert!(listing.contains('6'), "listing: {listing}");
        assert!(listing.contains("10"), "listing: {listing}");
        assert!(listing.contains("40"), "listing: {listing}");
    }

    // ── ODS delegation tests ──────────────────────────────────────────────

    #[cfg(not(feature = "graphics"))]
    #[test]
    fn execute_with_ods_on_no_feature_defers() {
        let mut session = make_session();
        session.ods_graphics.enabled = true;
        write_xy(&mut session, "XY");
        let ast = parse_plot("proc plot data=work.xy; plot y*x; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(log.contains("image deferred"), "log: {log}");
    }

    #[cfg(feature = "graphics")]
    #[test]
    fn execute_with_ods_on_feature_writes_image() {
        let mut session = make_session();
        session.ods_graphics.enabled = true;
        session.ods_graphics.output_dir = std::env::temp_dir();
        session.ods_graphics.file_stem = Some("plottest_img".into());
        write_xy(&mut session, "XY");
        let ast = parse_plot("proc plot data=work.xy; plot y*x='*'; run;").unwrap();
        execute(&ast, &mut session).unwrap();
        let log = session.log.into_string();
        assert!(log.contains("written"), "log: {log}");
        let p = std::env::temp_dir().join("plottest_img_1.png");
        assert!(p.exists(), "image not created: {p:?}");
        assert!(p.metadata().unwrap().len() > 0);
        let _ = std::fs::remove_file(&p);
    }
}
