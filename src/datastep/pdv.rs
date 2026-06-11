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

#![allow(unused_variables, dead_code)]

use crate::value::{Value, VarType};
use std::collections::HashMap;

pub struct PdvVar {
    pub name: String,
    pub ty: VarType,
    pub length: usize,
    pub retained: bool,
    pub from_input: bool,
    pub format: Option<String>,
}

pub struct Pdv {
    vars: Vec<PdvVar>,
    index: HashMap<String, usize>,
    values: Vec<Value>,
    pub n_: u64,
    pub error_: bool,
}

impl Pdv {
    pub fn new() -> Self {
        todo!()
    }

    /// Ajoute une variable (compile) ; renvoie son slot. Si déjà
    /// présente, renvoie le slot existant SANS modifier type/longueur
    /// (première référence fige tout).
    pub fn add_var(&mut self, var: PdvVar) -> usize {
        todo!()
    }

    pub fn slot(&self, name: &str) -> Option<usize> {
        todo!("lookup uppercase ; gérer _N_ / _ERROR_ côté évaluateur")
    }

    pub fn vars(&self) -> &[PdvVar] {
        todo!()
    }

    pub fn get(&self, slot: usize) -> &Value {
        todo!()
    }

    /// Assignation avec troncature char à `length` (cf. tête de fichier).
    pub fn set(&mut self, slot: usize, v: Value) {
        todo!()
    }

    pub fn reset_non_retained(&mut self) {
        todo!()
    }
}
