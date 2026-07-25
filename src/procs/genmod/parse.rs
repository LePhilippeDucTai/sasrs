use super::*;

// ───────────────────────── Parser ─────────────────────────

/// Parse PROC GENMOD. Called AFTER `proc genmod` has been consumed.
pub fn parse(ts: &mut StatementStream) -> Result<GenmodAst> {
    let mut input: Option<DatasetRef> = None;

    // PROC GENMOD statement options until `;`
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

    let mut class_vars: Vec<String> = Vec::new();
    let mut model: Option<GenmodModel> = None;
    let mut freq_var: Option<String> = None;

    common::parse_proc_body(ts, |ts, kw| {
        if kw == "class" {
            ts.next();
            while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                if let Some(name) = ts.peek().ident().map(str::to_string) {
                    class_vars.push(name);
                    ts.next();
                } else {
                    ts.next();
                }
            }
            ts.expect_semi()?;
            Ok(true)
        } else if kw == "model" {
            ts.next(); // consume "model"

            // Response variable
            let response = common::parse_model_response(ts, "expected response variable")?;

            // Optional response options: (event='val' descending ...)
            let (event, descending) = common::parse_response_options(ts);

            // Expect '='
            common::expect_model_eq(ts, "expected '=' after response variable in MODEL")?;

            // Predictors until '/' or ';'
            let predictors = common::parse_effect_list(ts);

            let mut dist_opt: Option<Distribution> = None;
            let mut link_opt: Option<LinkFunction> = None;
            let mut noprint = false;
            let mut scale_opt: Option<f64> = None;
            let mut noscale = false;

            if ts.peek().kind == TokenKind::Slash {
                ts.next(); // consume '/'
                // Parse options
                while ts.peek().kind != TokenKind::Semi && ts.peek().kind != TokenKind::Eof {
                    if ts.peek().is_kw("dist") {
                        ts.next();
                        if ts.peek().kind == TokenKind::Eq {
                            ts.next();
                        }
                        if let Some(name) = ts.peek().ident().map(str::to_string) {
                            ts.next();
                            match name.to_ascii_lowercase().as_str() {
                                "poisson" => dist_opt = Some(Distribution::Poisson),
                                "binomial" => dist_opt = Some(Distribution::Binomial),
                                "normal" => dist_opt = Some(Distribution::Normal),
                                "gamma" => dist_opt = Some(Distribution::Gamma),
                                _ => {} // ignore unknown
                            }
                        }
                    } else if ts.peek().is_kw("link") {
                        ts.next();
                        if ts.peek().kind == TokenKind::Eq {
                            ts.next();
                        }
                        if let Some(name) = ts.peek().ident().map(str::to_string) {
                            ts.next();
                            match name.to_ascii_lowercase().as_str() {
                                "log" => link_opt = Some(LinkFunction::Log),
                                "logit" => link_opt = Some(LinkFunction::Logit),
                                "identity" => link_opt = Some(LinkFunction::Identity),
                                "reciprocal" | "inverse" | "power" => {
                                    // POWER(-1) ≈ reciprocal; treat POWER as
                                    // reciprocal here (full power family deferred).
                                    link_opt = Some(LinkFunction::Reciprocal)
                                }
                                _ => {} // ignore unknown
                            }
                        }
                    } else if ts.peek().is_kw("noprint") {
                        noprint = true;
                        ts.next();
                    } else if ts.peek().is_kw("noscale") {
                        noscale = true;
                        ts.next();
                    } else if ts.peek().is_kw("scale") {
                        ts.next();
                        if ts.peek().kind == TokenKind::Eq {
                            ts.next();
                        }
                        // SCALE=<number>; accept numeric literal.
                        if let TokenKind::Num(v) = ts.peek().kind {
                            scale_opt = Some(v);
                            ts.next();
                        } else if let Some(s) = ts.peek().ident().map(str::to_string) {
                            if let Ok(v) = s.parse::<f64>() {
                                scale_opt = Some(v);
                            }
                            ts.next();
                        } else {
                            ts.next();
                        }
                    } else {
                        ts.next();
                    }
                }
            }
            ts.expect_semi()?;

            // Determine distribution (default Poisson if only link given)
            let dist = dist_opt.unwrap_or(Distribution::Poisson);
            // If LINK not given, use canonical link for the distribution
            let link = link_opt.unwrap_or_else(|| canonical_link(&dist));

            model = Some(GenmodModel {
                response,
                event,
                descending,
                predictors,
                dist,
                link,
                noprint,
                scale: scale_opt,
                noscale,
            });
            Ok(true)
        } else if kw == "freq" {
            ts.next();
            if let Some(name) = ts.peek().ident().map(str::to_string) {
                freq_var = Some(name);
                ts.next();
            }
            ts.expect_semi()?;
            Ok(true)
        } else {
            Ok(false)
        }
    })?;

    Ok(GenmodAst {
        data_options: GenmodDataOptions { input },
        model,
        freq_var,
        class_vars,
    })
}
