use super::*;

pub(super) struct Env {
    pub(super) vars: HashMap<String, Matrix>,
    /// Matrices de chaînes (listes de noms), p.ex. `cn = {"x" "y"}`. Stockées à
    /// part car la valeur IML numérique est `Vec<Vec<f64>>`.
    pub(super) str_vars: HashMap<String, Vec<String>>,
    /// Datasets ouverts en écriture (CREATE … APPEND … CLOSE), clé = nom canonique.
    pub(super) open_writes: HashMap<String, OpenWrite>,
    /// Datasets ouverts en lecture (USE … READ … CLOSE), clé = nom canonique.
    pub(super) open_reads: std::collections::HashSet<String>,
}

impl Env {
    pub(super) fn new() -> Self {
        Env {
            vars: HashMap::new(),
            str_vars: HashMap::new(),
            open_writes: HashMap::new(),
            open_reads: std::collections::HashSet::new(),
        }
    }
}

pub(super) fn scalar(v: f64) -> Matrix {
    vec![vec![v]]
}

pub(super) fn as_scalar(m: &Matrix) -> Result<f64> {
    if m.len() == 1 && m[0].len() == 1 {
        Ok(m[0][0])
    } else {
        Err(SasError::runtime("IML: expected a scalar (1x1 matrix)"))
    }
}

pub(super) fn dims(m: &Matrix) -> (usize, usize) {
    (m.len(), m.first().map(|r| r.len()).unwrap_or(0))
}
