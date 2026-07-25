use super::*;

// ───────────────────────────── AST ─────────────────────────────

/// A parsed table-expression (raw, before CLASS/VAR resolution).
#[derive(Debug, Clone)]
pub(super) struct DimExpr {
    /// Stacked terms (concatenation by blanks).
    pub(super) terms: Vec<Term>,
}

#[derive(Debug, Clone)]
pub(super) struct Term {
    /// Factors crossed by `*`.
    pub(super) factors: Vec<Factor>,
}

#[derive(Debug, Clone)]
pub(super) enum Factor {
    /// An identifier (resolved to CLASS / VAR / stat at execute time), with an
    /// optional `='label'` header override and an optional `*f=<fmt>` cell
    /// format (both M33.4). Both are `None` on the default byte-identical path.
    Name {
        name: String,
        label: Option<String>,
        format: Option<String>,
    },
    /// A parenthesized sub-expression (distributes over crossings).
    Group(DimExpr),
}

// ───────────────────────── expansion ─────────────────────────

/// A single atom of an expanded cell. `label`/`format` carry the optional
/// M33.4 `='label'` header override and `*f=<fmt>` cell format from the
/// originating factor. Both are `None` on the default byte-identical path.
#[derive(Debug, Clone)]
pub(super) enum Atom {
    /// A CLASS variable binding: (class column index, observed level value).
    ClassLevel {
        col: usize,
        level: Value,
        label: Option<String>,
        format: Option<String>,
    },
    /// The analysis VAR column index.
    Var {
        col: usize,
        label: Option<String>,
        format: Option<String>,
    },
    /// A statistic keyword (lowercase).
    Stat {
        stat: String,
        label: Option<String>,
        format: Option<String>,
    },
    /// The universal class (marginal total): no CLASS constraint, labelled
    /// "All". Aggregates over every category of its dimension.
    All {
        label: Option<String>,
        format: Option<String>,
    },
}

impl Atom {
    /// The per-cell format override carried by this atom, if any.
    pub(super) fn format(&self) -> Option<&str> {
        match self {
            Atom::ClassLevel { format, .. }
            | Atom::Var { format, .. }
            | Atom::Stat { format, .. }
            | Atom::All { format, .. } => format.as_deref(),
        }
    }
}

/// A fully-expanded cell: an ordered crossing of atoms (used for the header
/// label and for selecting rows + computing a statistic).
#[derive(Debug, Clone)]
pub(super) struct Cell {
    pub(super) atoms: Vec<Atom>,
}

/// Classification of a TABLE identifier.
pub(super) enum Ident3 {
    Class(usize),
    Var(usize),
    Stat(String),
    All,
}

/// Resolve a name appearing in a TABLE expression to a CLASS col / VAR col /
/// stat keyword. Errors cleanly on anything else.
pub(super) fn classify(
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
pub(super) fn expand_dim(
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

pub(super) fn expand_term(
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

pub(super) fn expand_factor(
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
        Factor::Name {
            name,
            label,
            format,
        } => {
            let label = label.clone();
            let format = format.clone();
            match classify(name, class_cols, var_cols)? {
                Ident3::All => Ok(vec![Cell {
                    atoms: vec![Atom::All { label, format }],
                }]),
                Ident3::Stat(s) => Ok(vec![Cell {
                    atoms: vec![Atom::Stat {
                        stat: s,
                        label,
                        format,
                    }],
                }]),
                Ident3::Var(ci) => Ok(vec![Cell {
                    atoms: vec![Atom::Var {
                        col: ci,
                        label,
                        format,
                    }],
                }]),
                Ident3::Class(ci) => {
                    // Expand to one cell per observed (non-missing) level, in
                    // sas_cmp order. A CLASS label overrides every level header.
                    let vals = &class_values
                        .iter()
                        .find(|(c, _)| *c == ci)
                        .expect("class col decoded")
                        .1;
                    let levels = observed_levels(vals, n_obs);
                    Ok(levels
                        .into_iter()
                        .map(|lv| Cell {
                            atoms: vec![Atom::ClassLevel {
                                col: ci,
                                level: lv,
                                label: label.clone(),
                                format: format.clone(),
                            }],
                        })
                        .collect())
                }
            }
        }
    }
}

/// Observed non-missing levels of a CLASS column, ordered by `sas_cmp`.
pub(super) fn observed_levels(vals: &[Value], n_obs: usize) -> Vec<Value> {
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
