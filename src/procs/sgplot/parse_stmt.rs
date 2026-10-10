use super::*;

/// Statement de tracé à deux variables :
/// `(x, y, group, markerattrs, degree, smooth)`.
type XyStmt = (
    String,
    String,
    Option<String>,
    Option<MarkerAttrs>,
    Option<u32>,
    Option<f64>,
);

/// Queue the display WARNING of an option the engine does not render (label
/// such as `GROUP=` or `STAT=SUM`).
fn warn_ignored(ts: &mut StatementStream, stmt: &str, label: &str) {
    ts.warn_ignored_display(contract::ignored_option("SGPLOT", Some(stmt), label));
}

/// Drive an option list through the terminating `;` (not consumed).
/// `handle(ts, name)` is called on the option name (not consumed) and returns
/// true once it has consumed an option it parses itself; any other option
/// gets the display WARNING and is consumed with its arguments (J02-P6: the
/// engine renders none of them — they used to be skipped silently).
fn parse_option_list<F>(ts: &mut StatementStream, stmt: &str, mut handle: F) -> Result<()>
where
    F: FnMut(&mut StatementStream, &str) -> Result<bool>,
{
    while !matches!(ts.peek().kind, TokenKind::Semi | TokenKind::Eof) {
        let Some(name) = ts.peek().ident().map(|s| s.to_ascii_lowercase()) else {
            return Err(contract::expected_option(ts, "SGPLOT", stmt));
        };
        if !handle(ts, &name)? {
            contract::warn_option(ts, "SGPLOT", Some(stmt));
        }
    }
    Ok(())
}

/// [`parse_option_list`] for the options after the `/` of a plot statement
/// (no-op without `/`).
fn parse_slash_options<F>(ts: &mut StatementStream, stmt: &str, handle: F) -> Result<()>
where
    F: FnMut(&mut StatementStream, &str) -> Result<bool>,
{
    if ts.peek().kind != TokenKind::Slash {
        return Ok(());
    }
    ts.next();
    parse_option_list(ts, stmt, handle)
}

/// Parse les options X=var Y=var et les options après `/` d'un statement de
/// tracé à deux variables (SCATTER, SERIES, REG, LOESS ; mot-clé `kw` déjà
/// consommé). Renvoie `(x, y, group, markerattrs, degree, smooth)`.
///
/// J02-P6 : seuls X= et Y= précèdent le `/` (syntaxe SAS 9.4) ; tout autre
/// jeton y est une ERROR (il était sauté). Après le `/`, DEGREE= (REG) et
/// SMOOTH= (LOESS) sont honorés ; GROUP= et MARKERATTRS= restent lus dans
/// l'AST mais, comme toute autre option, ne sont pas rendus : WARNING.
pub(super) fn parse_xy_stmt(ts: &mut StatementStream, kw: &str) -> Result<XyStmt> {
    let stmt = kw.to_ascii_uppercase();
    let mut x: Option<String> = None;
    let mut y: Option<String> = None;
    let mut group: Option<String> = None;
    let mut markerattrs: Option<MarkerAttrs> = None;
    let mut degree: Option<u32> = None;
    let mut smooth: Option<f64> = None;

    // Args avant le `/`.
    while !matches!(
        ts.peek().kind,
        TokenKind::Semi | TokenKind::Slash | TokenKind::Eof
    ) {
        match ts.peek().ident().map(|s| s.to_ascii_lowercase()).as_deref() {
            Some("x") => {
                common::consume_option_eq(ts, "X")?;
                x = Some(expect_ident(ts, "after X=")?);
            }
            Some("y") => {
                common::consume_option_eq(ts, "Y")?;
                y = Some(expect_ident(ts, "after Y=")?);
            }
            _ => {
                return Err(SasError::parse(
                    format!("Expected X=, Y= or '/' in the {stmt} statement of PROC SGPLOT."),
                    ts.peek().span,
                ));
            }
        }
    }

    // Options après le `/`.
    parse_slash_options(ts, &stmt, |ts, name| {
        match (kw, name) {
            ("scatter" | "series", "group") => {
                warn_ignored(ts, &stmt, "GROUP=");
                ts.next();
                expect_eq(ts, "GROUP")?;
                group = Some(expect_ident(ts, "after GROUP=")?);
            }
            ("scatter", "markerattrs") => {
                warn_ignored(ts, &stmt, "MARKERATTRS=");
                ts.next();
                if ts.peek().kind == TokenKind::Eq && ts.peek2().kind == TokenKind::LParen {
                    ts.next();
                    let mut m = MarkerAttrs {
                        symbol: None,
                        color: None,
                        size: None,
                    };
                    for (k, v) in parse_paren_attrs(ts) {
                        match k.as_str() {
                            "symbol" => m.symbol = Some(v),
                            "color" => m.color = Some(v),
                            "size" => m.size = Some(v),
                            _ => {}
                        }
                    }
                    markerattrs = Some(m);
                } else {
                    contract::skip_option_args(ts);
                }
            }
            ("reg", "degree") => {
                ts.next();
                expect_eq(ts, "DEGREE")?;
                degree = Some(expect_number(ts, "after DEGREE=")? as u32);
            }
            ("loess", "smooth") => {
                ts.next();
                expect_eq(ts, "SMOOTH")?;
                smooth = Some(expect_number(ts, "after SMOOTH=")?);
            }
            _ => return Ok(false),
        }
        Ok(true)
    })?;

    let x = x.ok_or_else(|| SasError::parse("missing X= in plot statement", ts.peek().span))?;
    let y = y.ok_or_else(|| SasError::parse("missing Y= in plot statement", ts.peek().span))?;
    Ok((x, y, group, markerattrs, degree, smooth))
}

/// Parse un statement de barres (VBAR/HBAR, mot-clé `kw` déjà consommé) :
/// `vbar category / response= stat=`.
///
/// J02-P6 : le moteur dessine toujours la fréquence des catégories.
/// RESPONSE= et STAT= autre que FREQ (SUM, MEAN, mais aussi MEDIAN, PERCENT
/// ou une valeur inconnue, qui retombaient en silence sur FREQ) restent lus
/// dans l'AST et donnent un WARNING ; les autres options aussi.
pub(super) fn parse_bar_stmt(
    ts: &mut StatementStream,
    kw: &str,
) -> Result<(String, Option<String>, BarStat)> {
    let stmt = kw.to_ascii_uppercase();
    let category = expect_ident(ts, "after VBAR/HBAR")?;
    let mut response: Option<String> = None;
    let mut stat = BarStat::Freq;
    parse_slash_options(ts, &stmt, |ts, name| {
        match name {
            "response" => {
                warn_ignored(ts, &stmt, "RESPONSE=");
                ts.next();
                expect_eq(ts, "RESPONSE")?;
                response = Some(expect_ident(ts, "after RESPONSE=")?);
            }
            "stat" => {
                ts.next();
                expect_eq(ts, "STAT")?;
                let s = expect_ident(ts, "after STAT=")?;
                stat = match s.to_ascii_lowercase().as_str() {
                    "sum" => BarStat::Sum,
                    "mean" => BarStat::Mean,
                    _ => BarStat::Freq,
                };
                if !s.eq_ignore_ascii_case("freq") {
                    warn_ignored(ts, &stmt, &format!("STAT={}", s.to_ascii_uppercase()));
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    })?;
    Ok((category, response, stat))
}

/// Parse un statement HISTOGRAM : `histogram var / binwidth= scale=`.
///
/// J02-P6 : BINWIDTH= est honoré ; le moteur dessine des effectifs, donc
/// SCALE= autre que COUNT (lu dans l'AST) et les autres options → WARNING.
pub(super) fn parse_histogram_stmt(
    ts: &mut StatementStream,
) -> Result<(String, Option<f64>, HistScale)> {
    let var = expect_ident(ts, "after HISTOGRAM")?;
    let mut binwidth: Option<f64> = None;
    let mut scale = HistScale::Count;
    parse_slash_options(ts, "HISTOGRAM", |ts, name| {
        match name {
            "binwidth" => {
                ts.next();
                expect_eq(ts, "BINWIDTH")?;
                binwidth = Some(expect_number(ts, "after BINWIDTH=")?);
            }
            "scale" => {
                ts.next();
                expect_eq(ts, "SCALE")?;
                let s = expect_ident(ts, "after SCALE=")?;
                scale = match s.to_ascii_lowercase().as_str() {
                    "percent" => HistScale::Percent,
                    "proportion" => HistScale::Proportion,
                    _ => HistScale::Count,
                };
                if !s.eq_ignore_ascii_case("count") {
                    warn_ignored(
                        ts,
                        "HISTOGRAM",
                        &format!("SCALE={}", s.to_ascii_uppercase()),
                    );
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    })?;
    Ok((var, binwidth, scale))
}

/// Parse un statement DENSITY : `density var / type=kernel|normal`. Renvoie
/// `(var, kernel)`.
///
/// TYPE=KERNEL|NORMAL est honoré (KERNEL / NORMAL nus restent acceptés comme
/// raccourcis). J02-P6 : une autre valeur de TYPE= retombait en silence sur
/// NORMAL et les paramètres `TYPE=KERNEL(C=…)` / `TYPE=NORMAL(MU=…)` étaient
/// avalés : WARNING, comme pour les autres options.
pub(super) fn parse_density_stmt(ts: &mut StatementStream) -> Result<(String, bool)> {
    let var = expect_ident(ts, "after DENSITY")?;
    let mut kernel = false;
    parse_slash_options(ts, "DENSITY", |ts, name| {
        match name {
            "kernel" => {
                ts.next();
                kernel = true;
            }
            "normal" => {
                ts.next();
                kernel = false;
            }
            "type" => {
                ts.next();
                expect_eq(ts, "TYPE")?;
                let t = expect_ident(ts, "after TYPE=")?;
                let upper = t.to_ascii_uppercase();
                match upper.as_str() {
                    "KERNEL" => kernel = true,
                    "NORMAL" => kernel = false,
                    _ => {
                        kernel = false;
                        warn_ignored(ts, "DENSITY", &format!("TYPE={upper}"));
                    }
                }
                if ts.peek().kind == TokenKind::LParen {
                    ts.warn_ignored_display(format!(
                        "The parameters of TYPE={upper} in the DENSITY statement are ignored in \
                         PROC SGPLOT; display customization is not supported."
                    ));
                    ts.skip_balanced_parens();
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    })?;
    Ok((var, kernel))
}

/// Parse un statement VBOX : `vbox response / category=var`. Renvoie
/// `(category, response)` ; les options autres que CATEGORY= → WARNING.
pub(super) fn parse_vbox_stmt(ts: &mut StatementStream) -> Result<(Option<String>, String)> {
    let response = expect_ident(ts, "after VBOX")?;
    let mut category: Option<String> = None;
    parse_slash_options(ts, "VBOX", |ts, name| {
        if name != "category" {
            return Ok(false);
        }
        ts.next();
        expect_eq(ts, "CATEGORY")?;
        category = Some(expect_ident(ts, "after CATEGORY=")?);
        Ok(true)
    })?;
    Ok((category, response))
}

/// Parse un statement AXIS (XAXIS/YAXIS, mot-clé `kw` déjà consommé) :
/// `xaxis label='..' values=(..) type=`.
///
/// LABEL= est honoré. J02-P6 : VALUES= n'est rendu que par ses deux premiers
/// nombres, pris comme bornes de l'axe (graduations et valeurs suivantes
/// perdues) → WARNING qui le dit ; TYPE= (lu dans l'AST, jamais rendu) et
/// toute autre option d'axe → WARNING.
pub(super) fn parse_axis_stmt(ts: &mut StatementStream, kw: &str) -> Result<AxisOpts> {
    let stmt = kw.to_ascii_uppercase();
    let mut opts = AxisOpts {
        label: None,
        values_min: None,
        values_max: None,
        type_: None,
    };
    parse_option_list(ts, &stmt, |ts, name| {
        match name {
            "label" => {
                ts.next();
                expect_eq(ts, "LABEL")?;
                opts.label = read_value(ts);
            }
            "type" => {
                ts.next();
                expect_eq(ts, "TYPE")?;
                let t = expect_ident(ts, "after TYPE=")?;
                warn_ignored(ts, &stmt, &format!("TYPE={}", t.to_ascii_uppercase()));
                opts.type_ = Some(match t.to_ascii_lowercase().as_str() {
                    "log" => AxisType::Log,
                    "discrete" => AxisType::Discrete,
                    _ => AxisType::Linear,
                });
            }
            // VALUES=(min to max by step) ou (v1 v2 ...) ; une autre forme
            // tombe dans le WARNING générique.
            "values"
                if ts.peek2().kind == TokenKind::Eq && ts.peek_nth(2).kind == TokenKind::LParen =>
            {
                ts.next(); // values
                ts.next(); // =
                let nums = contract::value_list_numbers(ts);
                if let Some(&mn) = nums.first() {
                    opts.values_min = Some(mn);
                }
                if nums.len() >= 2 {
                    // 2e nombre = max pour (min to max [by step]).
                    opts.values_max = Some(nums[1]);
                }
                ts.warn_ignored_display(contract::partial_value_list(
                    "SGPLOT", &stmt, "VALUES=", &nums,
                ));
            }
            _ => return Ok(false),
        }
        Ok(true)
    })?;
    Ok(opts)
}
