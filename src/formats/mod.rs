//! Moteur de formats/informats SAS (jalon M4).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! - `FormatSpec::parse("DATE9.")` → { name:"DATE", w:Some(9), d:None } ;
//!   `"8.2"` → { name:"", w:8, d:2 } ; `"$CHAR10."` → { name:"$CHAR",
//!   w:10 }. Un format se reconnaît au `.` final dans la source — le
//!   parser fournit la chaîne déjà assemblée.
//! - `FormatCatalog` : formats utilisateur (PROC FORMAT) par nom upcase,
//!   en session uniquement (pas de catalogue persistant — limitation
//!   documentée).
//! - `format(value, spec)` : ordre de résolution — format utilisateur,
//!   sinon builtin, sinon fallback BESTw. / $w. Missings spéciaux →
//!   `A`..`Z`/`_`, `.` → `.` (à respecter dans TOUS les formats
//!   numériques).
//! - `informat(s, spec)` : symétrique pour INPUT().

#![allow(unused_variables, dead_code)]

pub mod builtin;
pub mod userdef;

use crate::value::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct FormatSpec {
    /// Nom sans largeur ("DATE", "$CHAR", "" pour w.d), upcase.
    pub name: String,
    pub w: Option<u16>,
    pub d: Option<u16>,
}

impl FormatSpec {
    pub fn parse(s: &str) -> Option<FormatSpec> {
        todo!("cf. exemples en tête de fichier")
    }
}

#[derive(Default)]
pub struct FormatCatalog {
    user: HashMap<String, userdef::UserFormat>,
}

impl FormatCatalog {
    pub fn define(&mut self, name: &str, fmt: userdef::UserFormat) {
        todo!()
    }

    /// PUT : valeur → texte (largeur exacte `w`, aligné comme SAS).
    pub fn format(&self, v: &Value, spec: &FormatSpec) -> String {
        todo!()
    }

    /// INPUT : texte → valeur.
    pub fn informat(&self, s: &str, spec: &FormatSpec) -> Value {
        todo!()
    }
}
