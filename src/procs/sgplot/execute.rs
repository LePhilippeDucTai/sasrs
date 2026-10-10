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

/// Variables named by the plot statements, for the `DATA=` check.
fn referenced_vars(ast: &SgplotAst) -> Vec<&str> {
    let mut vars: Vec<&str> = Vec::new();
    for stmt in &ast.plot_stmts {
        match stmt {
            SgplotStmt::Scatter { x, y, group, .. } | SgplotStmt::Series { x, y, group } => {
                vars.extend([x.as_str(), y.as_str()]);
                vars.extend(group.as_deref());
            }
            SgplotStmt::Reg { x, y, .. } | SgplotStmt::Loess { x, y, .. } => {
                vars.extend([x.as_str(), y.as_str()]);
            }
            SgplotStmt::VBar {
                category, response, ..
            }
            | SgplotStmt::HBar {
                category, response, ..
            } => {
                vars.push(category);
                vars.extend(response.as_deref());
            }
            SgplotStmt::Histogram { var, .. } | SgplotStmt::Density { var, .. } => vars.push(var),
            SgplotStmt::VBox { category, response } => {
                vars.push(response);
                vars.extend(category.as_deref());
            }
        }
    }
    vars
}

/// Index of the statement the image engine draws first: the first SCATTER,
/// SERIES, HISTOGRAM or VBAR, else the first statement. Mirrors
/// `graphics_impl::primary_index` (the plot plan moves out of
/// `cfg(feature = "graphics")` in J13-P2).
fn primary_index(stmts: &[SgplotStmt]) -> usize {
    stmts
        .iter()
        .position(|s| {
            matches!(
                s,
                SgplotStmt::Scatter { .. }
                    | SgplotStmt::Series { .. }
                    | SgplotStmt::Histogram { .. }
                    | SgplotStmt::VBar { .. }
            )
        })
        .unwrap_or(0)
}

/// True when the engine draws `stmt` over the primary plot (LOESS, DENSITY,
/// SERIES and SCATTER overlays; `graphics_impl::build_overlays`).
fn drawn_as_overlay(stmt: &SgplotStmt) -> bool {
    matches!(
        stmt,
        SgplotStmt::Loess { .. }
            | SgplotStmt::Density { .. }
            | SgplotStmt::Series { .. }
            | SgplotStmt::Scatter { .. }
    )
}

pub fn execute(ast: &SgplotAst, session: &mut Session) -> Result<()> {
    // 0) J02-P6 — DATA= et variables validées dans les deux builds : le build
    //    par défaut n'ouvrait jamais la table (« image deferred », code 0,
    //    même pour une table ou une variable absente).
    contract::open_checked(&ast.data_ref, session, &referenced_vars(ast))?;

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

    // 3) J02-P6 — composition de l'image, annoncée par cette couche commune
    //    aux deux builds. HBAR/VBOX/REG ne sont pas rendus : en tracé
    //    principal, aucune image (NOTE du moteur, désormais aussi dans le
    //    build par défaut, qui annonçait une image différée) ; superposés à un
    //    autre tracé, ils étaient abandonnés en silence par le moteur, comme
    //    un VBAR ou un HISTOGRAM secondaire : WARNING.
    let primary = primary_index(&ast.plot_stmts);
    let primary_stmt = &ast.plot_stmts[primary];
    if matches!(
        primary_stmt,
        SgplotStmt::HBar { .. } | SgplotStmt::VBox { .. } | SgplotStmt::Reg { .. }
    ) {
        session.log.note(&format!(
            "{} plot deferred (not yet rendered in PROC SGPLOT).",
            stmt_kind(primary_stmt)
        ));
        return Ok(());
    }
    for (i, stmt) in ast.plot_stmts.iter().enumerate() {
        if i == primary || drawn_as_overlay(stmt) {
            continue;
        }
        let unit = match stmt {
            SgplotStmt::HBar { .. } | SgplotStmt::VBox { .. } | SgplotStmt::Reg { .. } => {
                contract::planned("J13-P3")
            }
            _ => String::new(),
        };
        session.log.warning(&format!(
            "The {} statement is ignored in PROC SGPLOT: it is not drawn over the {} plot{unit}.",
            stmt_kind(stmt),
            stmt_kind(primary_stmt)
        ));
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
