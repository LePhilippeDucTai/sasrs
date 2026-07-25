use super::*;

// ───────────────────────── Execute ─────────────────────────

/// Nom lisible d'un statement de tracé, pour les NOTE.
pub(super) fn stmt_kind(stmt: &SgplotStmt) -> &'static str {
    match stmt {
        SgplotStmt::Scatter { .. } => "SCATTER",
        SgplotStmt::Series { .. } => "SERIES",
        SgplotStmt::VBar { .. } => "VBAR",
        SgplotStmt::HBar { .. } => "HBAR",
        SgplotStmt::Histogram { .. } => "HISTOGRAM",
        SgplotStmt::Density { .. } => "DENSITY",
        SgplotStmt::VBox { .. } => "VBOX",
        SgplotStmt::Reg { .. } => "REG",
        SgplotStmt::Loess { .. } => "LOESS",
    }
}

pub fn execute(ast: &SgplotAst, session: &mut Session) -> Result<()> {
    // 1) ODS GRAPHICS non activé → NOTE de non-activation, EXIT 0.
    if !session.ods_graphics.enabled {
        session.log.note(
            "ODS GRAPHICS is not enabled. Use \"ods graphics on;\" before PROC SGPLOT to generate images.",
        );
        return Ok(());
    }

    // 2) Aucun statement de tracé : rien à dessiner.
    let first = match ast.plot_stmts.first() {
        Some(s) => s,
        None => {
            session
                .log
                .note("No plot statement found in PROC SGPLOT; nothing to plot.");
            return Ok(());
        }
    };

    // 3) BY-group → différé (NOTE), même sous --features graphics.
    if ast.by_var.is_some() {
        session
            .log
            .note("BY-group processing deferred in PROC SGPLOT.");
        return Ok(());
    }

    // 4) LOESS / DENSITY : différés UNIQUEMENT dans le build par défaut (sans
    //    --features graphics). Sous graphics, ils sont rendus (M34.11). On
    //    garde la NOTE de différé byte-identique au build par défaut.
    #[cfg(not(feature = "graphics"))]
    match first {
        SgplotStmt::Loess { .. } => {
            session
                .log
                .note("LOESS plot deferred (not yet implemented in PROC SGPLOT).");
            return Ok(());
        }
        SgplotStmt::Density { .. } => {
            session
                .log
                .note("DENSITY plot deferred (not yet implemented in PROC SGPLOT).");
            return Ok(());
        }
        _ => {}
    }

    // 5) Génération de l'image.
    #[cfg(not(feature = "graphics"))]
    {
        // v1 (build par défaut) : un seul plot par image — prévenir si plusieurs.
        if ast.plot_stmts.len() > 1 {
            session.log.note(&format!(
                "PROC SGPLOT v1 renders only the first plot statement ({}); {} additional statement(s) ignored.",
                stmt_kind(first),
                ast.plot_stmts.len() - 1
            ));
        }
        let _ = first;
        session
            .log
            .note("ODS GRAPHICS: image deferred (compile with --features graphics).");
        Ok(())
    }

    #[cfg(feature = "graphics")]
    {
        let _ = first;
        graphics_impl::render(ast, session)
    }
}
