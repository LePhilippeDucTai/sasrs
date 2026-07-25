use super::*;

// ───────────────────────── Parser helpers ─────────────────────────

/// Parse a list of numeric coefficients from the token stream.
/// Reads numbers (and optional leading minus sign) until `;` or `/`.
pub(super) fn parse_coefficients(ts: &mut StatementStream) -> Vec<f64> {
    let mut coeffs = Vec::new();
    loop {
        let kind = ts.peek().kind.clone();
        match kind {
            TokenKind::Semi | TokenKind::Slash | TokenKind::Eof => break,
            TokenKind::Minus => {
                // Could be a negative number: consume `-` then number
                ts.next();
                let next_kind = ts.peek().kind.clone();
                if let TokenKind::Num(v) = next_kind {
                    coeffs.push(-v);
                    ts.next();
                } else {
                    // Not a number — stop
                    break;
                }
            }
            TokenKind::Num(v) => {
                coeffs.push(v);
                ts.next();
            }
            _ => break,
        }
    }
    coeffs
}

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC GLM. Called AFTER `proc glm` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<GlmAst> {
    let mut input: Option<DatasetRef> = None;

    // PROC GLM statement options until `;`
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }
        if ts.peek().is_kw("data") {
            input = Some(common::parse_dataset_opt(ts, "DATA")?);
        } else {
            ts.next();
        }
    }

    // Sub-statements until run;/quit;
    let mut class_vars: Vec<String> = Vec::new();
    let mut model: Option<GlmModel> = None;
    let mut lsmeans_vars: Vec<String> = Vec::new();
    let mut estimates: Vec<GlmEstimate> = Vec::new();
    let mut contrasts: Vec<GlmContrast> = Vec::new();
    let mut means_vars: Vec<String> = Vec::new();

    common::parse_proc_body(ts, |ts, kw| {
        if kw == "class" {
            ts.next();
            class_vars = ts.parse_name_list()?;
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "model" {
            ts.next();
            // Read dependents: idents before `=` (the `=` itself is consumed).
            let dependents = common::parse_model_lhs(ts);
            // Read effects: idents (optionally joined by `*`) after `=` until `/` or `;`.
            // Build both the legacy flat `effects` list and the structured
            // `effect_terms` (Vec of CLASS-var-name lists) for the multiway engine.
            let (effects, effect_terms) = common::parse_effect_terms(ts);
            let mut solution = false;
            let mut noprint = false;
            if ts.peek().kind == TokenKind::Slash {
                ts.next();
                // Parse options until semi
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    if ts.peek().is_kw("solution") {
                        solution = true;
                    }
                    if ts.peek().is_kw("noprint") {
                        noprint = true;
                    }
                    ts.next();
                }
            }
            ts.expect_semi()?;
            model = Some(GlmModel {
                dependents,
                effects,
                effect_terms,
                solution,
                noprint,
            });
            Ok(true)
        } else if kw == "lsmeans" {
            ts.next();
            // Read lsmeans vars (idents before `/` or `;`)
            let mut vars: Vec<String> = Vec::new();
            loop {
                if ts.peek().kind == TokenKind::Semi
                    || ts.peek().kind == TokenKind::Eof
                    || ts.peek().kind == TokenKind::Slash
                {
                    break;
                }
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    vars.push(name);
                    ts.next();
                } else {
                    ts.next();
                }
            }
            // Skip options after `/`
            if ts.peek().kind == TokenKind::Slash {
                ts.next();
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            lsmeans_vars = vars;
            Ok(true)
        } else if kw == "estimate" {
            ts.next();
            // Read label (string literal)
            let label = if let TokenKind::Str { value, .. } = ts.peek().kind.clone() {
                ts.next();
                value
            } else {
                String::new()
            };
            // Read effect (ident)
            let effect = if let Some(name) = ts.peek().ident().map(str::to_string) {
                ts.next();
                name
            } else {
                String::new()
            };
            // Read coefficients
            let coefficients = parse_coefficients(ts);
            // Skip options after `/` if any
            if ts.peek().kind == TokenKind::Slash {
                ts.next();
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            estimates.push(GlmEstimate {
                label,
                effect,
                coefficients,
            });
            Ok(true)
        } else if kw == "contrast" {
            ts.next();
            // Read label (string literal)
            let label = if let TokenKind::Str { value, .. } = ts.peek().kind.clone() {
                ts.next();
                value
            } else {
                String::new()
            };
            // Read effect (ident)
            let effect = if let Some(name) = ts.peek().ident().map(str::to_string) {
                ts.next();
                name
            } else {
                String::new()
            };
            // Read coefficients
            let coefficients = parse_coefficients(ts);
            // Skip options after `/` if any
            if ts.peek().kind == TokenKind::Slash {
                ts.next();
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            contrasts.push(GlmContrast {
                label,
                effect,
                coefficients,
            });
            Ok(true)
        } else if kw == "means" {
            ts.next();
            means_vars = ts.parse_name_list()?;
            ts.expect_semi()?;
            Ok(true)
        } else {
            Ok(false)
        }
    })?;

    Ok(GlmAst {
        data_options: GlmDataOptions { input },
        class_vars,
        model,
        lsmeans_vars,
        estimates,
        contrasts,
        means_vars,
    })
}
