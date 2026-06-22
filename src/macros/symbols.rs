//! Table des symboles macro : résolution `&var`, affectation, variables auto.

use super::*;

impl MacroEngine {
    /// Pose un symbole macro GLOBAL (sémantique `CALL SYMPUT` — M11.5) : le
    /// symbole est créé/écrasé dans la table globale, insensible casse.
    pub fn set_symbol_global(&mut self, name: &str, value: String) {
        self.table.insert(name.to_uppercase(), value);
    }

    /// Lit la valeur d'un symbole macro (pile de portées puis table globale,
    /// comme `&var`). `None` si indéfini.
    pub fn get_symbol(&self, name: &str) -> Option<String> {
        self.lookup(name)
    }

    /// Instantané (clés MAJUSCULES → valeur) de la table macro VISIBLE en
    /// open code, pour alimenter `SYMGET` (M11.5). On aplatit la pile de
    /// portées (plus interne d'abord) puis la table globale ; en open code la
    /// pile est vide, donc seule `table` contribue.
    /// Variables macro GLOBALES (table globale uniquement, hors portées
    /// locales), pour `DICTIONARY.MACROS` / `sashelp.vmacro` (M20.3). Clés en
    /// MAJUSCULES → valeur. Le classement scope GLOBAL/AUTOMATIC est laissé à
    /// l'appelant (cf. `sql::dictionary`).
    pub fn global_symbols(&self) -> std::collections::HashMap<String, String> {
        self.table.clone()
    }

    pub fn symbols_snapshot(&self) -> std::collections::HashMap<String, String> {
        let mut snap = self.table.clone();
        // La table globale est la base ; les portées locales (s'il y en a)
        // l'emportent. En open code, `scopes` est vide.
        for scope in &self.scopes {
            for (k, v) in scope {
                snap.insert(k.clone(), v.clone());
            }
        }
        snap
    }

    /// Cherche un symbole macro par nom (insensible casse) : pile de portées du
    /// plus interne au plus externe, puis table globale. Rend la valeur si
    /// trouvée.
    pub(super) fn lookup(&self, name: &str) -> Option<String> {
        let key = name.to_uppercase();
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(&key) {
                return Some(v.clone());
            }
        }
        self.table.get(&key).cloned()
    }

    /// Affecte une valeur à un symbole (sémantique `%let`) : met à jour la
    /// variable là où elle est DÉJÀ définie (pile du plus interne au plus
    /// externe, puis table) ; sinon la crée en global (`table`). Cf. la règle
    /// documentée dans l'en-tête de `MacroEngine`.
    pub(super) fn assign(&mut self, name: &str, value: String) {
        let key = name.to_uppercase();
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(&key) {
                scope.insert(key, value);
                return;
            }
        }
        self.table.insert(key, value);
    }

    /// Résout récursivement (itérativement) les `&ref` d'une valeur en
    /// utilisant la table courante. Garde de récursion `MAX_RESOLVE_ITERS`.
    pub(super) fn resolve_value(&self, value: &str) -> String {
        let mut current = value.to_string();
        for _ in 0..Self::MAX_RESOLVE_ITERS {
            if !current.contains('&') {
                break;
            }
            let next = self.resolve_refs_once(&current);
            if next == current {
                break;
            }
            current = next;
        }
        current
    }

    /// M19.3 — produit les lignes SYMBOLGEN pour un token `&...` (potentiellement
    /// indirect `&&v&i`). On résout l'indirection jusqu'à obtenir un (ou
    /// plusieurs) `&name` direct(s), puis on émet une ligne par variable
    /// effectivement consultée, façon SAS :
    /// `SYMBOLGEN:  Macro variable NAME resolves to VALUE`.
    /// Les variables indéfinies ne produisent pas de ligne (SAS warne ailleurs).
    pub(super) fn symbolgen_trace(&mut self, run: &str) {
        // Réduit l'indirection : tant qu'il reste des `&&`, on résout une passe
        // (qui transforme `&&`→`&` et substitue les `&name` directs internes).
        let mut current = run.to_string();
        for _ in 0..Self::MAX_RESOLVE_ITERS {
            if !current.contains("&&") {
                break;
            }
            let next = self.resolve_refs_once(&current);
            if next == current {
                break;
            }
            current = next;
        }
        // À ce stade `current` ne contient plus que des `&name` directs.
        let chars: Vec<char> = current.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '&' {
                if let Some((name, after)) = Self::read_name(&chars, i + 1) {
                    if let Some(v) = self.lookup(&name) {
                        self.log_line(format!(
                            "SYMBOLGEN:  Macro variable {} resolves to {}",
                            name.to_uppercase(),
                            v
                        ));
                    }
                    i = after;
                    continue;
                }
            }
            i += 1;
        }
    }

    /// Une passe de résolution des `&ref` sur une chaîne, sans réinjection.
    pub(super) fn resolve_refs_once(&self, text: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '&' {
                // `&&` -> un seul `&`.
                if chars.get(i + 1) == Some(&'&') {
                    out.push('&');
                    i += 2;
                    continue;
                }
                if let Some((name, after)) = Self::read_name(&chars, i + 1) {
                    let mut next = after;
                    // Terminateur point : consommé qu'on résolve ou non.
                    if chars.get(next) == Some(&'.') {
                        next += 1;
                    }
                    match self.lookup(&name) {
                        Some(v) => out.push_str(&v),
                        None => {
                            // Non défini : on laisse `&name` verbatim. Le
                            // point terminateur a déjà été consommé.
                            out.push('&');
                            out.push_str(&name);
                        }
                    }
                    i = next;
                    continue;
                }
            }
            out.push(c);
            i += 1;
        }
        out
    }

    // ── M11.6 : variables automatiques ──────────────────────────────────────

    /// Amorce un sous-ensemble des variables automatiques SAS dans `table`.
    /// Sous `deterministic`, valeurs FIGÉES (snapshots stables) ; sinon dérivées
    /// de l'horloge réelle. Cf. la doc de [`MacroEngine::new`].
    pub(super) fn seed_automatic_vars(&mut self, deterministic: bool) {
        // ── Variables de date/heure (6 d'origine, inchangées) ───────────────
        let mut vars: Vec<(&str, String)> = if deterministic {
            vec![
                ("SYSDATE9", "01JAN1960".to_string()),
                ("SYSDATE", "01JAN60".to_string()),
                ("SYSTIME", "00:00".to_string()),
                ("SYSDAY", "Friday".to_string()),
                ("SYSVER", "9.4".to_string()),
                ("SYSSCP", "LIN X64".to_string()),
            ]
        } else {
            use chrono::{Datelike, Local, Timelike};
            let now = Local::now();
            const MONTHS: [&str; 12] = [
                "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
            ];
            const DAYS: [&str; 7] = [
                "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday",
            ];
            let mon = MONTHS[(now.month0()) as usize];
            let day = now.day();
            let year4 = now.year();
            let year2 = (year4 % 100).abs();
            let sysdate9 = format!("{day:02}{mon}{year4:04}");
            let sysdate = format!("{day:02}{mon}{year2:02}");
            let systime = format!("{:02}:{:02}", now.hour(), now.minute());
            let sysday = DAYS[now.weekday().num_days_from_monday() as usize].to_string();
            vec![
                ("SYSDATE9", sysdate9),
                ("SYSDATE", sysdate),
                ("SYSTIME", systime),
                ("SYSDAY", sysday),
                ("SYSVER", "9.4".to_string()),
                ("SYSSCP", "LIN X64".to_string()),
            ]
        };

        // ── Status/return codes (initial values, constant in all modes) ──────
        vars.extend([
            ("SYSCC", "0".to_string()),
            ("SYSERR", "0".to_string()),
            ("SYSRC", "0".to_string()),
            ("SYSFILRC", "0".to_string()),
            ("SYSLIBRC", "0".to_string()),
            ("SQLOBS", "0".to_string()),
            ("SQLRC", "0".to_string()),
            ("SQLEXITCODE", "0".to_string()),
        ]);

        // ── Last dataset (set to _NULL_ initially; updated live after each step) ─
        vars.push(("SYSLAST", "_NULL_".to_string()));

        // ── Static environment info (constant in all modes) ──────────────────
        vars.extend([
            ("SYSSCPL", "Linux".to_string()),
            ("SYSPROCESSNAME", "DMS Process".to_string()),
            ("SYSENV", "FORE".to_string()),
            ("SYSMACRONAME", "".to_string()),
            ("SYSPARM", "".to_string()),
            ("SYSADDRSPACE", "".to_string()),
            ("SYSNCPU", "1".to_string()),
            ("SYSSITE", "0".to_string()),
        ]);

        // ── User/host: frozen in deterministic mode, live otherwise ──────────
        if deterministic {
            vars.extend([
                ("SYSUSERID", "sasuser".to_string()),
                ("SYSHOSTNAME", "localhost".to_string()),
                ("SYSJOBID", "1".to_string()),
                ("SYSPROCESSID", "0".to_string()),
            ]);
        } else {
            let user = std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_else(|_| "sasuser".to_string());
            let hostname = std::env::var("HOSTNAME")
                .unwrap_or_else(|_| "localhost".to_string());
            let pid = std::process::id();
            vars.extend([
                ("SYSUSERID", user),
                ("SYSHOSTNAME", hostname),
                ("SYSJOBID", "1".to_string()),
                ("SYSPROCESSID", pid.to_string()),
            ]);
        }

        for (k, v) in vars {
            self.table.insert(k.to_string(), v);
        }
    }

    /// Expose a thin public setter for automatic macro variables (used by the
    /// executor to keep &SYSLAST in sync with `session.last_dataset`).
    pub fn set_automatic(&mut self, name: &str, value: String) {
        self.set_symbol_global(name, value);
    }
}
