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
//!
//! ## M39.1 — sidecar de catalogue par libref
//!
//! `FormatCatalog` (dé)sérialise en JSON via [`FormatCatalog::load_sidecar`] /
//! [`FormatCatalog::save_sidecar`], un fichier **par libref**, nommé
//! `formats.sascat.json`, posé à la RACINE du répertoire de la bibliothèque
//! (pas par table — un catalogue de formats est une ressource de bibliothèque,
//! pas de dataset, à l'image d'un `.sas7bcat` réel). Schéma (clés upcase) :
//! ```json
//! { "user": { "GRADEF": { "is_char": false, "ranges": [...], "other": null } },
//!   "user_informats": { "$SIZE": { ... } },
//!   "user_pictures":  { "DOLLARPIC": { ... } } }
//! ```
//! Écrit uniquement si le catalogue n'est pas vide (jamais de fichier pour
//! WORK, jamais de fichier vide) — même garde que le sidecar `.sasmeta.json`
//! de `dataset.rs`.
//!
//! Ordre de résolution retenu ici : WORK (`session.format_catalog`, le chemin
//! historique, jamais touché par le disque) **d'abord**, puis les
//! bibliothèques chargées par LIBNAME dans leur ordre d'assignation. En
//! pratique (voir `executor/global/libname.rs` et `procs/format/mod.rs`) les
//! formats d'un libref chargé sont fusionnés dans `session.format_catalog`
//! SANS écraser une clé déjà présente ([`FormatCatalog::merge_missing_from`]) :
//! toute définition WORK explicite (avant ou après le LIBNAME, car
//! `PROC FORMAT` sans LIB= écrase toujours sans condition) l'emporte sur la
//! valeur chargée depuis un libref.
//!
//! ## M39.3 — `FMTSEARCH=` : résolution ordonnée explicite
//!
//! Le mécanisme ci-dessus (implicite, "WORK puis ordre d'assignation") reste
//! le chemin par défaut, INCHANGÉ, tant que `OPTIONS FMTSEARCH=` n'a jamais
//! été posée dans la session (`session.options.fmtsearch.is_empty()`) — c'est
//! ce qui garantit l'octet-identité des jalons précédents. Dès que
//! `FMTSEARCH=` est posée (même vide n'est jamais reposé après coup dans ce
//! build, voir `formats::search`), `session.format_catalog` cesse d'être
//! alimenté par la fusion implicite ci-dessus et devient un **catalogue
//! recalculé en entier** à chaque changement pertinent (`OPTIONS FMTSEARCH=`,
//! `LIBNAME`, `PROC FORMAT`) par [`search::rebuild_format_catalog`], à partir
//! de deux sources qui, elles, ne sont JAMAIS mutées directement par la
//! fusion legacy :
//! - `session.format_catalog_own_work` — les définitions de CETTE session
//!   ciblant WORK (voir sa doc dans `session.rs`) ;
//! - `session.libref_format_catalogs[libref]` — déjà, de longue date, le
//!   catalogue propre à CE libref (chargé par sidecar + accumulé par
//!   `PROC FORMAT LIBRARY=libref`), jamais pollué par les autres librefs.
//!
//! `search::resolve_search_order` traduit la liste `FMTSEARCH=(a b …)` en un
//! ordre effectif : WORK et LIBRARY sont recherchés EN TÊTE par défaut, sauf
//! présence explicite dans la liste (auquel cas ils gardent leur position
//! écrite). "LIBRARY" n'est un libref réel dans ce build que si l'utilisateur
//! l'a lui-même assigné via `LIBNAME LIBRARY ...` — ce build n'a pas de
//! catalogue permanent LIBRARY préassigné comme le fait SAS ; une entrée
//! "LIBRARY" sans libref assigné est silencieusement sans effet (déviation
//! documentée, pas un oubli).
//!
//! Déviation assumée : repasser `FMTSEARCH=` à une liste VIDE après l'avoir
//! posée non-vide ne restaure PAS la fusion implicite par défaut — le dernier
//! ordre explicite reste actif (voir `search::rebuild_format_catalog`). Non
//! requis par l'oracle M39.3 (qui ne teste que la bascule entre deux ordres
//! non vides) ; un `OPTIONS FMTSEARCH=` jamais posé reste, lui, byte-identique
//! à avant M39.3.
//!
//! ## M39.2 — `CNTLOUT=`/`CNTLIN=`
//!
//! `PROC FORMAT` peut aussi échanger son catalogue avec un DATASET (pas un
//! sidecar JSON) : `CNTLOUT=ds` dépose une observation par plage de chaque
//! format VALUE/INVALUE dans `ds` (colonnes FMTNAME/START/END/LABEL/TYPE +
//! SEXCL/EEXCL/HLO) ; `CNTLIN=ds` fait l'inverse. Voir `procs::format::cntl`
//! pour le détail des colonnes et le round-trip. Les formats PICTURE ne sont
//! PAS couverts (leurs directives PREFIX/MULT/FILL n'ont pas de colonne dans
//! ce jeu minimal) — déferral documenté, pas un oubli.
//!
//! ## M43.1 — `MIN=`/`MAX=`/`DEFAULT=`/`FUZZ=` sur `VALUE`
//!
//! `PROC FORMAT VALUE` accepte quatre options FORMAT-LEVEL supplémentaires
//! (portées par `userdef::UserFormat`, voir sa doc de module pour le détail) :
//! `MIN=`/`MAX=` bornent la largeur de sortie effective, `DEFAULT=` fixe la
//! largeur par défaut (sinon calculée : longueur du plus long label),
//! `FUZZ=` assouplit les comparaisons de bornes numériques. `format()`
//! ci-dessous délègue le calcul de largeur à `UserFormat::effective_width`,
//! qui a un chemin rapide garantissant l'octet-identité de tout `VALUE` qui
//! ne pose aucune de ces options (l'écrasante majorité des fixtures
//! existantes). Round-trip via `CNTLOUT=`/`CNTLIN=` : voir `procs::format::cntl`.

#![allow(unused_variables, dead_code)]

pub mod builtin;
pub mod search;
pub mod userdef;

use crate::error::{Result, SasError};
use crate::value::{Value, format_best};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct FormatSpec {
    /// Nom sans largeur ("DATE", "$CHAR", "" pour w.d), upcase.
    pub name: String,
    pub w: Option<u16>,
    pub d: Option<u16>,
}

impl FormatSpec {
    /// Parse a SAS format token. Handles forms like:
    ///   "DATE9."  -> name="DATE", w=9, d=None
    ///   "DATE9"   -> name="DATE", w=9, d=None  (trailing dot optional)
    ///   "8.2"     -> name="",     w=8, d=2
    ///   "8."      -> name="",     w=8, d=None
    ///   "$CHAR10."-> name="$CHAR",w=10,d=None
    ///   "$10."    -> name="$",    w=10,d=None
    ///   "COMMA12.2"->name="COMMA",w=12,d=2
    ///   "BEST12." -> name="BEST", w=12,d=None
    pub fn parse(s: &str) -> Option<FormatSpec> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }

        // Strip trailing dot(s) — but be careful: "8.2" has a dot in the
        // middle, so we only strip a trailing dot AFTER we know there's no
        // decimal part following it.
        // Strategy: work character by character.
        let chars: Vec<char> = s.chars().collect();
        let mut pos = 0;

        // 1. Collect the name: leading '$' is allowed, then alphabetic chars.
        if pos < chars.len() && chars[pos] == '$' {
            pos += 1;
        }
        while pos < chars.len() && chars[pos].is_ascii_alphabetic() {
            pos += 1;
        }
        let name: String = s[..pos].to_uppercase();

        // 2. Collect optional width digits.
        let w_start = pos;
        while pos < chars.len() && chars[pos].is_ascii_digit() {
            pos += 1;
        }
        let w: Option<u16> = if pos > w_start {
            s[w_start..pos].parse().ok()
        } else {
            None
        };

        // 3. Optional '.' then optional decimal digits.
        let d: Option<u16> = if pos < chars.len() && chars[pos] == '.' {
            pos += 1; // consume the dot
            let d_start = pos;
            while pos < chars.len() && chars[pos].is_ascii_digit() {
                pos += 1;
            }
            if pos > d_start {
                s[d_start..pos].parse().ok()
            } else {
                None
            }
        } else {
            None
        };

        // Ignore any remaining trailing characters (e.g. stray dot).

        // Must have at least a name or a width.
        if name.is_empty() && w.is_none() {
            return None;
        }

        Some(FormatSpec { name, w, d })
    }
}

#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct FormatCatalog {
    user: HashMap<String, userdef::UserFormat>,
    /// User-defined informats (PROC FORMAT INVALUE, M18.2) keyed by upcased
    /// name (with `$` prefix for char informats, e.g. `$SIZE`).
    user_informats: HashMap<String, userdef::UserInformat>,
    /// User-defined PICTURE formats (PROC FORMAT PICTURE, M18.3) keyed by
    /// upcased name. Picture formats apply to NUMERIC values only.
    user_pictures: HashMap<String, userdef::UserPicture>,
}

impl FormatCatalog {
    /// Register a user-defined format (from PROC FORMAT VALUE) keyed by upcased name.
    pub fn define(&mut self, name: &str, fmt: userdef::UserFormat) {
        self.user.insert(name.to_uppercase(), fmt);
    }

    /// Register a user-defined informat (from PROC FORMAT INVALUE, M18.2).
    pub fn define_informat(&mut self, name: &str, inf: userdef::UserInformat) {
        self.user_informats.insert(name.to_uppercase(), inf);
    }

    /// Register a user-defined PICTURE format (PROC FORMAT PICTURE, M18.3).
    pub fn define_picture(&mut self, name: &str, pic: userdef::UserPicture) {
        self.user_pictures.insert(name.to_uppercase(), pic);
    }

    /// Return a sorted list of user-defined format names (M21.1 — PROC CATALOG CONTENTS).
    pub fn user_format_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.user.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }

    /// M39.2 — read-only iteration over VALUE-format entries (catalog key,
    /// which includes the `$` prefix for character formats) for `CNTLOUT=`.
    pub fn user_formats(&self) -> impl Iterator<Item = (&str, &userdef::UserFormat)> {
        self.user.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Lookup a single user-defined `VALUE` format by name (case-insensitive
    /// — mirrors the lookup in [`FormatCatalog::format`]'s user-format
    /// branch). M43.1 — used at DATA step COMPILE time by
    /// `datastep::helpers::put_width` to size a `put(x, fmt.)` PDV variable
    /// via [`userdef::UserFormat::effective_width`] instead of trusting only
    /// the literal width digits in the format token, when the name already
    /// resolves to a known format at that point in the program.
    pub fn user_format(&self, name: &str) -> Option<&userdef::UserFormat> {
        self.user.get(&name.to_uppercase())
    }

    /// M39.2 — read-only iteration over INVALUE entries, symmetric to
    /// [`FormatCatalog::user_formats`].
    pub fn user_informats(&self) -> impl Iterator<Item = (&str, &userdef::UserInformat)> {
        self.user_informats.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// M39.3 — read-only iteration over PICTURE entries, symmetric to
    /// [`FormatCatalog::user_formats`]. Used by `PROC FORMAT FMTLIB` to list
    /// picture formats (not covered by `CNTLOUT=`/`CNTLIN=`, see
    /// [`FormatCatalog::user_picture_count`]).
    pub fn user_pictures(&self) -> impl Iterator<Item = (&str, &userdef::UserPicture)> {
        self.user_pictures.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// M39.2 — number of PICTURE formats in this catalog. `CNTLOUT=`/`CNTLIN=`
    /// do not round-trip PICTURE formats in this build (their PREFIX/MULT/FILL
    /// directives have no column in the minimal CNTLOUT column set implemented
    /// here — see `procs::format::cntl` module doc); this accessor lets the
    /// CNTLOUT writer emit an honest NOTE about what it left out instead of
    /// silently dropping them.
    pub fn user_picture_count(&self) -> usize {
        self.user_pictures.len()
    }

    /// True when no VALUE/INVALUE/PICTURE has EVER been registered. Used to
    /// decide whether the M39.1 sidecar is worth writing at all (WORK never
    /// writes one; a `PROC FORMAT LIB=x;` with no sub-statement writes none
    /// either).
    pub fn is_empty(&self) -> bool {
        self.user.is_empty() && self.user_informats.is_empty() && self.user_pictures.is_empty()
    }

    /// M39.1 — merge entries from `other` into `self`, WITHOUT overwriting a
    /// key already present in `self`. Used exclusively when a `LIBNAME`
    /// statement loads a libref's persisted sidecar catalog into the live
    /// session catalog: whatever is already defined (WORK, or an earlier
    /// LIBNAME'd library) keeps priority — see the module doc for the full
    /// resolution-order rationale.
    pub fn merge_missing_from(&mut self, other: &FormatCatalog) {
        for (k, v) in &other.user {
            self.user.entry(k.clone()).or_insert_with(|| v.clone());
        }
        for (k, v) in &other.user_informats {
            self.user_informats
                .entry(k.clone())
                .or_insert_with(|| v.clone());
        }
        for (k, v) in &other.user_pictures {
            self.user_pictures
                .entry(k.clone())
                .or_insert_with(|| v.clone());
        }
    }

    /// M39.1 — filename of the per-libref sidecar catalog. One file at the
    /// ROOT of the libref's directory (a format catalog is a LIBRARY-level
    /// resource, not a per-table one).
    pub const SIDECAR_FILE: &'static str = "formats.sascat.json";

    /// Load the sidecar catalog from `dir` (the libref's physical directory),
    /// if it exists and parses. Any I/O or parse error is swallowed and
    /// treated as "no persisted catalog" — mirrors `dataset.rs::read_sidecar`.
    pub fn load_sidecar(dir: &Path) -> Option<FormatCatalog> {
        let path = dir.join(Self::SIDECAR_FILE);
        let data = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// Write the catalog as `dir/formats.sascat.json`. No-op (does not even
    /// touch the file) when the catalog is empty — mirrors the "no meta, no
    /// file" rule of `dataset.rs::write_sidecar`, which is what keeps the
    /// in-memory-only WORK path byte-identical.
    pub fn save_sidecar(&self, dir: &Path) -> Result<()> {
        if self.is_empty() {
            return Ok(());
        }
        let path = dir.join(Self::SIDECAR_FILE);
        let json = serde_json::to_string(self)
            .map_err(|e| SasError::runtime(format!("failed to serialize format catalog: {e}")))?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// PUT: value → formatted string (SAS-justified, width spec.w).
    ///
    /// Resolution order:
    ///   1. User format (spec.name upcased) — consults the HashMap
    ///   2. builtin::format_builtin
    ///   3. Fallback: Char → left-justified/truncated to w;
    ///      Missing → right-justified missing char to w;
    ///      Num → format_best right-justified to w (default 12).
    pub fn format(&self, v: &Value, spec: &FormatSpec) -> String {
        // Intercept numeric missings first — applies before ANY numeric format.
        if let Value::Missing(k) = v {
            let ch = k.display();
            let w = spec.w.unwrap_or(1) as usize;
            return right_justify(&ch, w);
        }

        // 1. Try user VALUE format.
        let uname = spec.name.to_uppercase();
        if let Some(uf) = self.user.get(&uname)
            && let Some(label) = uf.lookup(v)
        {
            let s = label.to_string();
            // M43.1 — effective width = spec.w, else DEFAULT=/computed max
            // label length, clamped into [MIN=, MAX=]. `effective_width`
            // has a fast path that reproduces the pre-M43.1 calculation
            // exactly when MIN=/MAX=/DEFAULT= are all unset (the common
            // case) — see its doc.
            return match uf.effective_width(spec.w) {
                Some(w) => right_justify(&s, w),
                None => s,
            };
        }

        // 1b. Try user PICTURE format (M18.3) — numeric only, before builtins.
        if let Some(pic) = self.user_pictures.get(&uname)
            && let Some(rendered) = pic.render(v)
        {
            return match spec.w {
                Some(w) => right_justify(&rendered, w as usize),
                None => rendered,
            };
        }

        // 2. Try builtin.
        if let Some(s) = builtin::format_builtin(v, spec) {
            return s;
        }

        // 3. Fallback.
        match v {
            Value::Char(s) => {
                match spec.w {
                    None => s.clone(),
                    Some(w) => {
                        let w = w as usize;
                        // Left-justify: truncate or pad with spaces.
                        let mut out = s.clone();
                        out.truncate(w);
                        while out.len() < w {
                            out.push(' ');
                        }
                        out
                    }
                }
            }
            Value::Num(n) => {
                let w = spec.w.unwrap_or(12) as usize;
                let s = format_best(*n, w);
                right_justify(&s, w)
            }
            Value::Missing(_) => unreachable!("handled above"),
        }
    }

    /// INPUT: string → value.
    ///
    /// Resolution order:
    ///   1. User informat (M18.2) — checked BEFORE builtins so user can shadow.
    ///   2. builtin::informat_builtin
    ///   3. Fallback: trim; empty/"." → missing; parse as f64 → Num; else Char.
    pub fn informat(&self, s: &str, spec: &FormatSpec) -> Value {
        // 1. Try user informat (M18.2) — shadows builtins.
        let uname = spec.name.to_uppercase();
        if let Some(ui) = self.user_informats.get(&uname) {
            if let Some(v) = ui.lookup(s) {
                return v;
            }
            // User informat matched by name but no range matched — return missing.
            return Value::missing();
        }

        // 2. Try builtin.
        if let Some(v) = builtin::informat_builtin(s, spec) {
            return v;
        }

        // 3. Fallback.
        let trimmed = s.trim();
        if trimmed.is_empty() || trimmed == "." {
            return Value::missing();
        }
        if let Ok(f) = trimmed.parse::<f64>() {
            return Value::Num(f);
        }
        Value::Char(trimmed.to_string())
    }
}

/// Right-justify `s` in a field of width `w`, truncating if longer.
pub(crate) fn right_justify(s: &str, w: usize) -> String {
    if w == 0 {
        return String::new();
    }
    if s.len() >= w {
        // Truncate from the right (keep rightmost w chars, SAS overflow rule).
        // Actually SAS fills with * on overflow; but for missing/name we truncate.
        s[s.len().saturating_sub(w)..].to_string()
    } else {
        format!("{:>width$}", s, width = w)
    }
}

#[cfg(test)]
mod tests;
