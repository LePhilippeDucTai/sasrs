use super::*;

/// Découpeur de segments bruts d'open code (M11.5, feature `macros`).
///
/// `run_program` (sous `macros`) traite le source ORIGINAL segment par
/// segment : chaque segment est expansé par `expand_open_code` avec l'état
/// VIVANT de l'engine, puis lexé/parsé/exécuté, AVANT de passer au suivant.
/// C'est ce qui rend `CALL SYMPUT` visible dans le segment suivant (le drain
/// du symput a lieu à la fin de l'étape, donc avant l'expansion du segment
/// d'après).
///
/// # Règle de segmentation (volontairement GROSSIÈRE mais correcte)
/// On découpe le source brut en unités de haut niveau terminées par un
/// `run;`/`quit;` de niveau supérieur. Concrètement, le scanner avance
/// caractère par caractère en :
/// - ignorant l'intérieur des chaînes `'...'` / `"..."` (les `;` y sont
///   inertes) ;
/// - suivant la profondeur `%macro … %mend` (un `;` à l'intérieur d'une
///   définition de macro NE termine PAS un segment) ;
/// - coupant un segment juste APRÈS le `;` qui suit un mot-clé `run` ou
///   `quit` de niveau supérieur (fin d'étape DATA/PROC).
///
/// Le reliquat après le dernier `run;` (open code final : `%put`, `%let`,
/// etc.) forme le dernier segment. Les instructions d'open code situées
/// AVANT une étape sont donc regroupées avec cette étape dans un même
/// segment — sans incidence : l'expansion est gauche→droite et l'état macro
/// persiste de toute façon entre segments.
///
/// Renvoie des PLAGES d'octets `[start, end)` dans le source d'origine, afin
/// que l'executor puisse à la fois (a) ré-expanser le texte brut du segment
/// et (b) écho­ter les lignes ORIGINALES correspondantes (numérotation
/// préservée).
pub struct RawSegmenter<'a> {
    chars: Vec<char>,
    /// Décalages OCTETS cumulés, `byte_offset[i]` = offset du i-ème char.
    byte_offsets: Vec<usize>,
    total_bytes: usize,
    pos: usize,
    _src: std::marker::PhantomData<&'a str>,
}

impl<'a> RawSegmenter<'a> {
    pub fn new(src: &'a str) -> Self {
        let chars: Vec<char> = src.chars().collect();
        let mut byte_offsets = Vec::with_capacity(chars.len() + 1);
        let mut off = 0usize;
        for c in &chars {
            byte_offsets.push(off);
            off += c.len_utf8();
        }
        byte_offsets.push(off);
        RawSegmenter {
            chars,
            byte_offsets,
            total_bytes: src.len(),
            pos: 0,
            _src: std::marker::PhantomData,
        }
    }

    /// Vrai si `chars[i..]` commence par le mot-clé `kw` (insensible casse)
    /// PRÉCÉDÉ d'une frontière de mot (début, blanc, `;`) et SUIVI d'un
    /// non-identifiant. Sert à reconnaître `run`/`quit` de niveau supérieur.
    fn word_at(chars: &[char], i: usize, kw: &str) -> bool {
        // Frontière gauche.
        if i > 0 {
            let p = chars[i - 1];
            if p.is_ascii_alphanumeric() || p == '_' {
                return false;
            }
        }
        let kwc: Vec<char> = kw.chars().collect();
        for (k, &kc) in kwc.iter().enumerate() {
            match chars.get(i + k) {
                Some(c) if c.to_ascii_lowercase() == kc => {}
                _ => return false,
            }
        }
        !matches!(chars.get(i + kwc.len()), Some(c) if c.is_ascii_alphanumeric() || *c == '_')
    }

    /// Renvoie la prochaine plage d'octets `[start, end)`, ou `None` à la fin.
    pub fn next_segment(&mut self) -> Option<(usize, usize)> {
        if self.pos >= self.chars.len() {
            return None;
        }
        let start_char = self.pos;
        let mut i = self.pos;
        let mut macro_depth = 0usize;
        // Mot-clé run/quit vu et en attente de son `;` terminal.
        let mut pending_boundary = false;
        while i < self.chars.len() {
            let c = self.chars[i];
            // Chaînes : sauter jusqu'au guillemet fermant (mêmes guillemets).
            if c == '\'' || c == '"' {
                let quote = c;
                i += 1;
                while i < self.chars.len() && self.chars[i] != quote {
                    i += 1;
                }
                if i < self.chars.len() {
                    i += 1; // guillemet fermant
                }
                continue;
            }
            // Profondeur %macro / %mend (un `;` interne ne coupe pas).
            if c == '%' {
                if MacroEngine::matches_kw(&self.chars, i, "macro") {
                    macro_depth += 1;
                    i += "%macro".len();
                    continue;
                }
                if MacroEngine::matches_kw(&self.chars, i, "mend") {
                    macro_depth = macro_depth.saturating_sub(1);
                    i += "%mend".len();
                    continue;
                }
            }
            if macro_depth == 0 {
                if Self::word_at(&self.chars, i, "run") {
                    pending_boundary = true;
                    i += "run".len();
                    continue;
                }
                if Self::word_at(&self.chars, i, "quit") {
                    pending_boundary = true;
                    i += "quit".len();
                    continue;
                }
                if c == ';' && pending_boundary {
                    // Fin de segment juste après ce `;`.
                    i += 1;
                    self.pos = i;
                    return Some((self.byte_offsets[start_char], self.byte_offsets[i]));
                }
            }
            i += 1;
        }
        // Reliquat : tout le reste forme le dernier segment.
        self.pos = self.chars.len();
        Some((self.byte_offsets[start_char], self.total_bytes))
    }
}
