//! Compilation de l'étape DATA : AST → `StepProgram` exécutable.
//!
//! # Plan du fichier — voir PLAN.md  (difficulté : ÉLEVÉE — sémantique SAS)
//!
//! SAS exécute une étape DATA en deux phases ; ce module est la phase de
//! COMPILATION. Une passe sur l'AST construit :
//!
//! ## 1. Le PDV (Program Data Vector)
//! - Variables dans l'ORDRE DE PREMIÈRE RÉFÉRENCE textuelle (cet ordre
//!   définit l'ordre des colonnes en sortie !).
//! - `set lib.a` : lire le dataset (via `session.libs`, libref par défaut
//!   WORK) — ses variables entrent dans le PDV avec type/longueur/format
//!   de `VarMeta`, marquées `from_input=true` (elles ne sont PAS remises
//!   à missing à chaque itération). Logger les notes de coercition
//!   parquet (`LogWriter::forward`).
//! - Cible d'assignation : si absente du PDV, créer avec le type INFÉRÉ
//!   de l'expression (voir §3). Variable seulement RÉFÉRENCÉE (jamais
//!   assignée ni lue d'un input) : créer numérique + NOTE
//!   "Variable x is uninitialized." à l'exécution de la 1re itération.
//!
//! ## 2. Les sorties
//! Pour chaque dataset de `ast.outputs` : appliquer KEEP/DROP (statements
//! M1 ; options de dataset M2) → `kept_slots` = indices PDV dans l'ordre
//! PDV. KEEP et DROP simultanés : DROP gagne sur l'intersection (SAS
//! émet WARNING). Variables KEEP inexistantes → ERROR à la compilation
//! ("The variable x in the DROP, KEEP, or RENAME list has never been
//! referenced.").
//!
//! ## 3. Inférence de type d'une expression (compile-time, comme SAS)
//! - littéral num / opération arithmétique / comparaison / logique → Num
//! - littéral chaîne → Char(longueur du littéral)
//! - `Var` → type/longueur de la variable au PDV
//! - `||` → Char(somme des longueurs des opérandes)
//! - `Call` : table par fonction (upcase/lowcase/trim/strip/left →
//!   Char(longueur arg), substr → Char(longueur arg1), cat* → 200,
//!   put → largeur du format ; défaut → Num)
//! - La PREMIÈRE assignation fige type et longueur (redéfinir → la
//!   longueur d'origine reste, SAS tronque silencieusement).
//!
//! ## 4. has_explicit_output
//! Si AU MOINS UN `output;` apparaît dans l'étape, l'output implicite de
//! fin d'itération est désactivé (règle SAS).
//!
//! Erreur de compilation → l'exécuteur loggue ERROR + NOTE "The SAS
//! System stopped processing this step because of errors." et n'exécute
//! pas (mais la session continue).
//!
//! ## Choix d'implémentation
//! - Ordre de première référence d'une assignation : la CIBLE entre au PDV
//!   avant les variables de son expression (ordre textuel gauche→droite).
//! - Opérande numérique de `||` : contribue 12 à la longueur inférée
//!   (conversion implicite BEST12. comme SAS).
//! - `put(x, fmt)` : largeur = chiffres finaux du nom de format si
//!   disponibles, sinon 200 (le parser M1 ne sait pas encore produire un
//!   littéral de format ; best-effort documenté).
//! - Un seul SET par étape en M1 ; le second → erreur "not yet implemented".

pub mod eval;
pub mod exec;
pub mod functions;
pub mod pdv;

use crate::ast::{BinaryOp, DataStepAst, DatasetSpec, DsStmt, Expr};
use crate::error::{Result, SasError};
use crate::missing::num_to_value;
use crate::session::Session;
use crate::value::{Value, VarType};
use pdv::{Pdv, PdvVar};
use std::collections::{HashMap, HashSet};

/// Données d'entrée matérialisées (M2 : un seul SET).
pub struct InputData {
    /// "WORK.B" pour les NOTEs du log.
    pub display: String,
    /// Colonnes décodées en `Value` (via `missing::num_to_value`), une
    /// seule passe de downcast par colonne — jamais de get_row. Seules les
    /// colonnes retenues par KEEP=/DROP= sont présentes ; une colonne
    /// renommée par RENAME= est au PDV sous son NOUVEAU nom.
    pub columns: Vec<Vec<Value>>,
    /// Slot PDV de chaque colonne (parallèle à `columns`).
    pub var_slots: Vec<usize>,
    pub n_rows: usize,
    /// `WHERE=` du SET : évalué à l'EXÉCUTION après chargement de chaque
    /// ligne dans le PDV ; une ligne qui échoue est sautée SANS exécuter le
    /// reste de l'itération et ne compte PAS dans les observations lues
    /// (comme la NOTE SAS "There were N observations read"). NB : comme
    /// l'évaluation se fait sur le PDV, un WHERE= combiné à RENAME=
    /// référence les NOUVEAUX noms (divergence documentée — SAS applique
    /// WHERE= avant RENAME= en entrée).
    pub where_: Option<Expr>,
}

/// Une sortie : où écrire et quels slots du PDV.
pub struct OutputSpec {
    pub libref: String,
    pub table: String,
    /// "WORK.A" pour les NOTEs.
    pub display: String,
    /// Slots PDV conservés, dans l'ordre PDV (= ordre des colonnes).
    /// Combinaison (intersection) des statements KEEP/DROP et des options
    /// de dataset KEEP=/DROP= de CETTE sortie.
    pub kept_slots: Vec<usize>,
    /// Nom d'écriture de chaque slot conservé (parallèle à `kept_slots`) :
    /// le nom PDV, ou le nouveau nom si RENAME= s'applique.
    pub out_names: Vec<String>,
}

pub struct StepProgram {
    pub pdv: Pdv,
    pub stmts: Vec<crate::ast::DsStmt>,
    pub input: Option<InputData>,
    pub outputs: Vec<OutputSpec>,
    pub has_explicit_output: bool,
    /// Noms (casse de première référence, ordre PDV) des variables jamais
    /// assignées ni lues d'un input : l'exécuteur émet la NOTE
    /// "Variable x is uninitialized." à la première itération.
    pub uninitialized: Vec<String>,
    /// Valeurs initiales (slot, valeur) issues de RETAIN avec init et des
    /// sum statements (0) : l'exécuteur les applique via `pdv.set` AVANT la
    /// première itération (la troncature char s'applique donc normalement).
    /// Appliquées dans l'ordre — une entrée ultérieure pour le même slot
    /// gagne (cas `n + 1; retain n 100;` : le RETAIN l'emporte).
    pub initial_values: Vec<(usize, Value)>,
    /// Arrays 1-D : nom UPPERCASE → slots PDV des éléments, dans l'ordre de
    /// déclaration. Passé tel quel à l'EvalCtx par l'exécuteur.
    pub arrays: HashMap<String, Vec<usize>>,
}

pub fn compile(ast: &DataStepAst, session: &mut Session) -> Result<StepProgram> {
    let mut c = Compiler {
        pdv: Pdv::new(),
        session,
        input: None,
        keeps: Vec::new(),
        drops: Vec::new(),
        output_displays: ast.outputs.iter().map(|s| s.display()).collect(),
        assigned: HashSet::new(),
        has_explicit_output: false,
        retain_all: false,
        retain_pending: Vec::new(),
        retained_slots: HashSet::new(),
        initial_values: Vec::new(),
        arrays: HashMap::new(),
    };
    for stmt in &ast.stmts {
        c.walk_stmt(stmt)?;
    }

    // RETAIN sans valeur initiale — SIMPLIFICATION M2 ASSUMÉE : en vrai
    // SAS, `retain x;` ne fige PAS le type — la variable le prend à sa
    // prochaine référence. Pour approcher ça sans bouleverser la passe
    // unique, le statement n'a fait que mémoriser le nom ; ICI (fin de
    // compilation) on applique le flag `retained` à la variable, qui doit
    // alors exister (créée par une autre référence). Sinon on la crée Num
    // + uninitialized — elle arrive donc en FIN d'ordre PDV (divergence
    // mineure assumée par rapport à l'ordre de première référence SAS).
    let pending = std::mem::take(&mut c.retain_pending);
    for name in &pending {
        let slot = match c.pdv.slot(name) {
            Some(slot) => slot,
            None => c.add_var(name, VarType::Num, 8),
        };
        c.retained_slots.insert(slot);
    }
    // `retain;` seul — SIMPLIFICATION M2 : retient TOUT le PDV (en vrai
    // SAS, seulement ce qui est connu au point du statement).
    if c.retain_all {
        c.retained_slots.extend(0..c.pdv.vars().len());
    }
    // Le PDV ne permet pas de modifier une variable existante (première
    // référence fige tout, et `pdv.rs` n'expose pas de mutateur) : on le
    // reconstruit à l'identique en appliquant les flags `retained`. Les
    // slots sont préservés (même ordre d'insertion) et aucune valeur n'a
    // encore été posée à la compilation.
    if !c.retained_slots.is_empty() {
        c.pdv = rebuild_with_retained(&c.pdv, &c.retained_slots);
    }

    let outputs = c.resolve_outputs(&ast.outputs)?;
    let uninitialized = c
        .pdv
        .vars()
        .iter()
        .filter(|v| !v.from_input && !c.assigned.contains(&v.name.to_uppercase()))
        .map(|v| v.name.clone())
        .collect();
    Ok(StepProgram {
        pdv: c.pdv,
        stmts: ast.stmts.clone(),
        input: c.input,
        outputs,
        has_explicit_output: c.has_explicit_output,
        uninitialized,
        initial_values: c.initial_values,
        arrays: c.arrays,
    })
}

/// Reconstruit le PDV en marquant `retained` les slots donnés (les autres
/// attributs sont copiés tels quels ; les indices de slots sont stables).
fn rebuild_with_retained(pdv: &Pdv, retained: &HashSet<usize>) -> Pdv {
    let mut rebuilt = Pdv::new();
    for (i, v) in pdv.vars().iter().enumerate() {
        let slot = rebuilt.add_var(PdvVar {
            name: v.name.clone(),
            ty: v.ty,
            length: v.length,
            retained: v.retained || retained.contains(&i),
            from_input: v.from_input,
            format: v.format.clone(),
        });
        debug_assert_eq!(slot, i, "rebuild must preserve slot indices");
    }
    rebuilt
}

struct Compiler<'a> {
    pdv: Pdv,
    session: &'a mut Session,
    input: Option<InputData>,
    keeps: Vec<String>,
    drops: Vec<String>,
    /// Displays ("WORK.A") des sorties du statement DATA, pour valider les
    /// OUTPUT ciblés.
    output_displays: Vec<String>,
    /// Noms (uppercase) ayant au moins une assignation dans l'étape.
    assigned: HashSet<String>,
    has_explicit_output: bool,
    /// `retain;` sans liste rencontré : tout le PDV sera retenu.
    retain_all: bool,
    /// Noms d'un RETAIN SANS init : flag appliqué en fin de compilation.
    retain_pending: Vec<String>,
    /// Slots à marquer `retained` en fin de compilation (RETAIN avec init,
    /// sum statements, RETAIN sans init résolus).
    retained_slots: HashSet<usize>,
    /// Valeurs initiales (slot, valeur) appliquées avant la 1re itération.
    initial_values: Vec<(usize, Value)>,
    /// Arrays déclarés : nom UPPERCASE → slots PDV des éléments.
    arrays: HashMap<String, Vec<usize>>,
}

impl Compiler<'_> {
    fn walk_stmt(&mut self, stmt: &DsStmt) -> Result<()> {
        match stmt {
            DsStmt::Set(r) => self.compile_set(r),
            DsStmt::Assign { var, expr } => {
                // Un nom d'array n'est pas une variable : `arr = e;` est
                // une référence illégale, pas la création d'une variable.
                if self.arrays.contains_key(&var.to_uppercase()) {
                    return Err(SasError::runtime(format!(
                        "Illegal reference to the array {var}."
                    )));
                }
                // La cible entre au PDV en premier (ordre textuel), avec le
                // type inféré AVANT création des variables de l'expression
                // (les inconnues comptent comme Num, cohérent avec SAS).
                let (ty, length) = self.infer(expr);
                self.add_var(var, ty, length);
                self.assigned.insert(var.to_uppercase());
                self.walk_expr(expr)?;
                Ok(())
            }
            DsStmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.walk_expr(cond)?;
                self.walk_stmt(then_branch)?;
                if let Some(e) = else_branch {
                    self.walk_stmt(e)?;
                }
                Ok(())
            }
            DsStmt::SubsettingIf(cond) => self.walk_expr(cond),
            DsStmt::Block(stmts) => {
                for s in stmts {
                    self.walk_stmt(s)?;
                }
                Ok(())
            }
            DsStmt::DoLoop {
                index,
                to,
                by,
                while_,
                until,
                body,
            } => {
                // L'index entre au PDV au point du DO (ordre de première
                // référence) : Num 8, NON retenu, et il compte comme
                // assigné (pas de NOTE "uninitialized"). Puis les bornes
                // et conditions en ordre textuel, puis le corps.
                if let Some((name, from)) = index {
                    self.add_var(name, VarType::Num, 8);
                    self.assigned.insert(name.to_uppercase());
                    self.walk_expr(from)?;
                }
                for e in [to, by, while_, until].into_iter().flatten() {
                    self.walk_expr(e)?;
                }
                for s in body {
                    self.walk_stmt(s)?;
                }
                Ok(())
            }
            // DELETE : purement exécutif, rien à compiler.
            DsStmt::Delete => Ok(()),
            DsStmt::Output(targets) => {
                // `has_explicit_output` dès qu'UN output (ciblé ou non)
                // apparaît. Chaque cible doit être une sortie déclarée du
                // statement DATA (comparaison par display "WORK.A").
                self.has_explicit_output = true;
                for t in targets {
                    let disp = t.display();
                    if !self.output_displays.contains(&disp) {
                        return Err(SasError::runtime(format!(
                            "Output dataset {disp} is not in the DATA statement output list."
                        )));
                    }
                }
                Ok(())
            }
            DsStmt::Keep(names) => {
                self.keeps.extend(names.iter().cloned());
                Ok(())
            }
            DsStmt::Drop(names) => {
                self.drops.extend(names.iter().cloned());
                Ok(())
            }
            DsStmt::Stop => Ok(()),
            DsStmt::Retain(items) => {
                if items.is_empty() {
                    // `retain;` seul : tout le PDV (cf. fin de compile()).
                    self.retain_all = true;
                    return Ok(());
                }
                for (name, init) in items {
                    match init {
                        // AVEC init : la variable entre au PDV ICI (ordre de
                        // première référence), type/longueur du littéral, et
                        // sa valeur initiale part dans `initial_values`. Elle
                        // compte comme initialisée (pas de NOTE
                        // "uninitialized" — comme SAS).
                        Some(expr) => {
                            let (ty, length, value) = retain_literal(expr)?;
                            let slot = self.add_var(name, ty, length);
                            self.retained_slots.insert(slot);
                            self.assigned.insert(name.to_uppercase());
                            self.initial_values.push((slot, value));
                        }
                        // SANS init : ne crée PAS la variable (le type sera
                        // figé par sa prochaine référence) — voir compile().
                        None => self.retain_pending.push(name.clone()),
                    }
                }
                Ok(())
            }
            DsStmt::Sum { var, expr } => {
                // `var + expr;` : var entre au PDV (Num, 8), retenue, valeur
                // initiale 0 — SAUF si un RETAIN avec init a déjà posé une
                // valeur pour ce slot (le RETAIN gagne, comme SAS). La cible
                // entre avant les variables de l'expression (ordre textuel).
                let slot = self.add_var(var, VarType::Num, 8);
                self.retained_slots.insert(slot);
                self.assigned.insert(var.to_uppercase());
                if !self.initial_values.iter().any(|(s, _)| *s == slot) {
                    self.initial_values.push((slot, Value::Num(0.0)));
                }
                self.walk_expr(expr)?;
                Ok(())
            }
            DsStmt::Array {
                name,
                size,
                char_len,
                vars,
            } => self.compile_array(name, *size, *char_len, vars),
            DsStmt::AssignIndexed { array, index, expr } => {
                let upper = array.to_uppercase();
                let Some(slots) = self.arrays.get(&upper) else {
                    return Err(SasError::runtime(format!(
                        "Undeclared array referenced: {array}."
                    )));
                };
                // Tous les éléments sont potentiellement assignés via
                // l'indice : pas de NOTE "uninitialized" pour eux.
                for slot in slots.clone() {
                    let n = self.pdv.vars()[slot].name.to_uppercase();
                    self.assigned.insert(n);
                }
                self.walk_expr(index)?;
                self.walk_expr(expr)?;
                Ok(())
            }
            DsStmt::Length(items) => {
                for (name, spec) in items {
                    // Plages SAS : char 1..=32767, num 3..=8.
                    let (lo, hi) = if spec.char { (1, 32767) } else { (3, 8) };
                    if spec.len < lo || spec.len > hi {
                        return Err(SasError::runtime(format!(
                            "The length {} specified for the variable {} is out of range ({}-{}).",
                            spec.len, name, lo, hi
                        )));
                    }
                    match self.pdv.slot(name) {
                        // LENGTH précède la première référence : crée la
                        // variable avec cette longueur. Pour une numérique,
                        // la longueur (3..=8) est une simple MÉTADONNÉE en
                        // M2 — le stockage reste f64 sur 8 octets.
                        None => {
                            let ty = if spec.char { VarType::Char } else { VarType::Num };
                            self.add_var(name, ty, spec.len);
                        }
                        // Déjà au PDV : la longueur est figée. SAS n'émet le
                        // WARNING que pour les variables CHAR dont la
                        // longueur demandée diffère ; num : silencieux.
                        Some(slot) => {
                            let v = &self.pdv.vars()[slot];
                            if v.ty == VarType::Char && spec.char && v.length != spec.len {
                                let name = v.name.clone();
                                self.session.log.warning(&format!(
                                    "Length of character variable {name} has already been set."
                                ));
                            }
                        }
                    }
                }
                Ok(())
            }
        }
    }

    /// Crée les variables simplement référencées (Num par défaut), en ordre
    /// textuel gauche→droite. Les noms d'array ne créent JAMAIS de variable
    /// au PDV : `Expr::Index` ne walke que son indice (le nom doit être un
    /// array déclaré), `dim(arr)` ne crée pas `arr`, et un nom d'array nu
    /// (`Expr::Var`) est une référence illégale.
    fn walk_expr(&mut self, expr: &Expr) -> Result<()> {
        match expr {
            Expr::Num(_) | Expr::Str(_) | Expr::Missing(_) => Ok(()),
            Expr::Var(name) => {
                let upper = name.to_uppercase();
                // Variables automatiques (_N_, _ERROR_) : servies par
                // l'évaluateur depuis les champs dédiés du PDV — elles ne
                // doivent JAMAIS créer de slot (sinon elles deviendraient
                // des colonnes de sortie + NOTE "uninitialized" parasite).
                if upper == "_N_" || upper == "_ERROR_" {
                    return Ok(());
                }
                if self.arrays.contains_key(&upper) {
                    return Err(SasError::runtime(format!(
                        "Illegal reference to the array {name}."
                    )));
                }
                self.add_var(name, VarType::Num, 8);
                Ok(())
            }
            Expr::Unary { expr, .. } => self.walk_expr(expr),
            Expr::Binary { left, right, .. } => {
                self.walk_expr(left)?;
                self.walk_expr(right)
            }
            Expr::In { expr, list } => {
                self.walk_expr(expr)?;
                for e in list {
                    self.walk_expr(e)?;
                }
                Ok(())
            }
            Expr::Index { name, index } => {
                if !self.arrays.contains_key(&name.to_uppercase()) {
                    return Err(SasError::runtime(format!(
                        "Undeclared array referenced: {name}."
                    )));
                }
                self.walk_expr(index)
            }
            Expr::Call { name, args } => {
                // `dim(arr)` : ne crée pas de variable pour le nom d'array.
                if name.eq_ignore_ascii_case("dim")
                    && args.len() == 1
                    && let Expr::Var(n) | Expr::Index { name: n, .. } = &args[0]
                    && self.arrays.contains_key(&n.to_uppercase())
                {
                    // `dim(a{i})` : l'indice reste walké.
                    if let Expr::Index { index, .. } = &args[0] {
                        self.walk_expr(index)?;
                    }
                    return Ok(());
                }
                for a in args {
                    self.walk_expr(a)?;
                }
                Ok(())
            }
        }
    }

    /// `add_var` PDV : la première référence fige tout (le PDV ignore les
    /// ajouts suivants du même nom).
    fn add_var(&mut self, name: &str, ty: VarType, length: usize) -> usize {
        self.pdv.add_var(PdvVar {
            name: name.to_string(),
            ty,
            length,
            retained: false,
            from_input: false,
            format: None,
        })
    }

    /// Déclare un array 1-D : les éléments entrent au PDV ICI (ordre de
    /// première référence) ; `vars` vide → éléments auto-nommés name1..N ;
    /// `size` None (`{*}`) → taille déduite de la liste ; `char_len` →
    /// éléments caractère de cette longueur. Le registre `arrays` associe
    /// le nom UPPERCASE aux slots.
    fn compile_array(
        &mut self,
        name: &str,
        size: Option<usize>,
        char_len: Option<usize>,
        vars: &[String],
    ) -> Result<()> {
        let upper = name.to_uppercase();
        if self.arrays.contains_key(&upper) {
            return Err(SasError::runtime(format!(
                "An array has already been defined with the name {name}."
            )));
        }
        let names: Vec<String> = if vars.is_empty() {
            // Éléments auto-nommés name1..nameN — il faut une taille.
            let Some(n) = size else {
                return Err(SasError::runtime(format!(
                    "The array {name} has been defined with zero elements."
                )));
            };
            (1..=n).map(|i| format!("{name}{i}")).collect()
        } else {
            if let Some(n) = size
                && n != vars.len()
            {
                return Err(SasError::runtime(format!(
                    "The number of variables in the list ({}) does not match \
                     the number of elements ({}) in the array {}.",
                    vars.len(),
                    n,
                    name
                )));
            }
            vars.to_vec()
        };
        let (ty, length) = match char_len {
            Some(l) => (VarType::Char, l),
            None => (VarType::Num, 8),
        };
        let slots: Vec<usize> = names.iter().map(|v| self.add_var(v, ty, length)).collect();
        self.arrays.insert(upper, slots);
        Ok(())
    }

    /// Type et longueur des éléments d'un array (premier slot — tous les
    /// éléments d'un array M2 partagent type et longueur déclarés ; un
    /// élément préexistant au PDV garde toutefois les siens).
    fn array_elem_type(&self, name: &str) -> (VarType, usize) {
        match self
            .arrays
            .get(&name.to_uppercase())
            .and_then(|slots| slots.first())
        {
            Some(&slot) => {
                let v = &self.pdv.vars()[slot];
                (v.ty, v.length)
            }
            None => (VarType::Num, 8),
        }
    }

    fn compile_set(&mut self, spec: &DatasetSpec) -> Result<()> {
        if self.input.is_some() {
            return Err(SasError::runtime(
                "Multiple SET statements are not yet implemented.",
            ));
        }
        let r = &spec.dref;
        let opts = &spec.options;
        let libref = r.libref_or_work();
        let provider = self.session.libs.get(&libref)?;
        if !provider.exists(&r.name) {
            return Err(SasError::runtime(format!(
                "File {}.DATA does not exist.",
                r.display()
            )));
        }
        let (ds, notes) = provider.read(&r.name)?;
        for note in &notes {
            self.session.log.forward(note);
        }

        // Validation des options : KEEP=/DROP=/RENAME= référencent les noms
        // D'ORIGINE de l'input (règle SAS : KEEP/DROP s'appliquent AVANT
        // RENAME). Un nom absent de l'input → même erreur que l'existant.
        let input_names: HashSet<String> =
            ds.vars.iter().map(|v| v.name.to_uppercase()).collect();
        for name in opts
            .keep
            .iter()
            .flatten()
            .chain(opts.drop.iter().flatten())
            .chain(opts.rename.iter().map(|(old, _)| old))
        {
            if !input_names.contains(&name.to_uppercase()) {
                return Err(SasError::runtime(format!(
                    "The variable {name} in the DROP, KEEP, or RENAME list has never been referenced."
                )));
            }
        }
        let keep_set: Option<HashSet<String>> = opts
            .keep
            .as_ref()
            .map(|v| v.iter().map(|n| n.to_uppercase()).collect());
        let drop_set: HashSet<String> = opts
            .drop
            .iter()
            .flatten()
            .map(|n| n.to_uppercase())
            .collect();
        let rename: HashMap<String, String> = opts
            .rename
            .iter()
            .map(|(old, new)| (old.to_uppercase(), new.clone()))
            .collect();

        let mut columns = Vec::with_capacity(ds.vars.len());
        let mut var_slots = Vec::with_capacity(ds.vars.len());
        for (col, meta) in ds.df.get_columns().iter().zip(&ds.vars) {
            let upper = meta.name.to_uppercase();
            // KEEP=/DROP= filtrent quelles variables d'input entrent au PDV
            // (une variable renommée mais non gardée est ignorée).
            if keep_set.as_ref().is_some_and(|k| !k.contains(&upper))
                || drop_set.contains(&upper)
            {
                continue;
            }
            // RENAME= : la variable entre au PDV sous le NOUVEAU nom
            // (appliqué APRÈS keep/drop).
            let pdv_name = rename
                .get(&upper)
                .cloned()
                .unwrap_or_else(|| meta.name.clone());
            let slot = self.pdv.add_var(PdvVar {
                name: pdv_name,
                ty: meta.ty,
                length: meta.length,
                retained: false,
                from_input: true,
                format: meta.format.clone(),
            });
            // Si la variable existait déjà (référence textuelle antérieure
            // au SET), la marquer issue de l'input malgré tout.
            self.pdv.mark_from_input(slot);
            var_slots.push(slot);

            // Downcast UNE FOIS par colonne — jamais de get_row.
            let s = col.as_materialized_series();
            let values: Vec<Value> = match meta.ty {
                VarType::Num => s.f64()?.iter().map(num_to_value).collect(),
                VarType::Char => s
                    .str()?
                    .iter()
                    .map(|o| Value::Char(o.unwrap_or("").to_string()))
                    .collect(),
            };
            columns.push(values);
        }

        // WHERE= : PAS de filtrage à la compilation — l'Expr est stockée et
        // évaluée par l'exécuteur après chaque chargement de ligne. On
        // walke ses variables pour valider qu'elles existent (elles doivent
        // référencer des variables d'input, déjà au PDV à ce point —
        // post-rename, cf. doc d'InputData).
        if let Some(w) = &opts.where_ {
            self.validate_where_vars(w, &r.display())?;
        }

        self.input = Some(InputData {
            display: r.display(),
            columns,
            var_slots,
            n_rows: ds.n_obs(),
            where_: opts.where_.clone(),
        });
        Ok(())
    }

    /// Toute variable d'un WHERE= de SET doit déjà être au PDV (= une
    /// variable de l'input, après keep/drop/rename) — message proche du
    /// SAS "Variable x is not on file WORK.A.". On ne walke PAS via
    /// `walk_expr` : cela créerait des variables Num parasites au PDV.
    fn validate_where_vars(&self, expr: &Expr, file: &str) -> Result<()> {
        match expr {
            Expr::Num(_) | Expr::Str(_) | Expr::Missing(_) => Ok(()),
            Expr::Var(name) => {
                let upper = name.to_uppercase();
                if upper == "_N_" || upper == "_ERROR_" || self.pdv.slot(name).is_some() {
                    Ok(())
                } else {
                    Err(SasError::runtime(format!(
                        "Variable {name} is not on file {file}."
                    )))
                }
            }
            Expr::Unary { expr, .. } => self.validate_where_vars(expr, file),
            Expr::Binary { left, right, .. } => {
                self.validate_where_vars(left, file)?;
                self.validate_where_vars(right, file)
            }
            Expr::In { expr, list } => {
                self.validate_where_vars(expr, file)?;
                for e in list {
                    self.validate_where_vars(e, file)?;
                }
                Ok(())
            }
            Expr::Index { index, .. } => self.validate_where_vars(index, file),
            Expr::Call { args, .. } => {
                for a in args {
                    self.validate_where_vars(a, file)?;
                }
                Ok(())
            }
        }
    }

    /// Type et longueur inférés d'une expression (compile-time, comme SAS).
    fn infer(&self, expr: &Expr) -> (VarType, usize) {
        match expr {
            Expr::Num(_) | Expr::Missing(_) => (VarType::Num, 8),
            Expr::Str(s) => (VarType::Char, s.chars().count().max(1)),
            Expr::Var(name) => match self.pdv.slot(name) {
                Some(slot) => {
                    let v = &self.pdv.vars()[slot];
                    (v.ty, v.length)
                }
                // Inconnue au moment de l'inférence : numérique.
                None => (VarType::Num, 8),
            },
            Expr::Unary { .. } => (VarType::Num, 8),
            Expr::Binary {
                op: BinaryOp::Concat,
                left,
                right,
            } => (VarType::Char, self.char_len(left) + self.char_len(right)),
            Expr::Binary { .. } | Expr::In { .. } => (VarType::Num, 8),
            // `arr{i}` : type/longueur des éléments de l'array.
            Expr::Index { name, .. } => self.array_elem_type(name),
            Expr::Call { name, args } => {
                // Forme parenthèses `arr(i)` : l'array masque la fonction.
                if args.len() == 1 && self.arrays.contains_key(&name.to_uppercase()) {
                    return self.array_elem_type(name);
                }
                let lower = name.to_ascii_lowercase();
                match lower.as_str() {
                    "upcase" | "lowcase" | "trim" | "strip" | "left" | "right" => {
                        let len = args.first().map_or(200, |a| self.char_len(a));
                        (VarType::Char, len)
                    }
                    "substr" => {
                        let len = args.first().map_or(200, |a| self.char_len(a));
                        (VarType::Char, len)
                    }
                    _ if lower.starts_with("cat") => (VarType::Char, 200),
                    "put" => (VarType::Char, put_width(args)),
                    _ => (VarType::Num, 8),
                }
            }
        }
    }

    /// Longueur d'un opérande en contexte caractère : un opérande numérique
    /// contribue 12 (conversion implicite BEST12., comme SAS).
    fn char_len(&self, expr: &Expr) -> usize {
        match self.infer(expr) {
            (VarType::Char, l) => l,
            (VarType::Num, _) => 12,
        }
    }

    fn resolve_outputs(&mut self, specs: &[DatasetSpec]) -> Result<Vec<OutputSpec>> {
        // Toute variable de KEEP/DROP (statements) doit exister au PDV.
        for name in self.keeps.iter().chain(self.drops.iter()) {
            if self.pdv.slot(name).is_none() {
                return Err(SasError::runtime(format!(
                    "The variable {} in the DROP, KEEP, or RENAME list has never been referenced.",
                    name
                )));
            }
        }
        let stmt_keep: Option<HashSet<String>> = if self.keeps.is_empty() {
            None
        } else {
            Some(self.keeps.iter().map(|n| n.to_uppercase()).collect())
        };
        let stmt_drop: HashSet<String> = self.drops.iter().map(|n| n.to_uppercase()).collect();

        // KEEP ∩ DROP (statements) : DROP gagne, avec WARNING.
        if let Some(ref ks) = stmt_keep {
            for d in &stmt_drop {
                if ks.contains(d) {
                    self.session.log.warning(&format!(
                        "Variable {d} is in both the KEEP and DROP lists; it will be dropped."
                    ));
                }
            }
        }

        let mut outputs = Vec::with_capacity(specs.len());
        for spec in specs {
            let opts = &spec.options;
            // WHERE= n'est pas valide sur une sortie (règle SAS).
            if opts.where_.is_some() {
                return Err(SasError::runtime(
                    "WHERE= is not a valid data set option for output data sets.",
                ));
            }
            // Les variables des options KEEP=/DROP=/RENAME= doivent exister
            // au PDV (KEEP/DROP avant RENAME : tout référence les noms PDV).
            for name in opts
                .keep
                .iter()
                .flatten()
                .chain(opts.drop.iter().flatten())
                .chain(opts.rename.iter().map(|(old, _)| old))
            {
                if self.pdv.slot(name).is_none() {
                    return Err(SasError::runtime(format!(
                        "The variable {name} in the DROP, KEEP, or RENAME list has never been referenced."
                    )));
                }
            }
            let opt_keep: Option<HashSet<String>> = opts
                .keep
                .as_ref()
                .map(|v| v.iter().map(|n| n.to_uppercase()).collect());
            let opt_drop: HashSet<String> = opts
                .drop
                .iter()
                .flatten()
                .map(|n| n.to_uppercase())
                .collect();
            let rename: HashMap<String, String> = opts
                .rename
                .iter()
                .map(|(old, new)| (old.to_uppercase(), new.clone()))
                .collect();

            // Combinaison statements + options : INTERSECTION des keeps
            // (un slot doit passer tous les KEEP présents), union des
            // drops (DROP gagne, sans WARNING supplémentaire pour les
            // options — simplification documentée).
            let mut kept_slots = Vec::new();
            let mut out_names = Vec::new();
            for (i, v) in self.pdv.vars().iter().enumerate() {
                let u = v.name.to_uppercase();
                let kept = stmt_keep.as_ref().is_none_or(|k| k.contains(&u))
                    && opt_keep.as_ref().is_none_or(|k| k.contains(&u))
                    && !stmt_drop.contains(&u)
                    && !opt_drop.contains(&u);
                if kept {
                    kept_slots.push(i);
                    // RENAME= : la colonne ÉCRITE porte le nouveau nom (le
                    // slot PDV, lui, garde son nom).
                    out_names.push(rename.get(&u).cloned().unwrap_or_else(|| v.name.clone()));
                }
            }
            outputs.push(OutputSpec {
                libref: spec.libref_or_work(),
                table: spec.dref.name.clone(),
                display: spec.display(),
                kept_slots,
                out_names,
            });
        }
        Ok(outputs)
    }
}

/// Type, longueur et valeur d'un littéral d'init RETAIN. Le parser ne
/// produit que `Num` (le `-` unaire y est replié), `Str` ou `Missing` ;
/// tout autre nœud est un garde-fou.
fn retain_literal(expr: &Expr) -> Result<(VarType, usize, Value)> {
    match expr {
        Expr::Num(n) => Ok((VarType::Num, 8, Value::Num(*n))),
        Expr::Missing(k) => Ok((VarType::Num, 8, Value::Missing(*k))),
        Expr::Str(s) => Ok((
            VarType::Char,
            s.chars().count().max(1),
            Value::Char(s.clone()),
        )),
        _ => Err(SasError::runtime(
            "RETAIN initial values must be literals.",
        )),
    }
}

/// Largeur du format d'un `put(x, fmt)` : chiffres finaux du nom du format
/// (`best12` → 12), sinon 200. Le parser M1 ne produit pas encore de
/// littéral de format complet — best-effort.
fn put_width(args: &[Expr]) -> usize {
    let Some(fmt) = args.get(1) else { return 200 };
    let name = match fmt {
        Expr::Var(n) => n.as_str(),
        Expr::Str(s) => s.as_str(),
        _ => return 200,
    };
    let trimmed = name.trim_end_matches('.');
    let digits: String = trimmed
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.parse().unwrap_or(200)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{SasDataset, VarMeta};
    use crate::parser::StatementStream;
    use crate::source::SourceFile;
    use polars::df;
    use std::path::PathBuf;

    fn session() -> Session {
        Session::new(None, PathBuf::from("."), true).unwrap()
    }

    /// Écrit un petit dataset (age num avec un missing, name char) dans WORK.
    fn write_class(session: &Session, table: &str) {
        let df = df!(
            "Age" => [Some(14.0), None, Some(13.0)],
            "Name" => ["Alfred", "Alice", "Barbara"],
        )
        .unwrap();
        let vars = vec![
            VarMeta {
                name: "Age".into(),
                ty: VarType::Num,
                length: 8,
                format: None,
                label: None,
            },
            VarMeta {
                name: "Name".into(),
                ty: VarType::Char,
                length: 7,
                format: None,
                label: None,
            },
        ];
        session
            .libs
            .get("WORK")
            .unwrap()
            .write(table, &SasDataset { df, vars })
            .unwrap();
    }

    fn parse_step(src: &str) -> DataStepAst {
        let file = SourceFile::new(src);
        let mut ts = StatementStream::new(&file).unwrap();
        assert!(ts.next().is_kw("data"));
        crate::parser::datastep::parse_data_step(&mut ts).unwrap()
    }

    fn compile_src(src: &str, session: &mut Session) -> Result<StepProgram> {
        compile(&parse_step(src), session)
    }

    #[test]
    fn set_brings_input_vars_in_dataset_order() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src("data out; set inp; x = age + 1; run;", &mut s).unwrap();

        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["Age", "Name", "x"]);
        assert!(prog.pdv.vars()[0].from_input);
        assert!(prog.pdv.vars()[1].from_input);
        assert!(!prog.pdv.vars()[2].from_input);
        assert_eq!(prog.pdv.vars()[1].ty, VarType::Char);
        assert_eq!(prog.pdv.vars()[1].length, 7);

        let input = prog.input.as_ref().unwrap();
        assert_eq!(input.n_rows, 3);
        assert_eq!(input.display, "WORK.INP");
        assert_eq!(input.var_slots, vec![0, 1]);
        // Colonnes décodées en Value, missing `.` inclus.
        assert_eq!(input.columns[0][0], Value::Num(14.0));
        assert_eq!(input.columns[0][1], Value::missing());
        assert_eq!(input.columns[1][2], Value::Char("Barbara".into()));

        // Implicit output : pas de OUTPUT explicite.
        assert!(!prog.has_explicit_output);
        assert!(prog.uninitialized.is_empty());
        assert_eq!(prog.outputs.len(), 1);
        assert_eq!(prog.outputs[0].display, "WORK.OUT");
        assert_eq!(prog.outputs[0].kept_slots, vec![0, 1, 2]);
    }

    #[test]
    fn first_reference_order_without_set() {
        let mut s = session();
        let prog = compile_src("data o; x = y; z = 'abc'; run;", &mut s).unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        // Cible avant les variables de l'expression, ordre textuel.
        assert_eq!(names, vec!["x", "y", "z"]);
        // x inféré Num (y inconnue au moment de l'inférence).
        assert_eq!(prog.pdv.vars()[0].ty, VarType::Num);
        // z : Char(3) du littéral.
        assert_eq!(prog.pdv.vars()[2].ty, VarType::Char);
        assert_eq!(prog.pdv.vars()[2].length, 3);
        // y : référencée jamais assignée → uninitialized.
        assert_eq!(prog.uninitialized, vec!["y".to_string()]);
    }

    #[test]
    fn first_assignment_freezes_type_and_length() {
        let mut s = session();
        let prog = compile_src("data o; s = 'ab'; s = 'abcdef'; run;", &mut s).unwrap();
        assert_eq!(prog.pdv.vars()[0].ty, VarType::Char);
        // La première assignation fige la longueur à 2.
        assert_eq!(prog.pdv.vars()[0].length, 2);
    }

    #[test]
    fn concat_length_is_sum_with_num_as_12() {
        let mut s = session();
        let prog = compile_src("data o; c = 'ab' || 'cde'; d = c || x; run;", &mut s).unwrap();
        // c = 2 + 3.
        assert_eq!(prog.pdv.vars()[0].length, 5);
        assert_eq!(prog.pdv.vars()[0].ty, VarType::Char);
        // d = len(c) + 12 (x numérique).
        let d = &prog.pdv.vars()[1];
        assert_eq!(d.name, "d");
        assert_eq!(d.length, 5 + 12);
    }

    #[test]
    fn call_inference_table() {
        let mut s = session();
        let prog = compile_src(
            "data o; a = 'xyz'; u = upcase(a); t = cats(a, a); n = sum(1, 2); run;",
            &mut s,
        )
        .unwrap();
        let var = |n: &str| {
            let slot = prog.pdv.slot(n).unwrap();
            &prog.pdv.vars()[slot]
        };
        assert_eq!((var("u").ty, var("u").length), (VarType::Char, 3));
        assert_eq!((var("t").ty, var("t").length), (VarType::Char, 200));
        assert_eq!(var("n").ty, VarType::Num);
    }

    #[test]
    fn keep_drop_interaction() {
        let mut s = session();
        let prog = compile_src(
            "data o; x = 1; y = 2; z = 3; keep x y; drop y; run;",
            &mut s,
        )
        .unwrap();
        // keep {x,y} puis drop y → x seul.
        assert_eq!(prog.outputs[0].kept_slots, vec![0]);
        // Le WARNING de l'intersection est dans le log.
        let log = s.log.into_string();
        assert!(log.contains("WARNING"), "log was: {log}");
        assert!(log.contains("KEEP and DROP"), "log was: {log}");
    }

    #[test]
    fn keep_unknown_variable_errors() {
        let mut s = session();
        let err = compile_src("data o; x = 1; keep nosuch; run;", &mut s).err().unwrap();
        assert!(
            err.to_string()
                .contains("in the DROP, KEEP, or RENAME list has never been referenced"),
            "got: {err}"
        );
    }

    #[test]
    fn data_null_has_no_outputs() {
        let mut s = session();
        let prog = compile_src("data _null_; x = 1; run;", &mut s).unwrap();
        assert!(prog.outputs.is_empty());
    }

    #[test]
    fn set_missing_table_errors() {
        let mut s = session();
        let err = compile_src("data o; set nosuch; run;", &mut s).err().unwrap();
        assert_eq!(err.to_string(), "File WORK.NOSUCH.DATA does not exist.");
    }

    #[test]
    fn second_set_errors_m1() {
        let mut s = session();
        write_class(&s, "a");
        write_class(&s, "b");
        let err = compile_src("data o; set a; set b; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("not yet implemented"));
    }

    #[test]
    fn explicit_output_detected_inside_if() {
        let mut s = session();
        let prog = compile_src(
            "data o; x = 1; if x then do; output; end; run;",
            &mut s,
        )
        .unwrap();
        assert!(prog.has_explicit_output);
    }

    #[test]
    fn multiple_outputs_share_kept_slots() {
        let mut s = session();
        let prog = compile_src("data a b; x = 1; y = 2; drop y; run;", &mut s).unwrap();
        assert_eq!(prog.outputs.len(), 2);
        assert_eq!(prog.outputs[0].kept_slots, vec![0]);
        assert_eq!(prog.outputs[1].kept_slots, vec![0]);
        assert_eq!(prog.outputs[0].libref, "WORK");
        assert_eq!(prog.outputs[1].display, "WORK.B");
    }

    #[test]
    fn assign_before_set_still_marks_from_input() {
        let mut s = session();
        write_class(&s, "inp");
        // `age` référencée avant le SET : elle doit malgré tout être
        // marquée from_input (pas de reset à chaque itération).
        let prog = compile_src("data o; age = 0; set inp; run;", &mut s).unwrap();
        let slot = prog.pdv.slot("age").unwrap();
        assert!(prog.pdv.vars()[slot].from_input);
        // Et l'ordre de première référence place age en tête.
        assert_eq!(prog.pdv.vars()[0].name, "age");
    }

    // ── RETAIN (M2) ──────────────────────────────────────────────────────

    #[test]
    fn retain_with_init_creates_retained_var_with_initial_value() {
        let mut s = session();
        let prog = compile_src("data o; retain x 5 s 'ab'; y = 1; run;", &mut s).unwrap();
        // Ordre de première référence : x et s entrent au RETAIN, avant y.
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["x", "s", "y"]);
        assert!(prog.pdv.vars()[0].retained);
        assert!(prog.pdv.vars()[1].retained);
        assert!(!prog.pdv.vars()[2].retained);
        // Types/longueurs des littéraux.
        assert_eq!(prog.pdv.vars()[0].ty, VarType::Num);
        assert_eq!(prog.pdv.vars()[1].ty, VarType::Char);
        assert_eq!(prog.pdv.vars()[1].length, 2);
        // Valeurs initiales.
        assert_eq!(
            prog.initial_values,
            vec![(0, Value::Num(5.0)), (1, Value::Char("ab".into()))]
        );
        // RETAIN avec init = initialisée : pas de NOTE uninitialized.
        assert!(prog.uninitialized.is_empty());
    }

    #[test]
    fn retain_without_init_flags_later_reference_or_creates_num() {
        let mut s = session();
        // k : retenue sans init, type figé par l'assignation ultérieure.
        // j : retenue sans init, jamais référencée → Num + uninitialized,
        // créée en FIN d'ordre PDV (simplification M2 documentée).
        let prog = compile_src("data o; retain k j; x = 1; k = 'ab'; run;", &mut s).unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["x", "k", "j"]);
        let var = |n: &str| &prog.pdv.vars()[prog.pdv.slot(n).unwrap()];
        assert!(var("k").retained);
        assert_eq!(var("k").ty, VarType::Char);
        assert_eq!(var("k").length, 2);
        assert!(var("j").retained);
        assert_eq!(var("j").ty, VarType::Num);
        assert!(!var("x").retained);
        assert_eq!(prog.uninitialized, vec!["j".to_string()]);
        // Pas de valeur initiale sans init.
        assert!(prog.initial_values.is_empty());
    }

    #[test]
    fn retain_bare_retains_whole_pdv() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src("data o; set inp; retain; x = 1; run;", &mut s).unwrap();
        assert!(prog.pdv.vars().iter().all(|v| v.retained));
    }

    // ── Sum statement (M2) ───────────────────────────────────────────────

    #[test]
    fn sum_statement_compiles_retained_num_with_initial_zero() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src("data o; set inp; total + age; run;", &mut s).unwrap();
        let slot = prog.pdv.slot("total").unwrap();
        let v = &prog.pdv.vars()[slot];
        assert_eq!(v.ty, VarType::Num);
        assert_eq!(v.length, 8);
        assert!(v.retained);
        assert_eq!(prog.initial_values, vec![(slot, Value::Num(0.0))]);
        // La cible d'un sum statement compte comme initialisée.
        assert!(prog.uninitialized.is_empty());
    }

    #[test]
    fn retain_init_wins_over_sum_zero_in_both_orders() {
        let mut s = session();
        // RETAIN d'abord : le sum statement ne pousse pas son 0.
        let prog = compile_src("data o; retain n 100; n + 1; run;", &mut s).unwrap();
        let slot = prog.pdv.slot("n").unwrap();
        assert_eq!(prog.initial_values, vec![(slot, Value::Num(100.0))]);
        // Sum d'abord : les deux entrées coexistent, le RETAIN (appliqué en
        // dernier par l'exécuteur) gagne.
        let prog = compile_src("data o; n + 1; retain n 100; run;", &mut s).unwrap();
        let slot = prog.pdv.slot("n").unwrap();
        assert_eq!(
            prog.initial_values,
            vec![(slot, Value::Num(0.0)), (slot, Value::Num(100.0))]
        );
    }

    // ── LENGTH (M2) ──────────────────────────────────────────────────────

    #[test]
    fn length_before_first_reference_fixes_type_and_length() {
        let mut s = session();
        let prog = compile_src(
            "data o; length c $ 3 n 4; c = 'abcdef'; n = 1; run;",
            &mut s,
        )
        .unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["c", "n"]);
        let c = &prog.pdv.vars()[0];
        assert_eq!(c.ty, VarType::Char);
        // La longueur du LENGTH gagne sur celle du littéral assigné.
        assert_eq!(c.length, 3);
        let n = &prog.pdv.vars()[1];
        assert_eq!(n.ty, VarType::Num);
        // Pour une numérique, la longueur est une métadonnée (stockage f64).
        assert_eq!(n.length, 4);
        assert!(prog.uninitialized.is_empty());
    }

    #[test]
    fn length_after_reference_warns_for_differing_char() {
        let mut s = session();
        let prog = compile_src("data o; c = 'ab'; length c $ 10; run;", &mut s).unwrap();
        // La longueur reste figée par la première référence.
        assert_eq!(prog.pdv.vars()[0].length, 2);
        let log = s.log.into_string();
        assert!(
            log.contains("WARNING: Length of character variable c has already been set."),
            "log was: {log}"
        );
    }

    #[test]
    fn length_after_reference_is_silent_for_num_and_same_char_length() {
        let mut s = session();
        let prog = compile_src(
            "data o; x = 1; length x 5; c = 'ab'; length c $ 2; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(prog.pdv.vars()[0].length, 8);
        let log = s.log.into_string();
        assert!(!log.contains("WARNING"), "log was: {log}");
    }

    #[test]
    fn length_out_of_range_errors() {
        let mut s = session();
        let err = compile_src("data o; length n 9; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("out of range (3-8)"), "got: {err}");
        let err = compile_src("data o; length c $ 40000; run;", &mut s)
            .err()
            .unwrap();
        assert!(
            err.to_string().contains("out of range (1-32767)"),
            "got: {err}"
        );
        let err = compile_src("data o; length n 2; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("out of range"), "got: {err}");
    }

    // ── DO itératif / DELETE (M2) ────────────────────────────────────────

    #[test]
    fn do_loop_index_enters_pdv_not_retained_and_assigned() {
        let mut s = session();
        let prog = compile_src("data o; do i = 1 to 3; x = i; end; run;", &mut s).unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        // L'index entre au point du DO, avant les variables du corps.
        assert_eq!(names, vec!["i", "x"]);
        let i = &prog.pdv.vars()[0];
        assert_eq!(i.ty, VarType::Num);
        assert_eq!(i.length, 8);
        assert!(!i.retained);
        // L'index compte comme assigné : pas de NOTE uninitialized.
        assert!(prog.uninitialized.is_empty());
    }

    #[test]
    fn do_loop_bound_and_condition_vars_enter_pdv() {
        let mut s = session();
        let prog = compile_src(
            "data o; do i = a to b by c while(w) until(u); end; run;",
            &mut s,
        )
        .unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        // Index d'abord, puis from/to/by/while/until en ordre textuel.
        assert_eq!(names, vec!["i", "a", "b", "c", "w", "u"]);
        // Les bornes sont référencées jamais assignées → uninitialized.
        assert_eq!(
            prog.uninitialized,
            vec!["a", "b", "c", "w", "u"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn delete_compiles_and_output_in_do_body_is_detected() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src(
            "data o; set inp; if age = . then delete; do i = 1 to 2; output; end; run;",
            &mut s,
        )
        .unwrap();
        assert!(prog.has_explicit_output);
        assert!(prog.pdv.slot("i").is_some());
    }

    // ── ARRAY (M2, lot 3) ────────────────────────────────────────────────

    #[test]
    fn array_elements_enter_pdv_in_order_and_registry_is_filled() {
        let mut s = session();
        let prog = compile_src("data o; array a{3} x y z; b = 1; run;", &mut s).unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        // Les éléments entrent au PDV au point de l'ARRAY, avant b.
        assert_eq!(names, vec!["x", "y", "z", "b"]);
        assert_eq!(prog.arrays.get("A"), Some(&vec![0, 1, 2]));
        assert_eq!(prog.pdv.vars()[0].ty, VarType::Num);
        // Le nom de l'array n'est PAS une variable du PDV.
        assert!(prog.pdv.slot("a").is_none());
    }

    #[test]
    fn array_star_size_deduced_and_char_length_applied() {
        let mut s = session();
        let prog = compile_src("data o; array c{*} $ 5 c1 c2; run;", &mut s).unwrap();
        assert_eq!(prog.arrays.get("C"), Some(&vec![0, 1]));
        for v in prog.pdv.vars() {
            assert_eq!(v.ty, VarType::Char);
            assert_eq!(v.length, 5);
        }
    }

    #[test]
    fn array_auto_named_elements() {
        let mut s = session();
        let prog = compile_src("data o; array a{3}; a{1} = 1; run;", &mut s).unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["a1", "a2", "a3"]);
        assert_eq!(prog.arrays.get("A"), Some(&vec![0, 1, 2]));
    }

    #[test]
    fn array_size_mismatch_errors() {
        let mut s = session();
        let err = compile_src("data o; array a{3} x y; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("does not match"), "got: {err}");
        let err = compile_src("data o; array a{2} x y z; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("does not match"), "got: {err}");
    }

    #[test]
    fn array_star_without_vars_errors() {
        let mut s = session();
        let err = compile_src("data o; array a{*}; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("zero elements"), "got: {err}");
    }

    #[test]
    fn array_redeclaration_errors() {
        let mut s = session();
        let err = compile_src("data o; array a{2} x y; array a{2} u v; run;", &mut s)
            .err()
            .unwrap();
        assert!(
            err.to_string().contains("already been defined"),
            "got: {err}"
        );
    }

    #[test]
    fn undeclared_array_lvalue_errors() {
        let mut s = session();
        // Forme accolades.
        let err = compile_src("data o; nosuch{1} = 0; run;", &mut s).err().unwrap();
        assert!(
            err.to_string().contains("Undeclared array referenced"),
            "got: {err}"
        );
        // Forme parenthèses : validée array à la COMPILATION.
        let err = compile_src("data o; nosuch(1) = 0; run;", &mut s).err().unwrap();
        assert!(
            err.to_string().contains("Undeclared array referenced"),
            "got: {err}"
        );
    }

    #[test]
    fn undeclared_array_rvalue_errors() {
        let mut s = session();
        let err = compile_src("data o; x = nosuch{1}; run;", &mut s).err().unwrap();
        assert!(
            err.to_string().contains("Undeclared array referenced"),
            "got: {err}"
        );
    }

    #[test]
    fn dim_of_array_does_not_create_variable() {
        let mut s = session();
        let prog = compile_src("data o; array a{3} x y z; n = dim(a); run;", &mut s).unwrap();
        // Pas de variable `a` au PDV, et n est bien là.
        assert!(prog.pdv.slot("a").is_none());
        assert!(prog.pdv.slot("n").is_some());
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["x", "y", "z", "n"]);
    }

    #[test]
    fn array_indexed_assignment_marks_elements_initialized() {
        let mut s = session();
        let prog =
            compile_src("data o; array a{3} x y z; do i = 1 to 3; a{i} = i; end; run;", &mut s)
                .unwrap();
        // x, y, z assignés via l'indice : pas de NOTE uninitialized.
        assert!(prog.uninitialized.is_empty());
    }

    #[test]
    fn bare_array_name_reference_errors() {
        let mut s = session();
        // Un nom d'array n'est pas une variable : référence nue illégale.
        let err = compile_src("data o; array a{2} x y; z = a; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("Illegal reference"), "got: {err}");
        let err = compile_src("data o; array a{2} x y; a = 1; run;", &mut s).err().unwrap();
        assert!(err.to_string().contains("Illegal reference"), "got: {err}");
    }

    #[test]
    fn array_indexed_rvalue_infers_element_type() {
        let mut s = session();
        let prog = compile_src(
            "data o; array c{2} $ 4 u v; s = c{1}; t = c(2); n = a(1); array a{2} p q; run;",
            &mut s,
        );
        // `a` est déclaré APRÈS son usage en forme parenthèses : Call normal
        // (fonction inconnue à l'évaluation) — la compilation passe et
        // infère Num. On vérifie surtout s et t.
        let prog = prog.unwrap();
        let var = |n: &str| &prog.pdv.vars()[prog.pdv.slot(n).unwrap()];
        assert_eq!((var("s").ty, var("s").length), (VarType::Char, 4));
        assert_eq!((var("t").ty, var("t").length), (VarType::Char, 4));
        assert_eq!(var("n").ty, VarType::Num);
    }

    #[test]
    fn put_width_parsing() {
        assert_eq!(put_width(&[Expr::Num(1.0), Expr::Var("best12".into())]), 12);
        assert_eq!(put_width(&[Expr::Num(1.0), Expr::Str("date9.".into())]), 9);
        assert_eq!(put_width(&[Expr::Num(1.0), Expr::Var("words".into())]), 200);
        assert_eq!(put_width(&[Expr::Num(1.0)]), 200);
    }

    // ── Options de dataset + OUTPUT ciblé (M2, lot 4) ────────────────────

    #[test]
    fn input_keep_filters_pdv() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src("data o; set inp(keep=name); run;", &mut s).unwrap();
        // Seule Name entre au PDV (Age filtrée AVANT le PDV).
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["Name"]);
        let input = prog.input.as_ref().unwrap();
        assert_eq!(input.columns.len(), 1);
        assert_eq!(input.var_slots, vec![0]);
        assert_eq!(prog.outputs[0].kept_slots, vec![0]);
    }

    #[test]
    fn input_drop_filters_pdv() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src("data o; set inp(drop=age); run;", &mut s).unwrap();
        let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["Name"]);
    }

    #[test]
    fn input_rename_renames_pdv_slot() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src("data o; set inp(rename=(age=years)); x = years; run;", &mut s)
            .unwrap();
        assert!(prog.pdv.slot("years").is_some());
        assert!(prog.pdv.slot("age").is_none());
        // Le slot renommé reste from_input (pas de reset par itération).
        let slot = prog.pdv.slot("years").unwrap();
        assert!(prog.pdv.vars()[slot].from_input);
    }

    #[test]
    fn input_where_is_stored_not_filtered_at_compile() {
        let mut s = session();
        write_class(&s, "inp");
        let prog = compile_src("data o; set inp(where=(age > 13)); run;", &mut s).unwrap();
        let input = prog.input.as_ref().unwrap();
        // Pas de filtrage à la compilation : toutes les lignes présentes.
        assert_eq!(input.n_rows, 3);
        assert!(input.where_.is_some());
    }

    #[test]
    fn input_where_unknown_variable_errors() {
        let mut s = session();
        write_class(&s, "inp");
        let err = compile_src("data o; set inp(where=(nosuch > 1)); run;", &mut s)
            .err()
            .unwrap();
        assert!(
            err.to_string().contains("Variable nosuch is not on file WORK.INP."),
            "got: {err}"
        );
    }

    #[test]
    fn output_option_keep_drop_combined_with_statements() {
        let mut s = session();
        // PDV : x y z. Statement keep x y ; option drop=y → x seul.
        let prog = compile_src(
            "data o(drop=y); x = 1; y = 2; z = 3; keep x y; run;",
            &mut s,
        )
        .unwrap();
        assert_eq!(prog.outputs[0].kept_slots, vec![0]);
        assert_eq!(prog.outputs[0].out_names, vec!["x".to_string()]);
    }

    #[test]
    fn output_option_keep_is_per_output() {
        let mut s = session();
        let prog = compile_src("data a(keep=x) b; x = 1; y = 2; run;", &mut s).unwrap();
        assert_eq!(prog.outputs[0].kept_slots, vec![0]);
        assert_eq!(prog.outputs[1].kept_slots, vec![0, 1]);
    }

    #[test]
    fn output_rename_changes_written_name_not_slot() {
        let mut s = session();
        let prog = compile_src("data o(rename=(x=xx)); x = 1; y = 2; run;", &mut s).unwrap();
        // Le slot PDV garde son nom ; seul le nom d'écriture change.
        assert_eq!(prog.pdv.vars()[0].name, "x");
        assert_eq!(
            prog.outputs[0].out_names,
            vec!["xx".to_string(), "y".to_string()]
        );
        assert_eq!(prog.outputs[0].kept_slots, vec![0, 1]);
    }

    #[test]
    fn where_on_output_dataset_errors() {
        let mut s = session();
        let err = compile_src("data o(where=(x > 1)); x = 1; run;", &mut s)
            .err()
            .unwrap();
        assert_eq!(
            err.to_string(),
            "WHERE= is not a valid data set option for output data sets."
        );
    }

    #[test]
    fn targeted_output_unknown_dataset_errors() {
        let mut s = session();
        let err = compile_src("data a b; x = 1; output c; run;", &mut s)
            .err()
            .unwrap();
        assert_eq!(
            err.to_string(),
            "Output dataset WORK.C is not in the DATA statement output list."
        );
    }

    #[test]
    fn targeted_output_known_dataset_compiles() {
        let mut s = session();
        let prog = compile_src("data a b; x = 1; output a; output a b; run;", &mut s).unwrap();
        assert!(prog.has_explicit_output);
    }

    #[test]
    fn option_variable_never_referenced_errors() {
        let mut s = session();
        write_class(&s, "inp");
        // En entrée : keep= d'une variable absente de l'input.
        let err = compile_src("data o; set inp(keep=nosuch); run;", &mut s)
            .err()
            .unwrap();
        assert!(
            err.to_string()
                .contains("in the DROP, KEEP, or RENAME list has never been referenced"),
            "got: {err}"
        );
        // En entrée : rename= d'une variable absente.
        let err = compile_src("data o; set inp(rename=(nosuch=x)); run;", &mut s)
            .err()
            .unwrap();
        assert!(
            err.to_string().contains("has never been referenced"),
            "got: {err}"
        );
        // En sortie : drop= d'une variable absente du PDV.
        let err = compile_src("data o(drop=nosuch); x = 1; run;", &mut s)
            .err()
            .unwrap();
        assert!(
            err.to_string().contains("has never been referenced"),
            "got: {err}"
        );
    }

    #[test]
    fn renamed_but_dropped_input_variable_is_ignored() {
        let mut s = session();
        write_class(&s, "inp");
        // age est dropée : le rename la concernant est ignoré (pas d'erreur,
        // pas de variable years).
        let prog = compile_src("data o; set inp(drop=age rename=(age=years)); run;", &mut s)
            .unwrap();
        assert!(prog.pdv.slot("years").is_none());
        assert!(prog.pdv.slot("age").is_none());
        assert!(prog.pdv.slot("name").is_some());
    }
}
