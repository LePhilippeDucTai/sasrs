use super::*;

impl MacroEngine {
    // ── M11.6 : %str / %nrstr (quoting par sentinelles) ─────────────────────
    //
    // Les primitives de masquage (constantes `MASK_BASE`/`STR_MASKED`/
    // `NRSTR_EXTRA` et helpers `mask_char`/`unmask`/`mask_special`) vivent dans
    // le sous-module `quoting`.

    /// Consomme `%str ( ... )` (si `!nrstr`) ou `%nrstr ( ... )`. Masque les
    /// caractères spéciaux du contenu (pour `%str`, `&`/`%` restent ACTIFS et
    /// sont donc résolus ; pour `%nrstr` ils sont AUSSI masqués → inertes). Pour
    /// `%str`, on ré-expanse le contenu masqué afin de résoudre les `&x`/`%m`
    /// éventuels ; pour `%nrstr`, on émet le contenu masqué tel quel. Rend
    /// l'index après la `)`, ou `None` si la parenthèse n'est pas trouvée.
    pub(crate) fn consume_quote(
        &mut self,
        chars: &[char],
        i: usize,
        kw: &str,
        nrstr: bool,
        out: &mut String,
    ) -> Option<usize> {
        let mut j = i + 1 + kw.len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) != Some(&'(') {
            return None;
        }
        let (inner, after) = Self::read_balanced_parens(chars, j)?;
        // `%nrstr` masque AUSSI `&`/`%` (contenu inerte, émis tel quel) ; `%str`
        // ne masque que la ponctuation et ré-expanse pour résoudre les `&x`/`%m`
        // résiduels (déclencheurs restés actifs).
        let mask = if nrstr { MaskSet::All } else { MaskSet::Punct };
        out.push_str(&Self::apply_quoting(self, &inner, mask, !nrstr));
        Some(after)
    }

    // ── M19.1 : fonctions macro différées ───────────────────────────────────

    /// Consomme `%unquote ( text )`. C'est l'INVERSE des fonctions de quoting
    /// (`%str`/`%nrstr`/`%bquote`/`%superq`/`%q*`) : il « dé-masque » le texte et
    /// RÉ-ACTIVE la résolution des déclencheurs `&`/`%` qui avaient été rendus
    /// inertes par le schéma de sentinelles.
    ///
    /// Interaction avec le schéma de sentinelles (point délicat) : les fonctions
    /// de quoting remplacent `&`/`%`/ponctuation par des sentinelles `MASK_BASE+k`.
    /// `%unquote` procède en trois temps :
    ///   1. résoudre les `&refs` ENCORE actifs de l'argument (texte non masqué) ;
    ///   2. `unmask` → rétablir les littéraux d'origine, ce qui ressuscite tout
    ///      `&`/`%` précédemment masqué ;
    ///   3. ré-`process_impl` le texte dé-masqué → les `&`/`%` ressuscités sont
    ///      maintenant résolus comme des déclencheurs normaux.
    ///
    /// La passe `unmask` finale de `expand_open_code` ne fait alors plus rien sur
    /// ce fragment (déjà dé-masqué). Rend l'index après la `)`.
    pub(crate) fn consume_unquote(
        &mut self,
        chars: &[char],
        i: usize,
        out: &mut String,
    ) -> Option<usize> {
        let mut j = i + 1 + "unquote".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) != Some(&'(') {
            return None;
        }
        let (inner, after) = Self::read_balanced_parens(chars, j)?;
        // 1. expanser l'argument tel quel : tout `%str`/`%nrstr`/`%q*` imbriqué
        //    s'exécute et POSE ses sentinelles (déclencheurs `&`/`%` masqués) ;
        // 2. `unmask` → rétablit les littéraux, ce qui RESSUSCITE `&`/`%` ;
        // 3. ré-`process_impl` → ces déclencheurs ressuscités sont maintenant
        //    résolus comme des déclencheurs normaux. La passe `unmask` finale de
        //    `expand_open_code` ne fait plus rien sur ce fragment.
        let expanded = self.process_impl(&inner);
        let unmasked = Self::unmask(&expanded);
        let reexpanded = self.process_impl(&unmasked);
        out.push_str(&reexpanded);
        Some(after)
    }

    /// Consomme `%sysevalf ( expr [, conv] )` : évaluation FLOTTANTE de `expr`
    /// (contrairement à `%eval` qui est entier seulement). Le résultat brut est
    /// un `f64` ; un éventuel deuxième argument `conv` le convertit :
    /// - `BOOLEAN` → `1` si non nul (et non missing), `0` sinon ;
    /// - `CEIL`    → plafond, formaté en entier ;
    /// - `FLOOR`   → plancher, formaté en entier ;
    /// - `INTEGER` → troncature vers zéro, formaté en entier ;
    /// - absent    → le flottant formaté (entier sans décimales si exact).
    ///
    /// `&refs`/macros imbriquées dans `expr` sont résolues d'abord. Erreur de
    /// syntaxe → note d'erreur (pas de panic). Rend l'index après la `)`.
    pub(crate) fn consume_sysevalf(
        &mut self,
        chars: &[char],
        i: usize,
        out: &mut String,
    ) -> Option<usize> {
        let mut j = i + 1 + "sysevalf".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) != Some(&'(') {
            return None;
        }
        let (inner, after) = Self::read_balanced_parens(chars, j)?;
        // L'argument peut contenir des &refs/macros : résoudre AVANT de découper
        // les virgules (les nombres ne contiennent pas de virgule de niveau sup.).
        let resolved = self.resolve_value(&inner);
        let expanded = self.process_impl(&resolved);
        let parts = Self::split_top_level_commas(&expanded);
        let expr = parts.first().map(String::as_str).unwrap_or("").trim();
        let conv = parts.get(1).map(|s| s.trim().to_ascii_uppercase());
        match Self::eval_float(expr) {
            Ok(v) => out.push_str(&Self::format_sysevalf(v, conv.as_deref())),
            Err(e) => Self::emit_error(out, &e),
        }
        Some(after)
    }

    // ── M12.2 : quoting étendu (%superq, %bquote, %nrbquote) ─────────────────

    /// Consomme `%superq ( name )`. Prend un NOM de variable (pas `&name`),
    /// lit sa valeur SANS résoudre aucun `&`/`%` qu'elle contient, et masque
    /// TOUT (y compris `&`/`%`) afin que le résultat soit littéral et inerte en
    /// aval — l'outil idéal pour des valeurs contenant des `&`/`%` parasites.
    /// L'argument peut lui-même être un `&ref` désignant le nom (SAS résout
    /// l'argument en un nom). Variable indéfinie → chaîne vide (SAS émet un
    /// WARNING ; on se contente d'émettre vide). Rend l'index après la `)`.
    pub(crate) fn consume_superq(
        &mut self,
        chars: &[char],
        i: usize,
        out: &mut String,
    ) -> Option<usize> {
        let mut j = i + 1 + "superq".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) != Some(&'(') {
            return None;
        }
        let (inner, after) = Self::read_balanced_parens(chars, j)?;
        // L'argument désigne un nom : on résout d'éventuels `&ref` puis on rogne
        // les blancs et un éventuel `&` de tête (SAS accepte `%superq(&x)` =
        // nom dans x, comme `%superq(x)`). On ne touche PAS à la valeur lue.
        let name_arg = self.resolve_value(&inner);
        let name = name_arg.trim().trim_start_matches('&').trim();
        match self.lookup(name) {
            Some(v) => out.push_str(&Self::apply_quoting(self, &v, MaskSet::All, false)),
            None => { /* indéfini → vide (SAS warne) */ }
        }
        Some(after)
    }

    /// Consomme `%bquote ( text )` (si `!nr`) ou `%nrbquote ( text )`. Résout
    /// d'abord les `&`/`%` du texte (expansion normale), PUIS masque le
    /// résultat pour le rendre littéral en aval :
    /// - `%bquote` masque la ponctuation/opérateurs (`; , ( ) ' " + - * / < >
    ///   = | ~`) mais laisse `&`/`%` ACTIFS (ils ont déjà été résolus ; un `&`
    ///   résiduel non défini reste tel quel) ;
    /// - `%nrbquote` masque EN PLUS `&`/`%` du résultat (empêche toute
    ///   résolution ultérieure).
    ///
    /// Les quotes/parenthèses NON APPARIÉES de l'entrée ne posent pas de
    /// problème : on ne fait pas d'analyse appariée du contenu — `read_balanced_parens`
    /// borne sur la `)` de `%bquote(...)` et tout `'`/`(` interne est traité
    /// comme un caractère ordinaire (puis masqué). Rend l'index après la `)`.
    pub(crate) fn consume_bquote(
        &mut self,
        chars: &[char],
        i: usize,
        kw: &str,
        nr: bool,
        out: &mut String,
    ) -> Option<usize> {
        let mut j = i + 1 + kw.len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        if chars.get(j) != Some(&'(') {
            return None;
        }
        let (inner, after) = Self::read_balanced_parens(chars, j)?;
        // Expansion normale du texte (résout `&x`/`%m`), puis masquage du
        // résultat. `%nrbquote` masque aussi `&`/`%`.
        let expanded = self.process_impl(&inner);
        let mask = if nr { MaskSet::All } else { MaskSet::Punct };
        out.push_str(&Self::apply_quoting(self, &expanded, mask, false));
        Some(after)
    }
}
