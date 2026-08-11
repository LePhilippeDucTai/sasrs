//! ODS et ODS GRAPHICS : ouverture/fermeture des destinations (MQ9.6).

use super::*;

/// M29.1 — exécute un statement `ODS GRAPHICS`.
///
/// Met à jour l'état GLOBAL `session.ods_graphics` à partir des options
/// PAR-STATEMENT, puis émet la NOTE de log. La NOTE reflète UNIQUEMENT ce que
/// CE statement a porté : les dimensions ne sont affichées que si WIDTH=/HEIGHT=
/// ont été fournis dans ce statement précis (même si la session conserve des
/// valeurs antérieures). C'est pourquoi on construit la NOTE AVANT/à partir du
/// statement, pas en relisant `session.ods_graphics`.
pub(crate) fn exec_ods_graphics(session: &mut Session, stmt: &crate::ast::OdsGraphicsStmt) {
    use crate::ast::OdsGraphicsToggle;

    // 1) Appliquer les options de config à l'état de session (persistantes).
    if let Some(w) = stmt.width {
        session.ods_graphics.width = w;
    }
    if let Some(h) = stmt.height {
        session.ods_graphics.height = h;
    }
    if let Some(fmt) = stmt.imagefmt {
        session.ods_graphics.image_format = fmt;
    }
    if let Some(ref name) = stmt.imagename {
        session.ods_graphics.file_stem = Some(name.clone());
    }

    // 2) Appliquer la bascule ON/OFF (persistante).
    match stmt.toggle {
        OdsGraphicsToggle::On => session.ods_graphics.enabled = true,
        OdsGraphicsToggle::Off => session.ods_graphics.enabled = false,
        OdsGraphicsToggle::None => {}
    }

    // 3) Émettre la NOTE — pilotée par CE statement seulement.
    //    Les dimensions n'apparaissent que si elles ont été fournies ici.
    let dims_suffix = match (stmt.width, stmt.height) {
        (Some(w), Some(h)) => Some(format!("(width={}, height={})", w, h)),
        (Some(w), None) => Some(format!("(width={})", w)),
        (None, Some(h)) => Some(format!("(height={})", h)),
        (None, None) => None,
    };
    match stmt.toggle {
        OdsGraphicsToggle::On => match dims_suffix {
            Some(d) => session.log.note(&format!("ODS GRAPHICS ON {}.", d)),
            None => session.log.note("ODS GRAPHICS ON."),
        },
        OdsGraphicsToggle::Off => session.log.note("ODS GRAPHICS OFF."),
        OdsGraphicsToggle::None => match dims_suffix {
            Some(d) => session.log.note(&format!("ODS GRAPHICS {}.", d)),
            None => session.log.note("ODS GRAPHICS."),
        },
    }
}

/// M22.2/M22.4 — exécute un statement `ODS` : ouvre/ferme la destination demandée.
///
/// Invariant : la destination courante reste `session.listing`. `ODS LISTING`
/// réinstalle le listing texte par défaut ; `ODS HTML` ouvre la destination
/// HTML (M22.4 : avec fichier si FILE= est fourni) ; RTF/PDF/EXCEL sont des
/// stubs (note « différé M23 »). `CLOSE` ferme la destination nommée (M22.4 :
/// déclenche l'écriture du fichier HTML si applicable).
pub(crate) fn exec_ods(
    session: &mut Session,
    destination: &str,
    action: OdsAction,
    file: Option<&str>,
    _style: Option<&str>,
) {
    use crate::output::{
        ExcelDestination, HtmlDestination, PdfDestination, RtfDestination, TextListing,
    };

    let dest = destination.to_ascii_lowercase();
    let ls = session.options.ls;

    match action {
        OdsAction::Close => {
            session.close_destination(&dest);
        }
        OdsAction::Open => match dest.as_str() {
            "listing" => {
                session.open_destination("listing", Box::new(TextListing::new(ls)));
            }
            "html" => {
                // M22.4 : si FILE= est fourni, ouvrir avec un fichier cible ;
                // sinon émettre une NOTE informant que la sortie n'est pas
                // matérialisée (aucun fichier).
                if let Some(f) = file {
                    let path = session.resolve_path(f);
                    session
                        .open_destination("html", Box::new(HtmlDestination::with_file(ls, path)));
                } else {
                    session.open_destination("html", Box::new(HtmlDestination::new(ls)));
                    session.log.note(
                        "ODS HTML sans FILE= : la sortie HTML n\u{2019}est pas mat\u{e9}rialis\u{e9}e (v1).",
                    );
                }
            }
            "rtf" => {
                if let Some(f) = file {
                    let path = session.resolve_path(f);
                    session.open_destination("rtf", Box::new(RtfDestination::with_file(ls, path)));
                } else {
                    session.open_destination("rtf", Box::new(RtfDestination::new(ls)));
                    session
                        .log
                        .note("ODS RTF sans FILE= : la sortie RTF n'est pas materialisee (v1).");
                }
            }
            "pdf" => {
                if let Some(f) = file {
                    let path = session.resolve_path(f);
                    session.open_destination("pdf", Box::new(PdfDestination::with_file(ls, path)));
                } else {
                    session.open_destination("pdf", Box::new(PdfDestination::new(ls)));
                    session
                        .log
                        .note("ODS PDF sans FILE= : la sortie PDF n'est pas materialisee (v1).");
                }
            }
            "excel" => {
                if let Some(f) = file {
                    let path = session.resolve_path(f);
                    session
                        .open_destination("excel", Box::new(ExcelDestination::with_file(ls, path)));
                } else {
                    session.open_destination("excel", Box::new(ExcelDestination::new(ls)));
                    session.log.note(
                        "ODS EXCEL sans FILE= : la sortie Excel n'est pas materialisee (v1).",
                    );
                }
            }
            other => {
                session.log.warning(&format!(
                    "ODS destination {} is not supported in this build.",
                    other.to_uppercase()
                ));
            }
        },
    }
}
