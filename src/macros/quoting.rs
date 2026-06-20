//! Primitives de quoting (masquage par sentinelles) associées à `MacroEngine`.
//!
//! Schéma partagé par `%str`/`%nrstr`/`%bquote`/`%superq`/`%q*` : les caractères
//! spéciaux sont remplacés par des points de code de la zone privée Unicode
//! (`MASK_BASE + offset`) pour les rendre littéraux/inertes en aval ; une passe
//! `unmask` finale rétablit les littéraux d'origine.

use super::*;

/// Sélection de la classe de caractères masqués par [`MacroEngine::apply_quoting`].
///
/// - [`MaskSet::Punct`] : ponctuation/opérateurs seulement (`%str`/`%bquote`),
///   les déclencheurs `&`/`%` restent ACTIFS ;
/// - [`MaskSet::All`] : ponctuation ET déclencheurs `&`/`%` (`%nrstr`/`%nrbquote`/
///   `%superq`/`%q*`/`%qcmpres`) → contenu totalement inerte en aval.
#[derive(Clone, Copy)]
pub(super) enum MaskSet {
    /// Masque uniquement la ponctuation/opérateurs (`&`/`%` actifs).
    Punct,
    /// Masque aussi les déclencheurs `&`/`%`.
    All,
}

impl MaskSet {
    /// Rend le drapeau `triggers` correspondant pour `mask_special`/`mask_char`.
    fn triggers(self) -> bool {
        matches!(self, MaskSet::All)
    }
}

impl MacroEngine {
    /// Applique le quoting unifié à `text` : masque les caractères de la classe
    /// `mask` via le schéma de sentinelles partagé, puis — si `reevaluate` — ré-
    /// expanse le texte masqué (`process_impl`) afin de résoudre les `&x`/`%m`
    /// laissés actifs (comportement de `%str`). Les sentinelles produites sont
    /// bit-identiques à celles de `mask_special`/`mask_char` (réutilisés ici).
    pub(super) fn apply_quoting(
        eng: &mut MacroEngine,
        text: &str,
        mask: MaskSet,
        reevaluate: bool,
    ) -> String {
        let masked = Self::mask_special(text, mask.triggers());
        if reevaluate {
            eng.process_impl(&masked)
        } else {
            masked
        }
    }

    /// Sentinelle de base (zone privée Unicode). Chaque caractère spécial masqué
    /// est remplacé par `MASK_BASE + offset`, où `offset` est un petit index
    /// stable. La passe `unmask` finale rétablit les littéraux. Ces points de
    /// code n'apparaissent jamais dans un source SAS normal.
    pub(super) const MASK_BASE: u32 = 0xE000;

    /// Caractères masqués par `%str` (et `%nrstr`), dans l'ordre des offsets.
    /// `%str` masque la ponctuation/opérateurs pour qu'un `;` ou `,` interne
    /// soit littéral ; `&` et `%` ne sont masqués QUE par `%nrstr`.
    pub(super) const STR_MASKED: &'static [char] = &[
        ';', '+', '-', '*', '/', '<', '>', '=', '|', '~', ',', '(', ')', '\'', '"',
    ];
    /// Caractères additionnels masqués UNIQUEMENT par `%nrstr` (déclencheurs).
    pub(super) const NRSTR_EXTRA: &'static [char] = &['&', '%'];

    /// Masque un caractère vers sa sentinelle si présent dans la table donnée.
    /// Rend `Some(sentinelle)` ou `None` si le caractère n'est pas masqué.
    pub(super) fn mask_char(c: char) -> Option<char> {
        // Index global stable sur la concaténation STR_MASKED ++ NRSTR_EXTRA.
        if let Some(idx) = Self::STR_MASKED.iter().position(|&m| m == c) {
            return char::from_u32(Self::MASK_BASE + idx as u32);
        }
        if let Some(idx) = Self::NRSTR_EXTRA.iter().position(|&m| m == c) {
            return char::from_u32(Self::MASK_BASE + Self::STR_MASKED.len() as u32 + idx as u32);
        }
        None
    }

    /// Passe finale d'« unmask » : rétablit chaque sentinelle en son littéral.
    pub(super) fn unmask(s: &str) -> String {
        if !s.chars().any(|c| {
            let v = c as u32;
            (Self::MASK_BASE..Self::MASK_BASE + 0x100).contains(&v)
        }) {
            return s.to_string();
        }
        let total = Self::STR_MASKED.len() + Self::NRSTR_EXTRA.len();
        s.chars()
            .map(|c| {
                let v = c as u32;
                if v >= Self::MASK_BASE && v < Self::MASK_BASE + total as u32 {
                    let idx = (v - Self::MASK_BASE) as usize;
                    if idx < Self::STR_MASKED.len() {
                        Self::STR_MASKED[idx]
                    } else {
                        Self::NRSTR_EXTRA[idx - Self::STR_MASKED.len()]
                    }
                } else {
                    c
                }
            })
            .collect()
    }

    /// Masque TOUS les caractères « spéciaux » d'une chaîne via le schéma de
    /// sentinelles partagé avec `%str`/`%nrstr`. Si `triggers` est vrai, masque
    /// aussi `&` et `%` (déclencheurs) — sinon ils restent actifs en aval. Les
    /// caractères non listés (lettres, chiffres, blancs, `.`) passent inchangés.
    pub(super) fn mask_special(s: &str, triggers: bool) -> String {
        s.chars()
            .map(|c| {
                if (c == '&' || c == '%') && !triggers {
                    return c;
                }
                Self::mask_char(c).unwrap_or(c)
            })
            .collect()
    }
}
