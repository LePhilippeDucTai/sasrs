//! Le Program Data Vector : la ligne de travail de l'étape DATA.
//!
//! # Plan du fichier — voir PLAN.md  (difficulté : MOYENNE, mécanique)
//!
//! - Lookup par nom INSENSIBLE À LA CASSE (index HashMap sur le nom
//!   uppercasé) ; l'affichage garde la casse de première référence.
//! - `set()` applique la sémantique de longueur fixe SAS : une valeur
//!   Char plus longue que `length` est TRONQUÉE silencieusement ; le
//!   stockage reste trimé (pas de padding), la comparaison ignorant les
//!   blancs finaux est déjà dans `Value::sas_cmp`.
//! - `reset_non_retained()` : début d'itération — remet à
//!   `Value::missing()` (num) / `Char("")` (char) toutes les variables
//!   NON retenues et NON issues d'un input (`from_input`). Les variables
//!   de SET gardent leur valeur jusqu'à la lecture suivante (règle SAS).
//! - Variables automatiques `_N_` et `_ERROR_` : champs dédiés, exposées
//!   à l'évaluateur comme des numériques en lecture seule.

use crate::value::{Value, VarType};
use std::collections::HashMap;

pub struct PdvVar {
    pub name: String,
    pub ty: VarType,
    pub length: usize,
    pub retained: bool,
    pub from_input: bool,
    pub format: Option<String>,
    /// Élément d'array `_TEMPORARY_` (M16.2) : n'est PAS écrit en sortie et
    /// est implicitement retenu (jamais remis à missing entre itérations).
    pub temporary: bool,
}

pub struct Pdv {
    vars: Vec<PdvVar>,
    /// Maps uppercase variable name → slot index.
    index: HashMap<String, usize>,
    values: Vec<Value>,
    pub n_: u64,
    pub error_: bool,
}

impl Pdv {
    pub fn new() -> Self {
        Pdv {
            vars: Vec::new(),
            index: HashMap::new(),
            values: Vec::new(),
            n_: 0,
            error_: false,
        }
    }

    /// Ajoute une variable (compile) ; renvoie son slot. Si déjà
    /// présente, renvoie le slot existant SANS modifier type/longueur
    /// (première référence fige tout).
    pub fn add_var(&mut self, var: PdvVar) -> usize {
        let key = var.name.to_uppercase();
        if let Some(&slot) = self.index.get(&key) {
            // Already exists — first declaration wins; return existing slot.
            return slot;
        }
        let slot = self.vars.len();
        // Initialise to missing (num) or empty string (char).
        let initial = match var.ty {
            VarType::Num => Value::missing(),
            VarType::Char => Value::Char(String::new()),
        };
        self.values.push(initial);
        self.index.insert(key, slot);
        self.vars.push(var);
        slot
    }

    /// Returns the slot index for a variable name (case-insensitive).
    /// `_N_` and `_ERROR_` are handled at the evaluator level; this method
    /// returns `None` for them so the evaluator can serve their dedicated
    /// fields directly.
    pub fn slot(&self, name: &str) -> Option<usize> {
        let upper = name.to_uppercase();
        // Automatic variables are not stored in the slot array.
        if upper == "_N_" || upper == "_ERROR_" {
            return None;
        }
        self.index.get(&upper).copied()
    }

    pub fn vars(&self) -> &[PdvVar] {
        &self.vars
    }

    pub fn get(&self, slot: usize) -> &Value {
        &self.values[slot]
    }

    /// Assignation avec troncature char à `length` (cf. tête de fichier).
    ///
    /// For `Char` values: if the new value is longer than `vars[slot].length`
    /// characters, it is silently truncated to that many characters. The
    /// stored string is *not* padded (trailing blanks are never stored).
    pub fn set(&mut self, slot: usize, v: Value) {
        let stored = match &self.vars[slot].ty {
            VarType::Char => {
                let max_len = self.vars[slot].length;
                match v {
                    Value::Char(s) => {
                        // Truncate to `max_len` *characters* (not bytes).
                        let truncated: String = s.chars().take(max_len).collect();
                        // Strip trailing blanks — we store trimmed.
                        let trimmed = truncated.trim_end().to_string();
                        Value::Char(trimmed)
                    }
                    // Assigning a non-Char value to a Char slot:
                    // treat as empty (type mismatch — evaluator should
                    // warn; we degrade gracefully).
                    other => {
                        let _ = other;
                        Value::Char(String::new())
                    }
                }
            }
            VarType::Num => {
                // Numeric slot: store as-is (type validation is the
                // evaluator's responsibility).
                v
            }
        };
        self.values[slot] = stored;
    }

    /// Marque une variable comme issue d'un input (SET) après coup — cas
    /// d'une variable créée par une référence textuelle antérieure au SET ;
    /// elle ne doit pas être remise à missing à chaque itération.
    pub fn mark_from_input(&mut self, slot: usize) {
        self.vars[slot].from_input = true;
    }

    /// Associe (ou remplace) le format d'affichage d'une variable. Utilisé
    /// par les statements FORMAT/ATTRIB (M4) à la compilation : le format
    /// déclaré l'emporte sur celui hérité de l'input.
    pub fn set_format(&mut self, slot: usize, format: String) {
        self.vars[slot].format = Some(format);
    }

    /// Réinitialise à missing les variables NON retenues ET NON issues
    /// d'un input (`from_input`). Appelé au début de chaque itération.
    ///
    /// - Variables `retained = true` : gardent leur valeur (RETAIN statement).
    /// - Variables `from_input = true` : gardent leur valeur jusqu'à la
    ///   prochaine lecture de dataset (SET statement).
    /// - Toutes les autres : remises à `.` (num) ou `""` (char).
    pub fn reset_non_retained(&mut self) {
        for (i, var) in self.vars.iter().enumerate() {
            if var.retained || var.from_input {
                continue;
            }
            self.values[i] = match var.ty {
                VarType::Num => Value::missing(),
                VarType::Char => Value::Char(String::new()),
            };
        }
    }
}

impl Default for Pdv {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
