use super::*;

impl Compiler<'_> {
    /// Compile un `DO` itératif/conditionnel (bras `DsStmt::DoLoop` de `walk_stmt`).
    pub(super) fn compile_do_loop(
        &mut self,
        index: &Option<(String, Expr)>,
        to: &Option<Expr>,
        by: &Option<Expr>,
        while_: &Option<Expr>,
        until: &Option<Expr>,
        body: &[DsStmt],
    ) -> Result<()> {
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

    /// Compile un `DO` sur liste de valeurs (bras `DsStmt::DoList` de `walk_stmt`).
    pub(super) fn compile_do_list(
        &mut self,
        index: &str,
        items: &[DoListItem],
        body: &[DsStmt],
    ) -> Result<()> {
        let (ty, length) = do_list_index_type(items);
        self.add_var(index, ty, length);
        self.assigned.insert(index.to_uppercase());
        for item in items {
            match item {
                DoListItem::Value(e) => self.walk_expr(e)?,
                DoListItem::Range { from, to, by } => {
                    self.walk_expr(from)?;
                    self.walk_expr(to)?;
                    if let Some(b) = by {
                        self.walk_expr(b)?;
                    }
                }
            }
        }
        for s in body {
            self.walk_stmt(s)?;
        }
        Ok(())
    }

    /// Compile un `DO OVER` (bras `DsStmt::DoOver` de `walk_stmt`).
    pub(super) fn compile_do_over(&mut self, array: &str, body: &[DsStmt]) -> Result<()> {
        let upper = array.to_uppercase();
        if !self.arrays.contains_key(&upper) {
            return Err(SasError::runtime(format!(
                "Undeclared array referenced: {array}."
            )));
        }
        let newly = self.do_over_arrays.insert(upper.clone());
        let mut result = Ok(());
        for s in body {
            if let Err(e) = self.walk_stmt(s) {
                result = Err(e);
                break;
            }
        }
        if newly {
            self.do_over_arrays.remove(&upper);
        }
        result
    }

    /// Compile un `SELECT` (bras `DsStmt::Select` de `walk_stmt`).
    pub(super) fn compile_select(
        &mut self,
        selector: &Option<Expr>,
        whens: &[WhenClause],
        otherwise: &Option<Box<DsStmt>>,
    ) -> Result<()> {
        if let Some(sel) = selector {
            self.walk_expr(sel)?;
        }
        for when in whens {
            for v in &when.values {
                self.walk_expr(v)?;
            }
            self.walk_stmt(&when.body)?;
        }
        if let Some(o) = otherwise {
            self.walk_stmt(o)?;
        }
        Ok(())
    }

    /// Compile un `OUTPUT` (bras `DsStmt::Output` de `walk_stmt`).
    pub(super) fn compile_output(&mut self, targets: &[DatasetRef]) -> Result<()> {
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
}
