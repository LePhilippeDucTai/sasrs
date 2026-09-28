use super::*;

/// METHOD= — comment deux valeurs numériques sont jugées (SAS 9.4, chap. 13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpMethod {
    Absolute,
    Relative,
    Exact,
    Percent,
}

impl CmpMethod {
    /// Valeur par défaut de METHOD= (doc SAS : ABSOLUTE).
    pub fn parse_kw(kw: &str) -> Option<CmpMethod> {
        Some(match kw {
            "absolute" => CmpMethod::Absolute,
            "relative" => CmpMethod::Relative,
            "exact" => CmpMethod::Exact,
            "percent" => CmpMethod::Percent,
            _ => return None,
        })
    }
}

pub struct CompareAst {
    pub base: DatasetRef,
    pub compare: DatasetRef,
    pub out: Option<DatasetRef>,
    pub novalues: bool,
    pub briefsummary: bool,
    /// J07-P3 — NOPRINT : aucun listing produit (la log et OUT= restent).
    pub noprint: bool,
    /// J07-P3 — BRIEF : rapport condensé (équivalent BRIEFSUMMARY).
    pub brief: bool,
    /// J07-P3 — LISTALL : la section valeurs liste toutes les variables
    /// comparées, égales comprises.
    pub listall: bool,
    /// J07-P9 — MAXPRINT=n | (n,p) (défauts 50/50) : n plafonne le nombre
    /// de différences imprimées par observation dans la section « Value
    /// Comparison Results », p le nombre d'observations avec différences
    /// imprimées. Une NOTE signale la troncature (doc SAS 9.4, MAXPRINT=).
    pub maxprint: (usize, usize),
    /// J07-P3 — CRITERION=c (défaut 0) : seuil de jugement METHOD=.
    pub criterion: f64,
    /// J07-P3 — METHOD= (défaut ABSOLUTE).
    pub method: CmpMethod,
    /// J07-P3 — OUT= : filtres de lignes écrites.
    pub outbase: bool,
    pub outcomp: bool,
    pub outdif: bool,
    pub outnoequal: bool,
    pub outpercent: bool,
    /// J07-P3 — ID v1 v2 ; : appariement des observations par clé.
    pub id: Vec<String>,
    /// J07-P3 — VAR v1 v2 ; / WITH w1 w2 ; : paires positionnelles.
    pub var: Vec<String>,
    pub with: Vec<String>,
    /// J07-P3 — BY v1 [DESCENDING v2] ; : comparaison par groupe.
    pub by: Vec<(String, bool)>,
}

/// Parse `proc compare base=... compare=... [options] ; [id ...;] [var ...;]
/// [with ...;] [by ...;] run;` — called AFTER "proc compare" has been
/// consumed.
pub fn parse(ts: &mut StatementStream) -> Result<CompareAst> {
    let mut base: Option<DatasetRef> = None;
    let mut compare: Option<DatasetRef> = None;
    let mut out: Option<DatasetRef> = None;
    let mut novalues = false;
    let mut briefsummary = false;
    let mut noprint = false;
    let mut brief = false;
    let mut listall = false;
    let mut maxprint: (usize, usize) = (50, 50);
    let mut criterion: f64 = 0.0;
    let mut method = CmpMethod::Absolute;
    let mut outbase = false;
    let mut outcomp = false;
    let mut outdif = false;
    let mut outnoequal = false;
    let mut outpercent = false;
    let mut id: Vec<String> = Vec::new();
    let mut var: Vec<String> = Vec::new();
    let mut with: Vec<String> = Vec::new();
    let mut by: Vec<(String, bool)> = Vec::new();

    // Parse header options until `;`
    loop {
        if ts.peek().kind == TokenKind::Semi {
            ts.next();
            break;
        }
        if ts.peek().kind == TokenKind::Eof {
            break;
        }

        if ts.peek().is_kw("base") {
            ts.next();
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse("expected '=' after BASE", ts.peek().span));
            }
            ts.next();
            base = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("compare") {
            ts.next();
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse(
                    "expected '=' after COMPARE",
                    ts.peek().span,
                ));
            }
            ts.next();
            compare = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("out") {
            ts.next();
            if ts.peek().kind != TokenKind::Eq {
                return Err(SasError::parse("expected '=' after OUT", ts.peek().span));
            }
            ts.next();
            out = Some(ts.parse_dataset_ref()?);
        } else if ts.peek().is_kw("novalues") {
            ts.next();
            novalues = true;
        } else if ts.peek().is_kw("briefsummary") {
            ts.next();
            briefsummary = true;
        } else if ts.peek().is_kw("noprint") {
            ts.next();
            noprint = true;
        } else if ts.peek().is_kw("brief") {
            // J07-P3 — BRIEF (raccourci documenté de BRIEFSUMMARY).
            ts.next();
            brief = true;
        } else if ts.peek().is_kw("listall") {
            ts.next();
            listall = true;
        } else if ts.peek().is_kw("criterion") {
            crate::procs::common::consume_option_eq(ts, "CRITERION")?;
            let tok = ts.peek().clone();
            match tok.kind {
                TokenKind::Num(f) if f >= 0.0 => {
                    ts.next();
                    criterion = f;
                }
                _ => {
                    return Err(SasError::parse(
                        "expected a non-negative number after CRITERION=",
                        tok.span,
                    ));
                }
            }
        } else if ts.peek().is_kw("method") {
            crate::procs::common::consume_option_eq(ts, "METHOD")?;
            let tok = ts.peek().clone();
            let val = tok.ident().map(|s| s.to_ascii_lowercase());
            match val.as_deref().and_then(CmpMethod::parse_kw) {
                Some(m) => {
                    ts.next();
                    method = m;
                }
                None => {
                    return Err(SasError::parse(
                        format!(
                            "Unexpected option 'METHOD={}' on PROC COMPARE statement.",
                            tok.ident().unwrap_or("?").to_uppercase()
                        ),
                        tok.span,
                    ));
                }
            }
        } else if ts.peek().is_kw("maxprint") {
            // MAXPRINT=n | MAXPRINT=(n,p) — n : nombre max de différences
            // imprimées par observation ; p : nombre max d'observations
            // avec différences imprimées. MAXPRINT=n seul laisse p à 50
            // (défauts SAS 9.4, option MAXPRINT=).
            crate::procs::common::consume_option_eq(ts, "MAXPRINT")?;
            let tok = ts.peek().clone();
            match tok.kind {
                TokenKind::Num(f) if f >= 0.0 && f.fract() == 0.0 => {
                    ts.next();
                    maxprint = (f as usize, 50);
                }
                TokenKind::LParen => {
                    ts.next();
                    let mut nums: Vec<usize> = Vec::new();
                    loop {
                        let tok = ts.peek().clone();
                        match tok.kind {
                            TokenKind::RParen => {
                                ts.next();
                                break;
                            }
                            TokenKind::Comma => {
                                ts.next();
                            }
                            TokenKind::Num(f) if f >= 0.0 && f.fract() == 0.0 => {
                                ts.next();
                                if nums.len() < 2 {
                                    nums.push(f as usize);
                                }
                            }
                            _ => {
                                return Err(SasError::parse(
                                    "expected a non-negative integer in MAXPRINT=",
                                    tok.span,
                                ));
                            }
                        }
                    }
                    match nums.as_slice() {
                        [n] => maxprint = (*n, 50),
                        [n, p] => maxprint = (*n, *p),
                        _ => {
                            return Err(SasError::parse(
                                "expected MAXPRINT=n or MAXPRINT=(n,p) with one or two non-negative integers",
                                tok.span,
                            ));
                        }
                    }
                }
                _ => {
                    return Err(SasError::parse(
                        "expected a non-negative integer in MAXPRINT=",
                        tok.span,
                    ));
                }
            }
        } else if ts.peek().is_kw("outbase") {
            ts.next();
            outbase = true;
        } else if ts.peek().is_kw("outcomp") {
            ts.next();
            outcomp = true;
        } else if ts.peek().is_kw("outdif") {
            ts.next();
            outdif = true;
        } else if ts.peek().is_kw("outnoequal") {
            ts.next();
            outnoequal = true;
        } else if ts.peek().is_kw("outpercent") {
            ts.next();
            outpercent = true;
        } else {
            // Unknown option: no silent skip (contrat J02-P3) — SAS would
            // reject it too.
            return Err(crate::procs::common::unknown_option_error(ts, "COMPARE"));
        }
    }

    // Sous-statements jusqu'à `run;`/`quit;` — J07-P3 : ID / VAR / WITH / BY.
    crate::procs::common::parse_proc_body(ts, "COMPARE", |ts, kw| {
        Ok(match kw {
            "id" => {
                ts.next();
                id.extend(crate::procs::common::parse_var_list(ts)?);
                true
            }
            "var" => {
                ts.next();
                var.extend(crate::procs::common::parse_var_list(ts)?);
                true
            }
            "with" => {
                ts.next();
                with.extend(crate::procs::common::parse_var_list(ts)?);
                true
            }
            "by" => {
                ts.next();
                by.extend(crate::procs::common::parse_by(ts)?);
                true
            }
            _ => false,
        })
    })?;

    let base = base.ok_or_else(|| {
        SasError::parse(
            "BASE= is required for PROC COMPARE",
            crate::token::Span::default(),
        )
    })?;
    let compare = compare.ok_or_else(|| {
        SasError::parse(
            "COMPARE= is required for PROC COMPARE",
            crate::token::Span::default(),
        )
    })?;
    if !with.is_empty() && with.len() != var.len() {
        return Err(SasError::runtime(
            "The WITH statement must name exactly as many variables as the VAR statement.",
        ));
    }

    Ok(CompareAst {
        base,
        compare,
        out,
        novalues,
        briefsummary,
        noprint,
        brief,
        listall,
        maxprint,
        criterion,
        method,
        outbase,
        outcomp,
        outdif,
        outnoequal,
        outpercent,
        id,
        var,
        with,
        by,
    })
}
