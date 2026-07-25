//! Parsing des statements de PROC REG (appelé après `proc reg`).

use super::*;


mod model;
mod options;
mod plots;

use model::*;
use options::*;
use plots::*;

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC REG. Called AFTER `proc reg` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<RegAst> {
    let ProcOptions {
        input,
        simple,
        corr,
        proc_all,
        outest,
        outsscp,
        ridge,
        pcomit,
        outvif,
        mut plot_requests,
    } = parse_proc_options(ts)?;

    // Sub-statements until run;/quit;
    let mut models: Vec<RegModelEntry> = Vec::new();
    let mut plots_requested = false;
    let mut plot_statements: Vec<PlotPair> = Vec::new();
    let mut weight: Option<String> = None;
    let mut freq: Option<String> = None;
    let mut by: Vec<String> = Vec::new();
    let mut id: Vec<String> = Vec::new();
    // M36.10 run-group / VAR bookkeeping.
    let mut var_list: Vec<String> = Vec::new();
    let mut reweight_seen = false;
    let mut refit_seen = false;
    let mut paint_seen = false;

    common::parse_proc_body(ts, |ts, kw| {
        if kw == "model" {
            models.push(parse_model_stmt(ts, proc_all)?);
            Ok(true)
        } else if kw == "output" {
            parse_output_stmt(ts, &mut models)?;
            Ok(true)
        } else if kw == "plots" {
            // M29.3 / M36.11 — PLOTS request. Two surface forms:
            //   PLOTS=(…)  /  PLOTS(UNPACK)=…  — a typed request set (M36.11),
            //   bare PLOTS [/ options];        — the legacy deferred flag (M29.3).
            // `parse_plots_option` consumes the `plots` keyword + value when a
            // `=`/`(` follows; otherwise we fall back to the legacy flag.
            let nxt = &ts.peek2().kind;
            if matches!(nxt, TokenKind::Eq | TokenKind::LParen) {
                parse_plots_option(ts, &mut plot_requests);
                // Consume an optional trailing `/ options` and the terminator.
                ts.skip_to_semi();
            } else {
                ts.next();
                ts.skip_to_semi();
                plots_requested = true;
            }
            Ok(true)
        } else if kw == "plot" {
            // M36.11 — traditional `PLOT y*x [=symbol] [y2*x2 …] [/ opts];`.
            ts.next(); // consume "plot"
            parse_plot_statement(ts, &mut plot_statements);
            Ok(true)
        } else if kw == "test" {
            // `TEST eq [, eq ...];` (unlabeled form — a leading `label:` is
            // handled by the catch-all branch below, which rewrites the kw).
            ts.next(); // consume "test"
            let equations = parse_lin_eqs(ts)?;
            ts.expect_semi()?;
            if let Some(entry) = models.last_mut() {
                entry.tests.push(RegTest {
                    label: None,
                    equations,
                });
            }
            Ok(true)
        } else if kw == "restrict" {
            ts.next(); // consume "restrict"
            let equations = parse_lin_eqs(ts)?;
            ts.expect_semi()?;
            if let Some(entry) = models.last_mut() {
                entry.restricts.push(RegRestrict { equations });
            }
            Ok(true)
        } else if kw == "mtest" {
            // `MTEST [equations] [/ options];` (unlabeled — a leading `label:` is
            // handled by the catch-all label branch). Equations are optional; an
            // empty list means the default "all regressors = 0" hypothesis.
            ts.next(); // consume "mtest"
            let equations = parse_mtest_equations(ts)?;
            // Options after `/` are accepted and ignored (e.g. CANPRINT, PRINT).
            if ts.peek().kind == TokenKind::Slash {
                ts.skip_to_semi();
            } else {
                ts.expect_semi()?;
            }
            if let Some(entry) = models.last_mut() {
                entry.mtests.push(RegMtest { label: None, equations });
            }
            Ok(true)
        } else if kw == "add" {
            // `ADD x1 x2 …;` — run-group regressor addition (M36.10).
            ts.next();
            if let Some(entry) = models.last_mut() {
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    if let Some(name) = ts.peek().ident().map(str::to_string) {
                        entry.add.push(name);
                        ts.next();
                    } else {
                        ts.next();
                    }
                }
            } else {
                ts.skip_to_semi();
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "delete" {
            // `DELETE x1 x2 …;` — run-group regressor removal (M36.10).
            ts.next();
            if let Some(entry) = models.last_mut() {
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    if let Some(name) = ts.peek().ident().map(str::to_string) {
                        entry.delete.push(name);
                        ts.next();
                    } else {
                        ts.next();
                    }
                }
            } else {
                ts.skip_to_semi();
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "var" {
            // `VAR v1 v2 …;` — declare variables for later interactive editing
            // (M36.10). Recorded only.
            ts.next();
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    var_list.push(name);
                    ts.next();
                } else {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "reweight" {
            // `REWEIGHT <condition>;` — interactive reweighting. Deferred (M36.10).
            ts.next();
            ts.skip_to_semi();
            reweight_seen = true;
            Ok(true)
        } else if kw == "refit" {
            // `REFIT;` — interactive refit. Deferred (M36.10).
            ts.next();
            ts.skip_to_semi();
            refit_seen = true;
            Ok(true)
        } else if kw == "paint" {
            // `PAINT <…>;` — interactive plot painting. Deferred (M36.10).
            ts.next();
            ts.skip_to_semi();
            paint_seen = true;
            Ok(true)
        } else if kw == "weight" {
            // `WEIGHT var;` — a single weight variable (M36.7).
            ts.next(); // consume "weight"
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                weight = Some(name);
                ts.next();
            }
            ts.skip_to_semi();
            Ok(true)
        } else if kw == "freq" {
            // `FREQ var;` — a single frequency variable (M36.7).
            ts.next(); // consume "freq"
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                freq = Some(name);
                ts.next();
            }
            ts.skip_to_semi();
            Ok(true)
        } else if kw == "by" {
            // `BY var1 var2 …;` — by-group processing (M36.7). DESCENDING is
            // accepted but unused here (REG runs the same per-group analysis);
            // we keep just the variable names (in order).
            ts.next(); // consume "by"
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if ts.peek().is_kw("descending") {
                    ts.next();
                    continue;
                }
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    by.push(name);
                    ts.next();
                } else {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "id" {
            // `ID var1 …;` — identification variables (M36.7).
            ts.next(); // consume "id"
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    id.push(name);
                    ts.next();
                } else {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            Ok(true)
        } else if ts.peek2().kind == TokenKind::Colon && ts.peek_nth(2).is_kw("test") {
            // `label: TEST eq [, eq ...];` — the leading identifier is a label.
            let label = ts.peek().ident().map(str::to_string);
            ts.next(); // label ident
            ts.next(); // ':'
            ts.next(); // 'test'
            let equations = parse_lin_eqs(ts)?;
            ts.expect_semi()?;
            if let Some(entry) = models.last_mut() {
                entry.tests.push(RegTest { label, equations });
            }
            Ok(true)
        } else if ts.peek2().kind == TokenKind::Colon && ts.peek_nth(2).is_kw("mtest") {
            // `label: MTEST [eq …] [/ opts];` — the leading identifier is a label.
            let label = ts.peek().ident().map(str::to_string);
            ts.next(); // label ident
            ts.next(); // ':'
            ts.next(); // 'mtest'
            let equations = parse_mtest_equations(ts)?;
            if ts.peek().kind == TokenKind::Slash {
                ts.skip_to_semi();
            } else {
                ts.expect_semi()?;
            }
            if let Some(entry) = models.last_mut() {
                entry.mtests.push(RegMtest { label, equations });
            }
            Ok(true)
        } else {
            Ok(false)
        }
    })?;

    Ok(RegAst {
        data_options: RegDataOptions {
            input,
            outest,
            outsscp,
            ridge,
            pcomit,
            outvif,
        },
        models,
        plots_requested,
        plot_requests,
        plot_statements,
        weight,
        freq,
        by,
        id,
        simple,
        corr,
        var_list,
        reweight_seen,
        refit_seen,
        paint_seen,
    })
}
