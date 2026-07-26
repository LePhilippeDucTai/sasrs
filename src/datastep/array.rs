/// Définition compilée d'un array (M16.2). `slots` = slots PDV des
/// éléments dans l'ordre row-major ; `dims` = bornes supérieures de chaque
/// dimension (borne inférieure = 1, comme SAS). Un array 1-D a `dims` de
/// longueur 1 ; le produit des `dims` égale toujours `slots.len()`.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrayDef {
    pub slots: Vec<usize>,
    pub dims: Vec<usize>,
}

impl ArrayDef {
    /// Traduit un sous-script multi-dimensionnel (1-based par dimension) en
    /// index linéaire 0-based row-major. `None` si un indice est hors
    /// bornes (ou si le nombre d'indices ne correspond ni à `dims.len()`
    /// ni à 1 — accès linéaire). `indices` doit déjà être arrondi/entier.
    pub fn linear_index(&self, indices: &[i64]) -> Option<usize> {
        if indices.len() == 1 && self.dims.len() != 1 {
            // Accès linéaire sur array multi-dim : `arr{n}` → 1..=total.
            let n = indices[0];
            if n >= 1 && (n as usize) <= self.slots.len() {
                return Some(n as usize - 1);
            }
            return None;
        }
        if indices.len() != self.dims.len() {
            return None;
        }
        let mut linear: usize = 0;
        for (k, &idx) in indices.iter().enumerate() {
            let bound = self.dims[k];
            if idx < 1 || (idx as usize) > bound {
                return None;
            }
            linear = linear * bound + (idx as usize - 1);
        }
        Some(linear)
    }
}
