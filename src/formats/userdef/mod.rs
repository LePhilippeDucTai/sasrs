//! Formats et informats utilisateur définis par PROC FORMAT (jalons M4/M18.2).
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `proc format; value sexfmt 1='Male' 2='Female' other='?'; run;`
//! produit un `UserFormat` à plages :
//! - plage = valeur unique, `low-<high` / `low<-high` (bornes exclusives
//!   côté `<`), `LOW`/`HIGH` symboliques, ou liste `1,2,3='x'`.
//! - `VALUE` (num→label), `VALUE $fmt` (char→label) ; `INVALUE` pour les
//!   informats utilisateur (M18.2).
//! - Résolution : première plage qui matche dans l'ordre de tri SAS des
//!   bornes ; sinon `other` ; sinon la valeur formatée en BEST./$.
//!
//! ## INVALUE (M18.2)
//! `invalue grade 'A'=4 'B'=3 'C'=2 'D'=1 'F'=0;`
//! produit un `UserInformat` : les CLÉS sont des chaînes (plages de chaînes,
//! comme les formats char), la VALEUR de résultat est `Value::Num` ou
//! `Value::Char` selon que le nom porte `$` ou non.
//! - `invalue name` → résultat numérique ; clés = chaînes brutes à matcher.
//! - `invalue $name` → résultat caractère.
//! - La valeur de résultat peut être : littéral numérique `=4`, littéral
//!   chaîne `='Small'`, ou le mot-clé `_SAME_` (copie l'entrée non modifiée).
//! - `other=value` : valeur de repli si aucune plage ne correspond.
//! - Ranges de chaînes : `'A'-'F'` inclusive, `low-'C'`, `'D'-high`, avec
//!   bornes exclusives `<` ; comparison via `str::trim_end()` (insensible aux
//!   blancs finaux, fidèle SAS).
//! - Valeur inconnue (aucune plage, pas d'other) → missing.
//!
//! ## MIN=/MAX=/DEFAULT=/FUZZ= (M43.1)
//! `VALUE fmtname (min=<n> max=<n> default=<n> fuzz=<n>) range=label ...;`
//! ajoute quatre options FORMAT-LEVEL (une seule valeur par format, pas par
//! plage) portées par [`UserFormat::min`]/[`max`]/[`default`]/[`fuzz`] :
//! - `DEFAULT=` : largeur de sortie par défaut, utilisée quand le point
//!   d'usage (`put x fmtname.;`) n'en précise pas. `None` → largeur par
//!   défaut = longueur du plus long label (y compris `OTHER=`), calculée à
//!   la volée par [`UserFormat::effective_width`].
//! - `MIN=`/`MAX=` : bornent la largeur EFFECTIVE (explicite au point
//!   d'usage, ou `DEFAULT=`/calculée) dans `[MIN, MAX]`. `None` → pas de
//!   bornage (comportement historique, inchangé).
//! - `FUZZ=` : tolérance numérique appliquée aux comparaisons de bornes de
//!   plage (voir [`UserFormat::range_matches`]) — absorbe les écarts
//!   d'arrondi flottant près d'une borne. `None` → tolérance minuscule par
//!   défaut de SAS (`1e-12`, cf. [`DEFAULT_FUZZ`]), pas de tolérance nulle :
//!   c'est déjà le comportement de SAS réel sans `FUZZ=` explicite, et les
//!   plages des fixtures existantes (bornes entières bien séparées) ne
//!   croisent jamais ce seuil — d'où l'octet-identité préservée.
//!
//! **Invariant clé** : les quatre champs valent `None` pour tout `VALUE`
//! qui ne les précise pas (comportement historique par construction), et
//! [`UserFormat::effective_width`] a un chemin rapide qui reproduit EXACTEMENT
//! l'ancien calcul (`spec.w` si fourni, sinon label brut non tronqué) dans ce
//! cas — voir sa doc.

#![allow(unused_variables, dead_code)]

use crate::value::Value;
use std::cmp::Ordering;

/// Tolérance par défaut de SAS pour les comparaisons de bornes numériques
/// quand un `VALUE` n'a pas de `FUZZ=` explicite (cf. doc de module) —
/// absorbe les erreurs d'arrondi flottant, pas une "flouterie" visible par
/// l'utilisateur.
pub const DEFAULT_FUZZ: f64 = 1e-12;

mod informat;
mod picture;

pub use informat::InformatRange;
pub use informat::InformatValue;
pub use informat::UserInformat;
pub use picture::PictureDirectives;
pub use picture::PictureRange;
pub use picture::UserPicture;

/// The three kinds of user-defined format object that PROC FORMAT can build.
///
/// - `Value`   — `VALUE name range='label';`   (num/char → display label) → [`UserFormat`].
/// - `Picture` — `PICTURE name range='template' (dirs);` (num → templated digits) → [`UserPicture`].
/// - `Invalue` — `INVALUE name 'key'=result;`  (string → [`Value`]) → [`UserInformat`].
///
/// The three carry structurally different payloads, so they are stored in
/// separate maps in the [`crate::formats::FormatCatalog`]; this enum documents
/// the shared taxonomy and tags each stored object with its kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatKind {
    Value,
    Picture,
    Invalue,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum Bound {
    Low,
    High,
    Num(f64),
    Char(String),
}

impl Clone for Bound {
    fn clone(&self) -> Self {
        match self {
            Bound::Low => Bound::Low,
            Bound::High => Bound::High,
            Bound::Num(n) => Bound::Num(*n),
            Bound::Char(s) => Bound::Char(s.clone()),
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Range {
    pub from: Bound,
    pub to: Bound,
    pub from_exclusive: bool,
    pub to_exclusive: bool,
    pub label: String,
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct UserFormat {
    pub is_char: bool,
    pub ranges: Vec<Range>,
    pub other: Option<String>,
    /// M43.1 — `MIN=` : largeur minimale de sortie (format-level). `None` =
    /// pas de bornage bas (comportement historique).
    #[serde(default)]
    pub min: Option<u32>,
    /// M43.1 — `MAX=` : largeur maximale de sortie (format-level). `None` =
    /// pas de bornage haut (comportement historique).
    #[serde(default)]
    pub max: Option<u32>,
    /// M43.1 — `DEFAULT=` : largeur de sortie par défaut quand le point
    /// d'usage n'en précise pas. `None` = calculée (longueur du plus long
    /// label), voir [`UserFormat::effective_width`].
    #[serde(default)]
    pub default: Option<u32>,
    /// M43.1 — `FUZZ=` : tolérance de comparaison de bornes numériques.
    /// `None` = tolérance par défaut de SAS ([`DEFAULT_FUZZ`]), pas zéro —
    /// voir la doc de module.
    #[serde(default)]
    pub fuzz: Option<f64>,
}

impl UserFormat {
    pub fn lookup(&self, v: &Value) -> Option<&str> {
        for range in &self.ranges {
            if self.range_matches(range, v) {
                return Some(&range.label);
            }
        }
        self.other.as_deref()
    }

    /// M43.1 — largeur effective de sortie pour ce format à un point d'usage
    /// donné (`spec_w` = largeur explicite `fmtname8.`, `None` si le point
    /// d'usage n'en précise pas) :
    /// 1. `spec_w` si fourni, sinon `self.default`, sinon la longueur du
    ///    plus long label (plages + `OTHER=`) ;
    /// 2. bornée dans `[self.min, self.max]` si ces options sont posées.
    ///
    /// **Chemin rapide (octet-identité M4..M42)** : si `min`/`max`/`default`
    /// valent tous `None`, ceci renvoie EXACTEMENT `spec_w.map(|w| w as
    /// usize)` — le calcul historique de `formats::FormatCatalog::format`
    /// (`Some(w) => right_justify(w)`, `None => label brut`) — sans passer
    /// par le calcul de longueur de label ni le bornage, pour que le cas
    /// (très majoritaire) où aucune de ces options n'est postée reste
    /// identique au comportement d'avant M43.1.
    pub fn effective_width(&self, spec_w: Option<u16>) -> Option<usize> {
        if self.min.is_none() && self.max.is_none() && self.default.is_none() {
            return spec_w.map(|w| w as usize);
        }
        let mut w = match spec_w {
            Some(w) => w as usize,
            None => self
                .default
                .map(|d| d as usize)
                .unwrap_or_else(|| self.max_label_len()),
        };
        if let Some(min) = self.min {
            w = w.max(min as usize);
        }
        if let Some(max) = self.max {
            w = w.min(max as usize);
        }
        Some(w)
    }

    /// Longueur du plus long label (plages + `OTHER=`) — la largeur par
    /// défaut de SAS quand `DEFAULT=` n'est pas posé (voir doc de module).
    fn max_label_len(&self) -> usize {
        self.ranges
            .iter()
            .map(|r| r.label.len())
            .chain(self.other.iter().map(|s| s.len()))
            .max()
            .unwrap_or(0)
    }

    fn range_matches(&self, range: &Range, v: &Value) -> bool {
        if self.is_char {
            // Character format: match Value::Char against Bound::Char bounds.
            let s = match v {
                Value::Char(s) => s.trim_end(),
                _ => return false,
            };
            let from_ok = match &range.from {
                Bound::Low => true,
                Bound::High => false,
                Bound::Char(c) => {
                    let c = c.trim_end();
                    if range.from_exclusive { s > c } else { s >= c }
                }
                Bound::Num(_) => false,
            };
            if !from_ok {
                return false;
            }

            match &range.to {
                Bound::High => true,
                Bound::Low => false,
                Bound::Char(c) => {
                    let c = c.trim_end();
                    if range.to_exclusive { s < c } else { s <= c }
                }
                Bound::Num(_) => false,
            }
        } else {
            // Numeric format: match Value::Num against numeric bounds, with
            // a FUZZ tolerance (M43.1 — `self.fuzz`, defaulting to
            // `DEFAULT_FUZZ` when unset) so a value that lands within
            // `fuzz` of a boundary is treated as exactly AT that boundary —
            // see `fuzzy_cmp`.
            // Missing values don't match numeric ranges unless there is a
            // special handling; here we treat them as no-match (falls to `other`).
            let n = match v {
                Value::Num(n) => *n,
                _ => return false,
            };
            let fuzz = self.fuzz.unwrap_or(DEFAULT_FUZZ);
            let from_ok = match &range.from {
                Bound::Low => true,
                Bound::High => false,
                Bound::Num(lo) => {
                    let cmp = fuzzy_cmp(n, *lo, fuzz);
                    if range.from_exclusive {
                        cmp == Ordering::Greater
                    } else {
                        cmp != Ordering::Less
                    }
                }
                Bound::Char(_) => false,
            };
            if !from_ok {
                return false;
            }

            match &range.to {
                Bound::High => true,
                Bound::Low => false,
                Bound::Num(hi) => {
                    let cmp = fuzzy_cmp(n, *hi, fuzz);
                    if range.to_exclusive {
                        cmp == Ordering::Less
                    } else {
                        cmp != Ordering::Greater
                    }
                }
                Bound::Char(_) => false,
            }
        }
    }
}

/// Compare `n` à la borne `b` avec une tolérance `fuzz` (M43.1) : traite `n`
/// comme ÉGAL à `b` dès que `|n - b| <= fuzz`, sinon comparaison normale.
/// Utilisé pour les deux côtés d'une plage numérique (inclusif ET exclusif)
/// via [`UserFormat::range_matches`] : une borne inclusive s'ouvre légèrement
/// (une valeur "fuzzy-égale" matche), une borne exclusive se ferme
/// symétriquement (une valeur "fuzzy-égale" ne matche PAS) — cohérent avec
/// l'intention de SAS : une valeur si proche de la borne qu'elle EST la
/// borne, aux erreurs d'arrondi flottant près.
fn fuzzy_cmp(n: f64, b: f64, fuzz: f64) -> Ordering {
    let diff = n - b;
    if diff.abs() <= fuzz {
        Ordering::Equal
    } else if diff < 0.0 {
        Ordering::Less
    } else {
        Ordering::Greater
    }
}

// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
