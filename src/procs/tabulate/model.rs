use super::*;

use crate::token::Span;

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
        /// Span of the name token (J02-P7 : diagnostics of the TABLE
        /// expression raised at parse time point at the offending name).
        span: Span,
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
    /// A CLASS variable binding: the class column index, the FORMATTED level
    /// (`key`, which selects the rows and labels the heading — J02-P7, SAS
    /// groups CLASS values by their formatted value) and the smallest
    /// unformatted value of the level (`level`: ordering and OUT= value).
    ClassLevel {
        col: usize,
        level: Value,
        key: String,
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

/// One decoded CLASS column (J02-P7). SAS groups the values of a CLASS
/// variable by their FORMATTED value — CLASS statement, GROUPINTERNAL:
/// « specifies not to apply formats to the class variables when PROC
/// TABULATE groups the values » — and orders the levels by their unformatted
/// values (ORDER=UNFORMATTED, the default). `keys` holds the formatted level
/// text of every observation, `values` the unformatted values.
#[derive(Debug, Clone)]
pub(super) struct ClassData {
    /// Index of the CLASS variable in the input data set.
    pub(super) col: usize,
    pub(super) values: Vec<Value>,
    pub(super) keys: Vec<String>,
}

impl ClassData {
    /// Decode a CLASS column and format every value with the variable's
    /// stored format (none: the historical BEST12. / character rendering).
    pub(super) fn decode(
        ds: &SasDataset,
        col: usize,
        catalog: &crate::formats::FormatCatalog,
    ) -> Result<ClassData> {
        let values = decode_column(ds, col)?;
        let spec = ds.vars[col].format.as_deref().and_then(FormatSpec::parse);
        let keys = values
            .iter()
            .map(|v| class_key(v, spec.as_ref(), catalog))
            .collect();
        Ok(ClassData { col, values, keys })
    }

    /// The column restricted to `rows` (a BY group without the excluded
    /// observations): row indices become `0..rows.len()`.
    pub(super) fn select(&self, rows: &[usize]) -> ClassData {
        ClassData {
            col: self.col,
            values: rows.iter().map(|&r| self.values[r].clone()).collect(),
            keys: rows.iter().map(|&r| self.keys[r].clone()).collect(),
        }
    }
}

/// Formatted text of a CLASS value: the stored format of the variable when it
/// has one, else the historical default rendering (BEST12. for numbers, the
/// value itself for characters, `.`/letter for missing values).
pub(super) fn class_key(
    v: &Value,
    spec: Option<&FormatSpec>,
    catalog: &crate::formats::FormatCatalog,
) -> String {
    match (spec, v) {
        (Some(spec), _) => catalog.format(v, spec).trim().to_string(),
        (None, Value::Char(s)) => s.trim_end().to_string(),
        (None, other) => level_label(other),
    }
}

/// Expand a `DimExpr` into a flat list of cells. Each cell is one column (or
/// one row stub). Stacking concatenates the cells of successive terms;
/// crossing builds the cartesian product of the factors' cell lists.
pub(super) fn expand_dim(
    dim: &DimExpr,
    class_cols: &[(String, usize)],
    var_cols: &[(String, usize)],
    class_data: &[ClassData],
    n_obs: usize,
) -> Result<Vec<Cell>> {
    let mut out: Vec<Cell> = Vec::new();
    for term in &dim.terms {
        out.extend(expand_term(term, class_cols, var_cols, class_data, n_obs)?);
    }
    Ok(out)
}

pub(super) fn expand_term(
    term: &Term,
    class_cols: &[(String, usize)],
    var_cols: &[(String, usize)],
    class_data: &[ClassData],
    n_obs: usize,
) -> Result<Vec<Cell>> {
    // Each factor expands to a list of cells; crossing = cartesian product
    // (concatenating atoms).
    let mut acc: Vec<Cell> = vec![Cell { atoms: Vec::new() }];
    for factor in &term.factors {
        let factor_cells = expand_factor(factor, class_cols, var_cols, class_data, n_obs)?;
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
    class_data: &[ClassData],
    n_obs: usize,
) -> Result<Vec<Cell>> {
    match factor {
        Factor::Group(inner) => expand_dim(inner, class_cols, var_cols, class_data, n_obs),
        Factor::Name {
            name,
            label,
            format,
            ..
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
                    // One cell per observed formatted level, in the order of
                    // the unformatted values. A CLASS label overrides every
                    // level header.
                    let cd = class_data
                        .iter()
                        .find(|c| c.col == ci)
                        .expect("class col decoded");
                    Ok(observed_levels(cd, n_obs)
                        .into_iter()
                        .map(|(lv, key)| Cell {
                            atoms: vec![Atom::ClassLevel {
                                col: ci,
                                level: lv,
                                key,
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

/// Observed levels of a CLASS column: one per distinct formatted value, with
/// the smallest unformatted value of the level (SAS sorts the observations by
/// their unformatted values — ORDER=UNFORMATTED « yields the same order as
/// PROC SORT » — then forms the formatted groups). Missing values only reach
/// this point under the MISSING option (otherwise their observations are
/// excluded beforehand); they sort first, as in PROC SORT.
pub(super) fn observed_levels(cd: &ClassData, n_obs: usize) -> Vec<(Value, String)> {
    let mut levels: Vec<(Value, String)> = Vec::new();
    let mut index: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (v, key) in cd.values.iter().zip(&cd.keys).take(n_obs) {
        match index.get(key.as_str()) {
            Some(&i) => {
                if v.sas_cmp(&levels[i].0) == Ordering::Less {
                    levels[i].0 = v.clone();
                }
            }
            None => {
                index.insert(key.as_str(), levels.len());
                levels.push((v.clone(), key.clone()));
            }
        }
    }
    levels.sort_by(|a, b| a.0.sas_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    levels
}
