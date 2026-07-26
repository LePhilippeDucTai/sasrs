use super::*;

/// M36.11 — parse a `PLOTS` request: `PLOTS[(global-opts)]=keyword | (kw …)`.
/// Accepts the panel modifiers `PLOTS(UNPACK)=…` / `PLOTS(ONLY)=…`, the bare
/// keywords DIAGNOSTICS / RESIDUALS / FIT / ALL / NONE, and a parenthesised list
/// `PLOTS=(DIAGNOSTICS RESIDUALS FIT)`. Unknown keywords are consumed cleanly
/// (ignored). The `plots` keyword token is consumed by this function. On a
/// malformed form (no `=`) the keyword alone is consumed and nothing recorded.
pub(super) fn parse_plots_option(ts: &mut StatementStream, req: &mut PlotRequests) {
    ts.next(); // consume "plots"
    // Optional global-option parenthesis directly after PLOTS: PLOTS(UNPACK)=…
    if ts.peek().kind == TokenKind::LParen {
        ts.next();
        while ts.peek().kind != TokenKind::RParen
            && ts.peek().kind != TokenKind::Semi
            && ts.peek().kind != TokenKind::Eof
        {
            if let Some(id) = ts.peek().ident() {
                match id.to_ascii_uppercase().as_str() {
                    "UNPACK" => req.unpack = true,
                    "ONLY" => req.only = true,
                    _ => {}
                }
            }
            ts.next();
        }
        if ts.peek().kind == TokenKind::RParen {
            ts.next();
        }
    }
    // The value must be introduced by `=`. Without it there is nothing to record.
    if ts.peek().kind != TokenKind::Eq {
        return;
    }
    ts.next(); // consume "="
    req.explicit = true;
    // Either a parenthesised list of keywords, or a single keyword.
    if ts.peek().kind == TokenKind::LParen {
        ts.next();
        while ts.peek().kind != TokenKind::RParen
            && ts.peek().kind != TokenKind::Semi
            && ts.peek().kind != TokenKind::Eof
        {
            if let Some(id) = ts.peek().ident() {
                apply_plot_keyword(id, req);
            }
            ts.next();
        }
        if ts.peek().kind == TokenKind::RParen {
            ts.next();
        }
    } else if let Some(id) = ts.peek().ident().map(str::to_string) {
        apply_plot_keyword(&id, req);
        ts.next();
    }
}

/// Apply one PLOTS= keyword to the request set. Unknown keywords are ignored.
pub(super) fn apply_plot_keyword(kw: &str, req: &mut PlotRequests) {
    match kw.to_ascii_uppercase().as_str() {
        "DIAGNOSTICS" | "DIAGNOSTIC" => req.diagnostics = true,
        "RESIDUALS" | "RESIDUAL" | "RESIDUALPLOT" => req.residuals = true,
        "FIT" | "FITPLOT" => req.fit = true,
        "ALL" => req.all = true,
        "NONE" => req.none = true,
        _ => {}
    }
}

/// M36.11 — parse one axis term of a traditional `PLOT` statement. Handles the
/// `keyword.` special variables (`PREDICTED.`/`P.`, `RESIDUAL.`/`R.`) and plain
/// variable names. The trailing `.` of a keyword variable is consumed when
/// present. Returns `None` at a non-identifier token.
pub(super) fn parse_plot_var(ts: &mut StatementStream) -> Option<PlotVar> {
    let name = ts.peek().ident().map(str::to_string)?;
    ts.next();
    // A trailing dot marks a SAS keyword variable (PREDICTED. / RESIDUAL. / …).
    let has_dot = ts.peek().kind == TokenKind::Dot;
    if has_dot {
        ts.next();
        match name.to_ascii_uppercase().as_str() {
            "PREDICTED" | "PRED" | "P" => return Some(PlotVar::Predicted),
            "RESIDUAL" | "RESID" | "R" => return Some(PlotVar::Residual),
            // An unknown `keyword.` — treat its bare name as a model variable.
            _ => return Some(PlotVar::Named(name.to_ascii_uppercase())),
        }
    }
    Some(PlotVar::Named(name.to_ascii_uppercase()))
}

/// M36.11 — parse the body of a `PLOT y*x [=symbol] [y2*x2 …] [/ opts];` statement
/// (the `plot` keyword has already been consumed). Each `y*x` pair is recorded;
/// an optional `=symbol` after a pair and a trailing `/ options` clause are
/// consumed and ignored. Stops at `;`/EOF.
pub(super) fn parse_plot_statement(ts: &mut StatementStream, out: &mut Vec<PlotPair>) {
    loop {
        match ts.peek().kind {
            TokenKind::Semi => {
                ts.next();
                break;
            }
            TokenKind::Eof => break,
            TokenKind::Slash => {
                // Trailing `/ options` — consume the rest of the statement.
                ts.skip_to_semi();
                break;
            }
            _ => {}
        }
        // Expect `y * x`. If we cannot read a y, skip a token to make progress.
        let y = match parse_plot_var(ts) {
            Some(v) => v,
            None => {
                ts.next();
                continue;
            }
        };
        if ts.peek().kind != TokenKind::Star {
            // Not a valid pair (e.g. a stray symbol); skip and continue.
            continue;
        }
        ts.next(); // consume "*"
        let x = match parse_plot_var(ts) {
            Some(v) => v,
            None => continue,
        };
        out.push(PlotPair { y, x });
        // Optional `=symbol` (a plot symbol assignment) — consume `= token`.
        if ts.peek().kind == TokenKind::Eq {
            ts.next();
            if ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                ts.next();
            }
        }
    }
}
