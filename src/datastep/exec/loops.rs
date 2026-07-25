use super::*;

impl Runner {
    /// DO itératif / conditionnel — sémantique SAS exacte (cf. en-tête).
    /// from/to/by sont évalués UNE FOIS à l'entrée ; l'index vit au PDV
    /// (le corps peut le modifier). Tout Flow non Normal du corps sort de
    /// la boucle ET remonte (DELETE/STOP/subsetting-IF/SET épuisé).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn exec_do_loop(
        &mut self,
        index: Option<&(String, crate::ast::Expr)>,
        to: Option<&crate::ast::Expr>,
        by: Option<&crate::ast::Expr>,
        while_: Option<&crate::ast::Expr>,
        until: Option<&crate::ast::Expr>,
        body: &[DsStmt],
    ) -> Result<Flow> {
        // Bornes figées à l'entrée (règle SAS). BY défaut 1.0.
        let idx_slot = match index {
            Some((name, from_expr)) => {
                let from = self.loop_control(from_expr)?;
                let Some(slot) = self.pdv.slot(name) else {
                    return Err(SasError::runtime(format!(
                        "Variable {name} is not addressable."
                    )));
                };
                self.pdv.set(slot, Value::Num(from));
                Some(slot)
            }
            None => None,
        };
        let to_v = match to {
            Some(e) => Some(self.loop_control(e)?),
            None => None,
        };
        let by_v = match by {
            Some(e) => self.loop_control(e)?,
            None => 1.0,
        };

        // Garde-fou anti-boucle infinie, PAR exécution de la boucle.
        let mut iters: u64 = 0;
        loop {
            // (1) Test TO : by>0 → i<=to, by<0 → i>=to ; by==0 → jamais de
            // sortie par TO (boucle potentiellement infinie, comme SAS —
            // couverte par le garde-fou).
            if let (Some(slot), Some(stop)) = (idx_slot, to_v) {
                let cur = self.index_value(slot);
                if (by_v > 0.0 && cur > stop) || (by_v < 0.0 && cur < stop) {
                    break;
                }
            }
            // (2) Test WHILE (avant le corps).
            if let Some(cond) = while_ {
                if !self.eval_checked(cond)?.truthy() {
                    break;
                }
            }
            // (3) Corps : un Flow non Normal traverse le DO et remonte.
            for s in body {
                let f = self.exec_stmt(s)?;
                if f != Flow::Normal {
                    return Ok(f);
                }
            }
            // (4) Test UNTIL (après le corps : au moins un tour exécuté).
            if let Some(cond) = until {
                if self.eval_checked(cond)?.truthy() {
                    break;
                }
            }
            // (5) Incrément de l'index (missing + by = missing, comme
            // l'arithmétique SAS).
            if let Some(slot) = idx_slot {
                if let Value::Num(f) = self.pdv.get(slot) {
                    let next = f + by_v;
                    self.pdv.set(slot, Value::Num(next));
                }
            }
            iters += 1;
            if iters > 10_000_000 {
                return Err(SasError::runtime(
                    "DO loop exceeded 10000000 iterations; stopping (possible infinite loop).",
                ));
            }
        }
        Ok(Flow::Normal)
    }

    /// DO sur une liste de valeurs (M16.3). L'index prend successivement
    /// chaque valeur de la liste développée (valeurs explicites évaluées une
    /// par une ; sous-listes `from to e [by k]` énumérées comme un DO
    /// classique). Le corps s'exécute une fois par valeur ; un Flow non
    /// Normal du corps sort de la boucle et remonte.
    pub(super) fn exec_do_list(
        &mut self,
        index: &str,
        items: &[crate::ast::DoListItem],
        body: &[DsStmt],
    ) -> Result<Flow> {
        use crate::ast::DoListItem;
        let Some(idx_slot) = self.pdv.slot(index) else {
            return Err(SasError::runtime(format!(
                "Variable {index} is not addressable."
            )));
        };
        let idx_ty = self.pdv.vars()[idx_slot].ty;
        let mut iters: u64 = 0;
        for item in items {
            match item {
                DoListItem::Value(e) => {
                    let v = self.eval_checked(e)?;
                    let coerced = self.coerce_assign(v, idx_ty);
                    self.pdv.set(idx_slot, coerced);
                    if let Some(f) = self.run_do_list_body(body)? {
                        return Ok(f);
                    }
                    self.bump_do_list_guard(&mut iters)?;
                }
                DoListItem::Range { from, to, by } => {
                    let from_v = self.loop_control(from)?;
                    let to_v = self.loop_control(to)?;
                    let by_v = match by {
                        Some(b) => self.loop_control(b)?,
                        None => 1.0,
                    };
                    if by_v == 0.0 {
                        return Err(SasError::runtime(
                            "Invalid DO loop control information.",
                        ));
                    }
                    let mut cur = from_v;
                    loop {
                        if (by_v > 0.0 && cur > to_v) || (by_v < 0.0 && cur < to_v) {
                            break;
                        }
                        self.pdv.set(idx_slot, Value::Num(cur));
                        if let Some(f) = self.run_do_list_body(body)? {
                            return Ok(f);
                        }
                        self.bump_do_list_guard(&mut iters)?;
                        cur += by_v;
                    }
                }
            }
        }
        Ok(Flow::Normal)
    }

    /// Exécute le corps d'un DO (liste/over) ; renvoie `Some(flow)` si un Flow
    /// non Normal doit remonter, `None` sinon.
    pub(super) fn run_do_list_body(&mut self, body: &[DsStmt]) -> Result<Option<Flow>> {
        for s in body {
            let f = self.exec_stmt(s)?;
            if f != Flow::Normal {
                return Ok(Some(f));
            }
        }
        Ok(None)
    }

    /// Pilote de niveau supérieur d'UNE itération de l'étape (M16.6). Exécute
    /// les statements de premier niveau via un COMPTEUR DE PROGRAMME, ce qui
    /// permet GOTO (saut), LINK (appel de sous-routine, pile d'adresses de
    /// retour) et RETURN (dépile). Sans aucune de ces directives, c'est un
    /// parcours séquentiel équivalent à `for stmt in stmts`.
    ///
    /// Renvoie le `Flow` TERMINAL de l'itération vu par la boucle implicite :
    /// `Normal` (corps épuisé → output implicite), `NextIter` (DELETE / IF
    /// subsetting faux → pas d'output) ou `EndStep` (STOP / fin d'entrée).
    /// Les `Flow::Goto/Link/Return` sont entièrement consommés ici (jamais
    /// remontés au-delà).
    ///
    /// Sémantique RETURN : avec un LINK actif, dépile l'adresse de retour ;
    /// sans LINK actif (pile vide), RETURN termine l'itération NORMALEMENT
    /// (output implicite), comme en SAS. Un LINK sans RETURN atteignant la fin
    /// du corps fait simplement tomber le PC en bout de liste (retour implicite
    /// en fin d'étape).
    pub(super) fn run_step_body(&mut self) -> Result<Flow> {
        let program = self.program.clone();
        let flow_labels = self.flow_labels.clone();
        let mut pc: usize = 0;
        // Garde-fou anti-boucle (GOTO pouvant boucler indéfiniment).
        let mut steps: u64 = 0;
        while pc < program.len() {
            steps += 1;
            if steps > 100_000_000 {
                return Err(SasError::runtime(
                    "DATA step control flow (GOTO/LINK) appears to loop infinitely; stopping.",
                ));
            }
            match self.exec_stmt(&program[pc])? {
                Flow::Normal => pc += 1,
                Flow::NextIter => return Ok(Flow::NextIter),
                Flow::EndStep => return Ok(Flow::EndStep),
                Flow::Goto(label) => {
                    // Cible validée à la compilation : présente dans flow_labels.
                    let Some(&target) = flow_labels.get(&label) else {
                        return Err(SasError::runtime(format!(
                            "The statement label {label} is not defined in the DATA step."
                        )));
                    };
                    pc = target;
                }
                // RETURN au niveau supérieur (hors sous-routine LINK) : fin
                // d'itération normale (output implicite), comme en SAS.
                Flow::Return => return Ok(Flow::Normal),
            }
        }
        // Corps épuisé : fin d'itération normale.
        Ok(Flow::Normal)
    }

    /// Exécute INLINE le corps d'une sous-routine LINK (M16.6) : du statement
    /// étiqueté `label` (premier niveau) jusqu'au prochain `RETURN` (ou la fin
    /// de l'étape). Renvoie le `Flow` à propager au-delà du LINK :
    /// - `Flow::Normal` après un RETURN (ou la fin de l'étape) → on reprend
    ///   normalement après le LINK ;
    /// - `Flow::NextIter`/`EndStep` (DELETE/STOP/fin d'entrée dans la
    ///   sous-routine) → remontés tels quels (terminent l'itération/l'étape) ;
    /// - `Flow::Goto` (GOTO dans la sous-routine) → remonté pour saut non local.
    ///
    /// Un LINK imbriqué (`link` dans la sous-routine) récursionne ici : la pile
    /// d'appels Rust EST la pile d'adresses de retour.
    pub(super) fn exec_link_subroutine(&mut self, label: &str) -> Result<Flow> {
        let program = self.program.clone();
        let flow_labels = self.flow_labels.clone();
        let Some(&start) = flow_labels.get(label) else {
            return Err(SasError::runtime(format!(
                "The statement label {label} is not defined in the DATA step."
            )));
        };
        let mut pc = start;
        let mut steps: u64 = 0;
        while pc < program.len() {
            steps += 1;
            if steps > 100_000_000 {
                return Err(SasError::runtime(
                    "DATA step control flow (LINK) appears to loop infinitely; stopping.",
                ));
            }
            match self.exec_stmt(&program[pc])? {
                Flow::Normal => pc += 1,
                // RETURN : fin de la sous-routine → reprise après le LINK.
                Flow::Return => return Ok(Flow::Normal),
                // GOTO dans une sous-routine : saut non local (remonté au
                // pilote de niveau supérieur, qui repositionne le PC global —
                // la sous-routine est abandonnée, comme en SAS).
                Flow::Goto(label) => return Ok(Flow::Goto(label)),
                // DELETE / STOP / fin d'entrée : terminent l'itération/l'étape.
                Flow::NextIter => return Ok(Flow::NextIter),
                Flow::EndStep => return Ok(Flow::EndStep),
            }
        }
        // Fin de l'étape atteinte sans RETURN : retour implicite.
        Ok(Flow::Normal)
    }

    /// Garde-fou anti-boucle infinie partagé par DO liste / DO OVER.
    pub(super) fn bump_do_list_guard(&self, iters: &mut u64) -> Result<()> {
        *iters += 1;
        if *iters > 10_000_000 {
            return Err(SasError::runtime(
                "DO loop exceeded 10000000 iterations; stopping (possible infinite loop).",
            ));
        }
        Ok(())
    }

    /// DO OVER (M16.3) : itère implicitement sur les éléments d'un array dans
    /// l'ordre row-major (= ordre des `slots`, déjà row-major par
    /// construction). À chaque tour, le slot de l'élément courant est exposé
    /// via `ctx.do_over` (référence nue au nom de l'array = élément courant).
    /// Un Flow non Normal du corps sort de la boucle, en restaurant l'état
    /// `do_over` précédent.
    pub(super) fn exec_do_over(&mut self, array: &str, body: &[DsStmt]) -> Result<Flow> {
        let upper = array.to_uppercase();
        let Some(def) = self.ctx.arrays.get(&upper) else {
            return Err(SasError::runtime(format!(
                "Undeclared array referenced: {array}."
            )));
        };
        let slots = def.slots.clone();
        // Sauvegarde de l'entrée éventuellement masquée (DO OVER imbriqués sur
        // le même nom — improbable, mais correct).
        let prev = self.ctx.do_over.remove(&upper);
        let mut iters: u64 = 0;
        let mut out = Flow::Normal;
        for slot in slots {
            self.ctx.do_over.insert(upper.clone(), slot);
            if let Some(f) = self.run_do_list_body(body)? {
                out = f;
                break;
            }
            self.bump_do_list_guard(&mut iters)?;
        }
        // Restaure l'état précédent.
        match prev {
            Some(p) => {
                self.ctx.do_over.insert(upper, p);
            }
            None => {
                self.ctx.do_over.remove(&upper);
            }
        }
        Ok(out)
    }

    /// Évalue une borne de DO (from/to/by) en numérique. Missing (ou char
    /// vide/invalide) → erreur runtime "Invalid DO loop control
    /// information." qui stoppe l'étape (divergence documentée : SAS émet
    /// une erreur d'exécution équivalente et stoppe l'étape aussi).
    pub(super) fn loop_control(&mut self, expr: &crate::ast::Expr) -> Result<f64> {
        let v = self.eval_checked(expr)?;
        match v {
            Value::Num(f) => Ok(f),
            Value::Char(s) => {
                self.ctx.note_char_to_num = true;
                if let Ok(f) = s.trim().parse::<f64>() {
                    return Ok(f);
                }
                self.ctx.invalid_data += 1;
                self.pdv.error_ = true;
                Err(SasError::runtime("Invalid DO loop control information."))
            }
            Value::Missing(_) => {
                Err(SasError::runtime("Invalid DO loop control information."))
            }
        }
    }

    /// Évalue, propage les fatals, reporte `_ERROR_` au PDV.
    /// SELECT/WHEN/OTHERWISE (M16.1). Cherche la PREMIÈRE clause WHEN qui
    /// correspond, exécute son corps et retourne (pas de fall-through). Sinon
    /// OTHERWISE s'il existe, sinon erreur runtime fidèle à SAS.
    ///
    /// Forme sélecteur (`selector = Some`) : le sélecteur est évalué UNE seule
    /// fois ; chaque valeur de WHEN est comparée avec la sémantique `=` de SAS
    /// (`sas_values_equal`). Forme booléenne (`selector = None`) : chaque WHEN
    /// porte une unique condition évaluée en contexte booléen.
    pub(super) fn exec_select(
        &mut self,
        selector: Option<&crate::ast::Expr>,
        whens: &[crate::ast::WhenClause],
        otherwise: Option<&DsStmt>,
    ) -> Result<Flow> {
        // Sélecteur évalué une seule fois (sémantique SAS).
        let sel_val = match selector {
            Some(expr) => Some(self.eval_checked(expr)?),
            None => None,
        };
        for when in whens {
            let matched = match &sel_val {
                // Forme sélecteur : vrai si le sélecteur égale l'une des
                // valeurs listées (court-circuit dès le premier match).
                Some(sv) => {
                    let mut hit = false;
                    for v in &when.values {
                        let val = self.eval_checked(v)?;
                        if sas_values_equal(sv.clone(), val, &mut self.ctx) {
                            hit = true;
                            break;
                        }
                    }
                    hit
                }
                // Forme booléenne : la condition (unique) est vraie ?
                None => {
                    // Le parser garantit exactement une expression ici.
                    let cond = &when.values[0];
                    self.eval_checked(cond)?.truthy()
                }
            };
            if matched {
                return self.exec_stmt(&when.body);
            }
        }
        match otherwise {
            Some(body) => self.exec_stmt(body),
            None => Err(SasError::runtime(
                "The WHEN list does not match any clause and there is no OTHERWISE clause.",
            )),
        }
    }
}
