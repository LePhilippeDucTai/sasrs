//! Bibliothèque de fonctions SAS (table de dispatch).
//!
//! # Plan du fichier — voir PLAN.md  (difficulté : MOYENNE, mécanique et
//! table-driven — idéal pour un modèle économique, fonction par fonction)
//!
//! `call(name, args, ctx)` : nom matché en MAJUSCULES ; renvoie `None` si
//! la fonction est inconnue (l'évaluateur en fait une erreur).
//!
//! ## Lot M1/M2 (sémantique SAS exacte)
//! Statistiques sur arguments (IGNORENT les missings, contrairement aux
//! opérateurs !) :
//! - `SUM(a,b,...)`  somme des non-missings ; TOUS missings → `.`
//! - `MEAN`, `MIN`, `MAX`, `N` (nb non-missings), `NMISS`
//! - `COALESCE(a,b,...)` premier non-missing
//! - `MISSING(x)` → 1.0/0.0 (marche aussi sur char blanc)
//! Math :
//! - `ABS`, `SQRT` (négatif → `.` + invalid note), `EXP`,
//!   `LOG`/`LOG2`/`LOG10` (≤0 → `.` + note), `INT` (troncature vers 0),
//!   `ROUND(x[,unit])` — ATTENTION : round SAS = demi-arrondi loin de
//!   zéro (`(x/unit).round()` Rust fait déjà half-away-from-zero),
//!   `MOD(a,b)` — signe du résultat = signe de a (comme `%` Rust f64).
//! Caractères (les longueurs/blancs comptent — relire la doc SAS !) :
//! - `UPCASE`, `LOWCASE`, `TRIM` (blancs finaux ; chaîne blanche → ""),
//!   `STRIP`, `LEFT` (M1 : équivalent trim_start), `LENGTH` (sans blancs
//!   finaux, minimum 1 même pour ""), `SUBSTR(s, pos[, len])` (pos
//!   1-based ; hors bornes → "" + `_ERROR_` + note "Invalid ... argument"),
//!   `INDEX(s, sub)` (1-based, 0 si absent), `CAT` (concat brut),
//!   `CATS` (strip chaque arg), `CATX(sep, ...)` (strip + séparateur,
//!   args blancs sautés), `COMPRESS(s[,chars])` (défaut : enlève les
//!   espaces), `TRANWRD(s, from, to)`, `SCAN(s, n[, delims])` (n<0 =
//!   depuis la fin ; délimiteurs par défaut SAS : ` .<>()+&!$*);^-/,%|`).
//! Dates (M4 affinera avec les formats) :
//! - `TODAY()`/`DATE()` → jours depuis 1960 (sous --deterministic,
//!   l'exécuteur peut figer la date — passer l'info via EvalCtx si
//!   nécessaire), `MDY(m,d,y)` (invalide → `.` + note), `YEAR`, `MONTH`,
//!   `DAY`, `WEEKDAY` (dimanche=1).
//! Conversion :
//! - `INPUT(s, informat)` / `PUT(v, format)` → DÉLÉGUER au moteur
//!   formats/ (M4) ; M1-M3 : non disponibles (None).
//!
//! Arguments num : utiliser les helpers de coercition d'eval.rs (un char
//! passé à ABS déclenche la conversion automatique char→num).
//!
//! ## Tests
//! Table-driven : (nom, args, résultat attendu) — une trentaine de cas,
//! dont missings : `SUM(., .)` → `.`, `SUM(., 1)` → 1, `MEAN(1,.,3)` → 2.

#![allow(unused_variables, dead_code)]

use super::eval::EvalCtx;
use crate::value::Value;

/// Renvoie None si la fonction est inconnue.
pub fn call(name: &str, args: &[Value], ctx: &mut EvalCtx) -> Option<Value> {
    todo!("dispatch sur name.to_uppercase(), cf. lot en tête de fichier")
}
