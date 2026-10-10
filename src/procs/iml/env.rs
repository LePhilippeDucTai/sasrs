use super::*;

pub(super) struct Env {
    pub(super) vars: HashMap<String, Matrix>,
    /// Matrices de chaînes (listes de noms), p.ex. `cn = {"x" "y"}`. Stockées à
    /// part car la valeur IML numérique est `Vec<Vec<f64>>`.
    pub(super) str_vars: HashMap<String, Vec<String>>,
    /// Datasets ouverts en écriture (CREATE … APPEND … CLOSE), clé = nom
    /// canonique `LIB.NAME`, dans l'ordre des CREATE (J02-P5 : ceux encore
    /// ouverts sont fermés et écrits à QUIT dans cet ordre, déterministe).
    pub(super) open_writes: Vec<(String, OpenWrite)>,
    /// Datasets ouverts en lecture (USE … READ … CLOSE), clé = nom canonique.
    pub(super) open_reads: std::collections::HashSet<String>,
    /// WARNINGs émis pendant l'évaluation d'une instruction (division par
    /// zéro, J02-P5) ; l'évaluateur ne voit que `&Env`, l'exécuteur les
    /// transmet au log après chaque instruction.
    pub(super) warnings: std::cell::RefCell<Vec<String>>,
}

impl Env {
    pub(super) fn new() -> Self {
        Env {
            vars: HashMap::new(),
            str_vars: HashMap::new(),
            open_writes: Vec::new(),
            open_reads: std::collections::HashSet::new(),
            warnings: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// Met un WARNING en attente (émis à la fin de l'instruction courante).
    pub(super) fn warn(&self, msg: impl Into<String>) {
        self.warnings.borrow_mut().push(msg.into());
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
