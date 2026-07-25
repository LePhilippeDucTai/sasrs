use super::*;

impl MacroEngine {
    // ── M11.6 : %sysfunc ─────────────────────────────────────────────────────

    /// Parse `func(a, b, ...)` et évalue la fonction. Le contenu a déjà ses
    /// `&refs` résolus. Rend le texte du résultat ou une `MacroError`.
    pub(crate) fn eval_sysfunc(&self, content: &str) -> Result<String, MacroError> {
        let content = content.trim();
        let chars: Vec<char> = content.chars().collect();
        // Nom de fonction.
        let (name, after) = Self::read_name(&chars, 0)
            .ok_or_else(|| MacroError::new("ERROR: %SYSFUNC requires a function call"))?;
        let mut j = after;
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        // Arguments : `(...)` optionnel (fonctions sans argument : TODAY()). On
        // garde l'index APRÈS la parenthèse fermante de la fonction pour repérer
        // un éventuel `, <format>` de niveau supérieur (M35.1).
        let (arg_strings, after_call) = if chars.get(j) == Some(&'(') {
            let (args_inner, after_call) = Self::read_balanced_parens(&chars, j)
                .ok_or_else(|| MacroError::new("ERROR: unbalanced parentheses in %SYSFUNC"))?;
            (Self::split_top_level_commas(&args_inner), after_call)
        } else {
            (Vec::new(), j)
        };
        // Argument de format optionnel : `%sysfunc(func(args), format)`. Le
        // contenu restant après l'appel doit être `, <format>` (un seul token au
        // niveau supérieur). On ne consomme PAS de virgule à l'intérieur des
        // parenthèses de la fonction (elles sont équilibrées par read_balanced).
        let mut k = after_call;
        while matches!(chars.get(k), Some(c) if c.is_whitespace()) {
            k += 1;
        }
        let format: Option<String> = if chars.get(k) == Some(&',') {
            let rest: String = chars[k + 1..].iter().collect();
            let f = rest.trim().to_string();
            if f.is_empty() {
                None
            } else {
                Some(f)
            }
        } else {
            None
        };
        let upper = name.to_uppercase();
        // Typage des arguments : un argument qui parse en nombre devient
        // `Value::Num`, sinon `Value::Char` (le trim de bord est appliqué, comme
        // SAS pour les arguments macro).
        let args: Vec<crate::value::Value> = arg_strings
            .iter()
            .map(|a| {
                let t = a.trim();
                match t.parse::<f64>() {
                    Ok(n) => crate::value::Value::Num(n),
                    Err(_) => crate::value::Value::Char(t.to_string()),
                }
            })
            .collect();
        // EvalCtx minimal jetable : `Default` suffit. Les fonctions qui auraient
        // besoin du PDV/dataset n'y ont pas accès (comme %SYSFUNC sous SAS) et
        // rendent une valeur missing — comportement acceptable.
        let mut ctx = crate::datastep::eval::EvalCtx::default();
        // Délégation à la bibliothèque COMPLÈTE de fonctions DATA-step (plus de
        // liste blanche, M35.1). Fonction inconnue → None → note d'erreur propre.
        let result = crate::datastep::functions::call(&upper, &args, &mut ctx).ok_or_else(|| {
            MacroError::new(format!(
                "ERROR: Function {} not found or not supported by %SYSFUNC",
                name
            ))
        })?;
        match format {
            // Format présent : on l'applique via le MÊME chemin que PUT (module
            // formats), puis on rogne les blancs (SAS gauche-aligne / retire les
            // blancs de tête du résultat formaté de %sysfunc).
            Some(fmt) => {
                let formatted = crate::datastep::functions::call(
                    "PUT",
                    &[result, crate::value::Value::Char(fmt)],
                    &mut ctx,
                )
                .map(|v| Self::value_to_text(&v))
                .unwrap_or_default();
                Ok(formatted.trim().to_string())
            }
            None => Ok(Self::value_to_text(&result)),
        }
    }
}
