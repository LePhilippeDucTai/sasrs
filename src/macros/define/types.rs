
/// Définition d'une macro capturée par `%macro name(params); <body> %mend;`.
///
/// `body` est le texte VERBATIM entre le `;` qui clôt la liste de paramètres et
/// le `%mend` correspondant. Il n'est PAS expansé à la définition ; il l'est à
/// chaque invocation, dans la portée locale créée pour cet appel.
#[derive(Clone, Debug)]
pub struct MacroDef {
    /// Nom de la macro, stocké tel quel (la recherche se fait en MAJUSCULES).
    pub name: String,
    /// Paramètres déclarés, dans l'ordre (positionnels puis mots-clés en SAS,
    /// mais on stocke l'ordre déclaré tel quel).
    pub params: Vec<MacroParam>,
    /// Corps verbatim (non expansé) de la macro.
    pub body: String,
}

/// Un paramètre formel de macro.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MacroParam {
    /// Paramètre positionnel `p` (sans valeur par défaut ; défaut = chaîne vide).
    Positional(String),
    /// Paramètre mot-clé `kw=default`.
    Keyword { name: String, default: String },
}
