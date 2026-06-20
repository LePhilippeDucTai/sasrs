//! Inclusion de fichiers (`%include`), chargement paresseux autocall
//! (`SASAUTOS`) et écriture au log (`%put`).

use super::*;

impl MacroEngine {
    /// M19.2 — profondeur maximale d'imbrication des `%include` (garde contre
    /// les inclusions cycliques : un fichier qui s'inclut lui-même, ou un cycle
    /// A→B→A). Au-delà, l'inclusion est refusée avec une note SAS-like.
    pub(super) const MAX_INCLUDE_DEPTH: usize = 50;
}

impl MacroEngine {
    /// M19.2 — consomme un `%include 'chemin';` à partir de `i` (qui pointe sur
    /// le `%`), charge le fichier référencé, l'expanse RÉCURSIVEMENT (les
    /// `%macro` qu'il définit s'enregistrent dans l'état vivant de l'engine, et
    /// son code ouvert est émis) et splice le résultat dans `out` À LA PLACE du
    /// statement. Rend l'index APRÈS le `;` (un `\n` final est préservé pour la
    /// numérotation), ou `None` si la syntaxe ne tient pas (le `%` est alors
    /// laissé brut).
    ///
    /// # Formes reconnues
    /// - `%include 'chemin';` / `%include "chemin";` : littéral entre guillemets.
    ///   Le chemin est résolu via `include_base_dir` (relatif) ou tel quel
    ///   (absolu).
    ///
    /// # Cas d'erreur (jamais de `panic`)
    /// - profondeur d'inclusion > `MAX_INCLUDE_DEPTH` (cycle présumé) → un
    ///   commentaire de note SAS-like est émis, le statement est consommé ;
    /// - fichier illisible/absent → idem (commentaire d'erreur) ;
    /// - `%include` sans guillemets (ex. `%include fileref;` ou `*` / stdin) →
    ///   non supporté ici : un commentaire de note est émis et le statement
    ///   consommé jusqu'au `;`.
    pub(super) fn consume_include(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let mut j = i + "%include".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        // Chemin entre guillemets simples ou doubles.
        let quote = match chars.get(j) {
            Some(&q @ ('\'' | '"')) => q,
            _ => {
                // Forme non supportée (fileref nu, `*` stdin) : consommer jusqu'au
                // `;` et émettre une note plutôt que de laisser un résidu.
                let mut k = j;
                while k < chars.len() && chars[k] != ';' {
                    k += 1;
                }
                if chars.get(k) != Some(&';') {
                    return None;
                }
                out.push_str(
                    "/* %include: only quoted file paths are supported (fileref/stdin deferred) */",
                );
                return Some(Self::skip_trailing_newline(chars, k + 1, out));
            }
        };
        j += 1; // après le guillemet ouvrant
        let path_start = j;
        while j < chars.len() && chars[j] != quote {
            j += 1;
        }
        if chars.get(j) != Some(&quote) {
            return None; // guillemet non fermé
        }
        let path: String = chars[path_start..j].iter().collect();
        j += 1; // après le guillemet fermant
                // Consommer le reste jusqu'au `;` terminal (options éventuelles ignorées).
        while j < chars.len() && chars[j] != ';' {
            j += 1;
        }
        if chars.get(j) != Some(&';') {
            return None;
        }
        let resume = Self::skip_trailing_newline(chars, j + 1, out);

        // Garde contre les inclusions cycliques (profondeur max).
        if self.include_depth >= Self::MAX_INCLUDE_DEPTH {
            out.push_str(&format!(
                "/* %include nesting limit ({}) reached for '{}' */",
                Self::MAX_INCLUDE_DEPTH,
                path
            ));
            return Some(resume);
        }

        // Résolution du chemin : absolu → tel quel ; relatif → joint à la base.
        let resolved = {
            let p = std::path::PathBuf::from(&path);
            if p.is_absolute() {
                p
            } else {
                self.include_base_dir.join(p)
            }
        };
        let contents = match std::fs::read_to_string(&resolved) {
            Ok(text) => text,
            Err(e) => {
                out.push_str(&format!(
                    "/* %include: cannot read '{}': {} */",
                    path, e
                ));
                return Some(resume);
            }
        };

        // Expansion récursive du fichier inclus avec l'état VIVANT de l'engine.
        self.include_depth += 1;
        let expanded = self.process_impl(&contents);
        self.include_depth -= 1;
        out.push_str(&expanded);
        Some(resume)
    }

    /// M19.3 — consomme un `%put <texte>;` à partir de `i` (sur le `%`). Le
    /// texte va jusqu'au prochain `;` de niveau supérieur (en sautant les
    /// régions à parenthèses équilibrées des fonctions macro, pour ne pas
    /// couper sur un `;` interne). Il est résolu (`&var` + `%function`) puis
    /// écrit AU LOG via le tampon `pending_log_lines` — `%put` n'émet RIEN dans
    /// le flux de code. Rend l'index après le `;` (un `\n` final préservé).
    ///
    /// Conformément à SAS, `%put;` (sans argument) écrit une ligne vide.
    pub(super) fn consume_put(&mut self, chars: &[char], i: usize, out: &mut String) -> Option<usize> {
        let mut j = i + "%put".len();
        while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
            j += 1;
        }
        let arg_start = j;
        // Texte jusqu'au `;` de niveau 0 (parenthèses équilibrées sautées).
        let mut paren = 0i32;
        while j < chars.len() {
            let ch = chars[j];
            if ch == '(' {
                paren += 1;
            } else if ch == ')' {
                if paren > 0 {
                    paren -= 1;
                }
            } else if ch == ';' && paren == 0 {
                break;
            }
            j += 1;
        }
        if chars.get(j) != Some(&';') {
            return None; // pas de `;` terminal : abandon, on ne consomme rien.
        }
        let raw: String = chars[arg_start..j].iter().collect();
        j += 1; // après le `;`
        // Résolution immédiate (interprétation des `&var` et `%function`).
        let resolved = if raw.contains('%') {
            // Ré-expansion complète (gère `%upcase`, `%sysfunc`, etc.), puis
            // dé-masquage des sentinelles `%str`/`%nrstr`.
            Self::unmask(&self.process_impl(&raw))
        } else if raw.contains('&') {
            Self::unmask(&self.resolve_value(&raw))
        } else {
            Self::unmask(&raw)
        };
        // SAS rogne le blanc de tête laissé après `%put` ; le reste est verbatim.
        self.log_line(resolved.trim_end().to_string());
        Some(Self::skip_trailing_newline(chars, j, out))
    }

    /// M19.2 — chargement paresseux d'une macro autocall (`SASAUTOS`).
    ///
    /// Appelé à l'expansion de `%nom(...)` lorsque `nom` n'est PAS encore défini
    /// dans `self.macros`. Cherche `nom.sas` (nom en minuscules) dans chaque
    /// répertoire de `sasautos_path` (premier trouvé gagne), lit le fichier et
    /// l'expanse via `process_impl` (ce qui ENREGISTRE la `%macro` qu'il
    /// contient ; toute sortie de code ouvert du fichier est ignorée — un
    /// fichier autocall ne doit définir que la macro). La tentative est mémoïsée
    /// dans `autocall_tried` (trouvée ou non), pour ne pas relire le disque à
    /// chaque appel suivant.
    pub(super) fn try_autocall(&mut self, name: &str) {
        let key = name.to_uppercase();
        if self.autocall_tried.contains(&key) {
            return;
        }
        self.autocall_tried.insert(key);
        if self.sasautos_path.is_empty() {
            return;
        }
        let filename = format!("{}.sas", name.to_lowercase());
        // Premier fichier lisible gagne. On lit AVANT d'appeler `process_impl`
        // pour ne pas garder `self.sasautos_path` emprunté pendant l'expansion
        // (qui prend `&mut self`).
        let mut found: Option<String> = None;
        for dir in &self.sasautos_path {
            let candidate = dir.join(&filename);
            if let Ok(contents) = std::fs::read_to_string(&candidate) {
                found = Some(contents);
                break;
            }
        }
        if let Some(contents) = found {
            // Compilation : on expanse le fichier (qui enregistre la `%macro`)
            // et on jette la sortie de code ouvert.
            let _ = self.process_impl(&contents);
        }
    }
}
