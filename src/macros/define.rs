//! Définition (`%macro`/`%mend`) et invocation (`%name(...)`) de macros, plus
//! les déclarations de portée (`%local`/`%global`) et `%call execute`.

use super::*;

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

impl MacroEngine {
    /// Profondeur maximale d'invocation de macro (garde anti-récursion).
    pub(super) const MAX_MACRO_DEPTH: usize = 100;
}

impl MacroEngine {
    /// Consomme une définition `%macro name[(params)] ; <body> %mend [name];`.
    /// Enregistre la définition et n'émet RIEN. Rend l'index après le `;` du
    /// `%mend` (un `\n` final juste après est préservé pour la numérotation),
    /// ou `None` si la syntaxe ne tient pas (le `%` est alors laissé brut).
    pub(super) fn consume_macro_def(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let mut j = i + "%macro".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        let (name, after_name) = Self::read_name(chars, j)?;
        j = after_name;
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        // Liste de paramètres optionnelle.
        let mut params = Vec::new();
        if chars.get(j) == Some(&'(') {
            let (parsed, after_paren) = Self::parse_param_list(chars, j)?;
            params = parsed;
            j = after_paren;
            while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
                j += 1;
            }
        }
        // `;` qui clôt l'en-tête.
        if chars.get(j) != Some(&';') {
            return None;
        }
        j += 1;
        // Corps verbatim jusqu'au `%mend` (au niveau courant). On scanne en
        // suivant les `%macro`/`%mend` imbriqués pour équilibrer.
        let body_start = j;
        let mut depth = 1usize;
        let mut body_end = None;
        while j < chars.len() {
            if chars[j] == '%' {
                if Self::matches_kw(chars, j, "macro") {
                    depth += 1;
                    j += "%macro".len();
                    continue;
                }
                if Self::matches_kw(chars, j, "mend") {
                    depth -= 1;
                    if depth == 0 {
                        body_end = Some(j);
                        break;
                    }
                    j += "%mend".len();
                    continue;
                }
            }
            j += 1;
        }
        let body_end = body_end?; // pas de `%mend` : abandon.
        // Corps capturé verbatim, puis trim des blancs de bord. SAS conserve
        // les blancs internes du corps ; on rogne uniquement les bords pour que
        // `%macro p; x=1; %mend; %p` n'introduise pas d'espaces parasites.
        let body: String = chars[body_start..body_end]
            .iter()
            .collect::<String>()
            .trim()
            .to_string();
        // Avancer après `%mend [name] ;`.
        j = body_end + "%mend".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        // Nom optionnel après %mend.
        if let Some((_, after)) = Self::read_name(chars, j) {
            j = after;
        }
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) == Some(&';') {
            j += 1;
        }
        // Consommer les blancs en ligne suivants ; préserver un `\n` final.
        while matches!(chars.get(j), Some(c) if *c == ' ' || *c == '\t') {
            j += 1;
        }
        if chars.get(j) == Some(&'\n') {
            out.push('\n');
            j += 1;
        }
        self.macros.insert(
            name.to_uppercase(),
            MacroDef {
                name,
                params,
                body,
            },
        );
        Some(j)
    }

    /// Parse une liste de paramètres `(p1, p2, kw=def, ...)` à partir de `(`.
    /// Rend `(params, index après `)`)`. `None` si pas de `)` fermant.
    pub(super) fn parse_param_list(chars: &[char], lparen: usize) -> Option<(Vec<MacroParam>, usize)> {
        let mut j = lparen + 1;
        let mut params = Vec::new();
        loop {
            while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
                j += 1;
            }
            if chars.get(j) == Some(&')') {
                return Some((params, j + 1));
            }
            // Nom du paramètre.
            let (name, after) = Self::read_name(chars, j)?;
            j = after;
            while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
                j += 1;
            }
            if chars.get(j) == Some(&'=') {
                // Mot-clé avec défaut jusqu'à `,` ou `)`.
                j += 1;
                let start = j;
                while j < chars.len() && chars[j] != ',' && chars[j] != ')' {
                    j += 1;
                }
                let default: String = chars[start..j].iter().collect();
                params.push(MacroParam::Keyword {
                    name,
                    default: default.trim().to_string(),
                });
            } else {
                params.push(MacroParam::Positional(name));
            }
            while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
                j += 1;
            }
            match chars.get(j) {
                Some(&',') => {
                    j += 1;
                    continue;
                }
                Some(&')') => return Some((params, j + 1)),
                _ => return None,
            }
        }
    }

    /// Consomme `%local v1 v2 ...;` (si `is_local`) ou `%global v1 v2 ...;`.
    /// Crée les variables (vides) dans la portée appropriée. Rend l'index après
    /// le `;`, ou `None` si pas de `;`.
    pub(super) fn consume_scope_decl(
        &mut self,
        chars: &[char],
        i: usize,
        is_local: bool,
        out: &mut String,
    ) -> Option<usize> {
        let kwlen = if is_local { "%local".len() } else { "%global".len() };
        let mut j = i + kwlen;
        let mut names = Vec::new();
        loop {
            while matches!(chars.get(j), Some(c) if c.is_whitespace() || *c == ',') {
                j += 1;
            }
            if chars.get(j) == Some(&';') {
                j += 1;
                break;
            }
            match Self::read_name(chars, j) {
                Some((name, after)) => {
                    names.push(name);
                    j = after;
                }
                None => return None, // syntaxe inattendue : laisser brut.
            }
        }
        // Comme `%let` : absorber les blancs en ligne suivants, préserver un
        // `\n` final pour la numérotation des lignes.
        while matches!(chars.get(j), Some(c) if *c == ' ' || *c == '\t') {
            j += 1;
        }
        if chars.get(j) == Some(&'\n') {
            out.push('\n');
            j += 1;
        }
        for name in names {
            let key = name.to_uppercase();
            if is_local {
                if let Some(scope) = self.scopes.last_mut() {
                    scope.entry(key).or_default();
                } else {
                    // `%local` hors d'une macro : SAS l'ignore ~ ; on retombe
                    // sur global pour rester simple.
                    self.table.entry(key).or_default();
                }
            } else {
                self.table.entry(key).or_default();
            }
        }
        Some(j)
    }

    /// Expanse une invocation `%name[(args)]`. `name_start` pointe sur le nom
    /// (après le `%`), `after_name` sur le premier caractère après le nom.
    /// Lie les arguments, empile une portée, ré-expanse le corps, insère le
    /// résultat dans `out`, dépile. Rend l'index de reprise du scan.
    pub(super) fn expand_invocation(
        &mut self,
        chars: &[char],
        _name_start: usize,
        name: &str,
        after_name: usize,
        out: &mut String,
    ) -> usize {
        // Parse des arguments éventuels.
        let mut j = after_name;
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        let (pos_args, kw_args, resume) = if chars.get(j) == Some(&'(') {
            match Self::parse_arg_list(chars, j) {
                Some(v) => v,
                None => {
                    // Parenthèse non fermée : émettre `%name` verbatim, ne rien
                    // consommer de plus.
                    out.push('%');
                    out.push_str(name);
                    return after_name;
                }
            }
        } else {
            // Appel sans parenthèses : consommer un `;` immédiat optionnel
            // (SAS termine l'appel macro par `;` qui n'est pas réémis).
            let mut r = after_name;
            if chars.get(r) == Some(&';') {
                r += 1;
            }
            (Vec::new(), Vec::new(), r)
        };

        // Garde de récursion.
        if self.depth >= Self::MAX_MACRO_DEPTH {
            out.push_str(&format!(
                "/* macro recursion limit ({}) reached for %{} */",
                Self::MAX_MACRO_DEPTH,
                name
            ));
            return resume;
        }

        let def = self.macros.get(&name.to_uppercase()).cloned();
        let def = match def {
            Some(d) => d,
            None => {
                // Ne devrait pas arriver (vérifié par l'appelant) ; sûreté.
                out.push('%');
                out.push_str(name);
                return after_name;
            }
        };

        // Les valeurs d'arguments sont résolues dans la portée APPELANTE (SAS
        // évalue les arguments au moment de l'appel) avant la liaison.
        let pos_args: Vec<String> = pos_args.iter().map(|a| self.resolve_value(a)).collect();
        let kw_args: Vec<(String, String)> = kw_args
            .iter()
            .map(|(k, v)| (k.clone(), self.resolve_value(v)))
            .collect();

        // Liaison des paramètres -> portée locale.
        let scope = Self::bind_params(&def.params, &pos_args, &kw_args);
        let label = name.to_uppercase();
        // M19.3 — MLOGIC : décision d'entrée de macro.
        if self.mlogic {
            self.log_line(format!("MLOGIC({label}):  Beginning execution."));
            // Écho de la valeur reçue par chaque paramètre (façon SAS).
            for param in &def.params {
                let pname = match param {
                    MacroParam::Positional(n) => n,
                    MacroParam::Keyword { name, .. } => name,
                };
                let val = scope.get(&pname.to_uppercase()).cloned().unwrap_or_default();
                self.log_line(format!(
                    "MLOGIC({label}):  Parameter {} has value {}",
                    pname.to_uppercase(),
                    val
                ));
            }
        }
        self.scopes.push(scope);
        self.macro_stack.push(label.clone());
        self.depth += 1;
        // M35.4 — budget de sauts `%goto` propre à CETTE invocation (save/restore
        // pour les appels imbriqués). Un `%goto` posé par une macro plus interne
        // ne doit pas franchir la frontière de macro : on capture l'état avant le
        // corps et on le restaure après.
        let saved_goto_budget = self.goto_budget;
        let saved_goto_requested = self.goto_requested.take();
        self.goto_budget = Self::MAX_GOTO_JUMPS;
        let mut expanded = self.process_impl(&def.body);
        // Un `%goto` non résolu remonté jusqu'ici = étiquette introuvable dans CE
        // corps : NOTE propre (et on ne propage pas hors de la macro).
        if let Some(missing) = self.goto_requested.take() {
            expanded.push_str(&format!(
                "/* NOTE: %GOTO target label %{}: not found; statement ignored */",
                missing.to_lowercase()
            ));
        }
        self.goto_budget = saved_goto_budget;
        self.goto_requested = saved_goto_requested;
        // M35.4 — `%return` est local à CE corps : on réinitialise le drapeau
        // après l'expansion afin qu'il ne fuie ni vers l'appelant ni vers l'open
        // code (garantie de ré-entrance : un 2ᵉ appel de la même macro se comporte
        // à l'identique). `%abort`, lui, se PROPAGE (drapeau laissé tel quel) :
        // l'expansion de l'appelant l'observera en tête de sa boucle.
        self.return_requested = false;
        self.depth -= 1;
        self.macro_stack.pop();
        self.scopes.pop();
        // M19.3 — MPRINT : écho du code produit par la macro, ligne à ligne.
        // Chaque ligne NON VIDE (après trim) du texte expansé est écho­tée
        // avec le préfixe `MPRINT(nom):`.
        if self.mprint {
            for line in expanded.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    self.log_line(format!("MPRINT({label}):   {trimmed}"));
                }
            }
        }
        // M19.3 — MLOGIC : décision de sortie de macro.
        if self.mlogic {
            self.log_line(format!("MLOGIC({label}):  Ending execution."));
        }
        out.push_str(&expanded);
        resume
    }

    /// Parse une liste d'arguments d'appel `(a, b, key=val, ...)` à partir de
    /// `(`. Rend `(positionnels, mots-clés, index après `)`)`. Les valeurs sont
    /// prises telles quelles (trim des bords) jusqu'au `,`/`)` de même niveau ;
    /// les parenthèses imbriquées sont équilibrées.
    pub(super) fn parse_arg_list(
        chars: &[char],
        lparen: usize,
    ) -> Option<(Vec<String>, Vec<(String, String)>, usize)> {
        let mut j = lparen + 1;
        let mut positional = Vec::new();
        let mut keyword = Vec::new();
        // Cas liste vide `()`.
        {
            let mut k = j;
            while matches!(chars.get(k), Some(c) if c.is_whitespace()) {
                k += 1;
            }
            if chars.get(k) == Some(&')') {
                return Some((positional, keyword, k + 1));
            }
        }
        loop {
            // Détecter `name=` : un identifiant suivi de `=` (avant tout `,`/`(`).
            let seg_start = j;
            let mut paren = 0i32;
            let mut eq_pos: Option<usize> = None;
            let mut k = j;
            while k < chars.len() {
                let c = chars[k];
                if c == '(' {
                    paren += 1;
                } else if c == ')' {
                    if paren == 0 {
                        break;
                    }
                    paren -= 1;
                } else if c == ',' && paren == 0 {
                    break;
                } else if c == '=' && paren == 0 && eq_pos.is_none() {
                    eq_pos = Some(k);
                }
                k += 1;
            }
            let seg_end = k; // pointe sur `,` ou `)` ou fin.
            // Déterminer si c'est `key=value` : la partie avant `=` doit être un
            // identifiant pur (trim).
            let mut is_keyword = false;
            if let Some(eq) = eq_pos {
                let key: String = chars[seg_start..eq].iter().collect();
                let key_trim = key.trim();
                if !key_trim.is_empty()
                    && key_trim
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_alphabetic() || c == '_')
                        .unwrap_or(false)
                    && key_trim.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    is_keyword = true;
                    let val: String = chars[eq + 1..seg_end].iter().collect();
                    keyword.push((key_trim.to_string(), val.trim().to_string()));
                }
            }
            if !is_keyword {
                let val: String = chars[seg_start..seg_end].iter().collect();
                positional.push(val.trim().to_string());
            }
            match chars.get(seg_end) {
                Some(&',') => {
                    j = seg_end + 1;
                    continue;
                }
                Some(&')') => return Some((positional, keyword, seg_end + 1)),
                _ => return None, // parenthèse non fermée.
            }
        }
    }

    /// Lie les paramètres formels aux arguments fournis et rend la portée
    /// locale (toutes les variables des paramètres y sont présentes).
    ///
    /// Règles (documentées) :
    /// - Les arguments positionnels remplissent les paramètres dans l'ordre de
    ///   déclaration (positionnels comme mots-clés peuvent recevoir une valeur
    ///   positionnelle, fidèle à SAS où l'ordre prime).
    /// - Les `clé=valeur` écrasent ensuite le paramètre nommé correspondant.
    /// - Paramètres non fournis : `Keyword` prend son défaut, `Positional`
    ///   prend la chaîne vide.
    /// - Trop d'arguments positionnels : les excédentaires sont IGNORÉS
    ///   (SAS émet une erreur ; on choisit la tolérance — documenté).
    /// - `clé=valeur` pour une clé inconnue : IGNORÉ (SAS erreur ; toléré ici).
    pub(super) fn bind_params(
        params: &[MacroParam],
        pos_args: &[String],
        kw_args: &[(String, String)],
    ) -> std::collections::HashMap<String, String> {
        let mut scope = std::collections::HashMap::new();
        // Valeurs par défaut.
        for p in params {
            match p {
                MacroParam::Positional(n) => {
                    scope.insert(n.to_uppercase(), String::new());
                }
                MacroParam::Keyword { name, default } => {
                    scope.insert(name.to_uppercase(), default.clone());
                }
            }
        }
        // Positionnels dans l'ordre déclaré.
        for (p, arg) in params.iter().zip(pos_args.iter()) {
            let key = match p {
                MacroParam::Positional(n) => n.to_uppercase(),
                MacroParam::Keyword { name, .. } => name.to_uppercase(),
            };
            scope.insert(key, arg.clone());
        }
        // Mots-clés : écrasent le paramètre nommé s'il existe.
        for (k, v) in kw_args {
            let key = k.to_uppercase();
            if scope.contains_key(&key) {
                scope.insert(key, v.clone());
            }
            // sinon : clé inconnue, ignorée.
        }
        scope
    }

    /// M19.3 — consomme un `%call <routine>(args);` à partir de `i` (sur le
    /// `%`). Seul `%call execute(text)` est interprété : le texte (résolu) est
    /// mis en file dans `pending_call_execute` pour exécution APRÈS le segment
    /// courant, comme le `CALL EXECUTE` côté DATA step. Les autres routines
    /// sont consommées sans effet (note SAS-like). N'émet RIEN dans le flux.
    /// Rend l'index après le `;` (un `\n` final préservé), ou `None` si la
    /// syntaxe ne tient pas.
    pub(super) fn consume_macro_call(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let mut j = i + "%call".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        let (routine, after_name) = Self::read_name(chars, j)?;
        j = after_name;
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) != Some(&'(') {
            return None;
        }
        let (inner, after_paren) = Self::read_balanced_parens(chars, j)?;
        j = after_paren;
        // Consommer un `;` terminal optionnel.
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) == Some(&';') {
            j += 1;
        }
        if routine.eq_ignore_ascii_case("execute") {
            // L'argument est résolu (macro + symboles) puis mis en file.
            let code = if inner.contains('%') {
                Self::unmask(&self.process_impl(&inner))
            } else if inner.contains('&') {
                Self::unmask(&self.resolve_value(&inner))
            } else {
                Self::unmask(&inner)
            };
            self.pending_call_execute.push(code);
        } else {
            out.push_str(&format!(
                "/* %call {}: only EXECUTE is supported in macro code */",
                routine
            ));
        }
        Some(Self::skip_trailing_newline(chars, j, out))
    }
}
