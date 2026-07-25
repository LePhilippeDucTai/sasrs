//! Boucle d'exécution principale : tire les blocs un à un et les exécute.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! ```text
//! run_program(src, session):
//!   stream = StatementStream::new(src)            // erreur lexer → ERROR + stop
//!   tant que Some((bloc, span)) = stream.next_block():
//!     session.log.echo_source(src.lines_of_span(span))   // AVANT exécution
//!     selon bloc :
//!       Err(e)            → log.error(e) ; continuer (récupération déjà
//!                            faite par le stream)
//!       Global(stmt)      → exécuter ici même :
//!           Libname    : résoudre le chemin (relatif → session.base_dir),
//!                        libs.assign ; NOTE "Libref XXX was successfully
//!                        assigned as follows: ... Physical Name: ..."
//!           LibnameClear : libs.clear + NOTE
//!           Title      : session.listing.title = texte (M1 : title1)
//!           Options    : appliquer ls= ; autres → WARNING "not supported"
//!       DataStep(ast)     → timer = StepTimer::start()
//!                           datastep::compile(&ast, session) ;
//!                             Err → ERROR + NOTE "The SAS System stopped
//!                                   processing this step because of errors."
//!                           Ok  → datastep::exec::execute → NOTEs lues/écrites
//!                           log.step_used("DATA statement", &timer)
//!       Proc{name, ast}   → procs::execute_proc (timing inclus)
//!       Empty             → rien
//! ```
//! Aucune erreur n'arrête la session (style batch SAS) sauf l'échec du
//! lexer. Le code retour est dérivé des compteurs du LogWriter par
//! lib.rs (0 propre / 1 warnings / 2 erreurs).

use crate::ast::{GlobalStmt, OdsAction};
use crate::datastep;
use crate::error::Result;
use crate::log::StepTimer;
use crate::parser::{Block, StatementStream};
use crate::procs;
use crate::session::Session;
use crate::source::SourceFile;
use std::path::PathBuf;


mod global;

use global::*;

/// M11.5/M11.7 : expansion macro INTERFOLIÉE segment par segment (toujours
/// active désormais — il n'y a plus de feature `macros`).
///
/// On découpe le source ORIGINAL en segments bruts (`RawSegmenter`, coupe sur
/// `run;`/`quit;` de niveau supérieur). Pour CHAQUE segment, dans l'ordre :
/// 1. écho des lignes ORIGINALES du segment (numérotation préservée — cf.
///    divergence ci-dessous) ;
/// 2. `expand_open_code` du texte brut du segment avec l'état VIVANT de
///    l'engine (les `%let`/symput des segments antérieurs sont donc visibles) ;
/// 3. lexing/parsing/exécution du texte expansé via un `StatementStream`
///    transitoire.
///
/// Comme le drain de `CALL SYMPUT` a lieu à la fin de l'étape (donc à la fin
/// du segment qui contient le `run;`), un `&var` du segment SUIVANT voit bien
/// la valeur posée par le symput — c'est l'objectif de M11.5.
///
/// # Écho de numéros de ligne — préservation
/// L'écho reste BLOC PAR BLOC, comme le build par défaut : pour chaque bloc
/// du segment expansé, on écho­te les lignes de son span via
/// `seg_src.lines_of_span(span)`. Le compteur de lignes du `LogWriter`
/// (`src_line`) avance naturellement d'un segment à l'autre. Lorsqu'un
/// segment n'a subi AUCUNE expansion (cas des fixtures sans macro :
/// `expand_open_code` est l'identité), le texte du segment est
/// caractère-pour-caractère la tranche correspondante du source original,
/// donc l'écho et la numérotation sont IDENTIQUES au chemin mono-source de
/// M11.1. La seule divergence POSSIBLE concerne un segment dont l'expansion
/// macro change le nombre/contenu des lignes : on écho­te alors le texte
/// EXPANSÉ de ce segment (pas l'original). C'est sans incidence sur les
/// fixtures de snapshot (aucune n'emploie de macro), et sans fixture dédiée
/// pour ce cas.
pub fn run_program(src: &SourceFile, session: &mut Session) -> Result<()> {
    use crate::preprocess::RawSegmenter;

    let orig = src;
    let mut seg = RawSegmenter::new(&orig.text);
    while let Some((start, end)) = seg.next_segment() {
        let raw = &orig.text[start..end];
        // Expansion avec l'état vivant (visibilité des symput antérieurs).
        let expanded = session.macro_engine.expand_open_code(raw);
        // M19.3 — relayer au log les lignes produites par l'expansion (écho
        // MPRINT/MLOGIC/SYMBOLGEN et sortie de `%put`), AVANT d'exécuter le
        // segment expansé (elles précèdent le code dans le log SAS).
        for line in session.macro_engine.take_pending_log_lines() {
            session.log.put_line(&line);
        }
        // M19.3 — `%call execute(...)` côté macro : mettre en file pour exécution
        // après le segment courant (même file que le CALL EXECUTE des étapes).
        let macro_ce = session.macro_engine.take_pending_call_execute();
        session.call_execute_queue.extend(macro_ce);
        let seg_src = SourceFile::new(expanded);
        let mut stream = match StatementStream::new(&seg_src) {
            Ok(s) => s,
            Err(e) => {
                session.log.error(&e.to_string());
                continue;
            }
        };
        while let Some((block, span)) = stream.next_block() {
            let lines = seg_src.lines_of_span(span);
            let line_texts: Vec<&str> = lines.iter().map(|(_, text)| *text).collect();
            session.log.echo_source(&line_texts);
            run_one_block(block, session);
        }
        // M19.3 — un `%call execute(...)` en code ouvert (hors étape DATA) doit
        // tout de même être rejoué après le segment qui l'a produit. Les DATA
        // steps drainent déjà la file à leur RUN ; ce drain couvre le code
        // ouvert pur (segment sans étape DATA).
        run_call_execute_queue(session);
    }
    Ok(())
}

/// Exécute UN bloc déjà lexé/parsé (commun aux deux builds). L'écho de source
/// est fait par l'appelant (différemment selon le build).
fn run_one_block(block: Result<Block>, session: &mut Session) {
    match block {
        Err(e) => {
            // La récupération de flux est déjà faite par le stream.
            session.log.error(&e.to_string());
        }
        Ok(Block::Empty) => {}
        Ok(Block::Global(stmt)) => exec_global(&stmt, session),
        Ok(Block::DataStep(ast)) => {
            exec_data_step(&ast, session);
            // M35.3 — keep &SYSLAST in sync with session.last_dataset.
            let syslast = session
                .last_dataset
                .as_deref()
                .unwrap_or("_NULL_")
                .to_uppercase();
            session.macro_engine.set_automatic("SYSLAST", syslast);
        }
        Ok(Block::Proc { name, ast }) => {
            if let Err(e) = procs::execute_proc(&name, &ast, session) {
                session.log.error(&e.to_string());
                session
                    .log
                    .note("The SAS System stopped processing this step because of errors.");
            }
            // M35.3 — keep &SYSLAST in sync with session.last_dataset.
            let syslast = session
                .last_dataset
                .as_deref()
                .unwrap_or("_NULL_")
                .to_uppercase();
            session.macro_engine.set_automatic("SYSLAST", syslast);
        }
    }
}

fn exec_data_step(ast: &crate::ast::DataStepAst, session: &mut Session) {
    let timer = StepTimer::start();
    let compiled = datastep::compile(ast, session);
    match compiled {
        Err(e) => {
            session.log.error(&e.to_string());
            session
                .log
                .note("The SAS System stopped processing this step because of errors.");
        }
        Ok(prog) => {
            if let Err(e) = datastep::exec::execute(prog, session) {
                session.log.error(&e.to_string());
                session
                    .log
                    .note("The SAS System stopped processing this step because of errors.");
            }
        }
    }
    // SAS imprime la NOTE de timing même quand l'étape a échoué.
    session.log.step_used("DATA statement", &timer);
    // CALL EXECUTE (M15.6) : le code mis en file pendant l'étape s'exécute
    // APRÈS son RUN. On draine la file et on rejoue le code concaténé comme un
    // programme SAS à part entière (il repasse donc par le processeur macro et
    // les statements globaux/DATA/PROC). Garde de profondeur : le code rejoué
    // peut lui-même générer du CALL EXECUTE, mais on traite la file en boucle
    // tant qu'elle se remplit.
    run_call_execute_queue(session);
}

/// Rejoue (M15.6) le code mis en file par CALL EXECUTE. Chaque entrée est un
/// fragment SAS ; on les concatène (séparés par un saut de ligne) et on les
/// exécute via `run_program`. Si le rejeu re-remplit la file (CALL EXECUTE
/// imbriqué), on boucle, avec une garde de profondeur anti-récursion infinie.
fn run_call_execute_queue(session: &mut Session) {
    let mut depth = 0;
    while !session.call_execute_queue.is_empty() {
        depth += 1;
        if depth > 1000 {
            session.log.error(
                "CALL EXECUTE generated too many nested steps (possible infinite loop); stopping.",
            );
            session.call_execute_queue.clear();
            return;
        }
        let code = std::mem::take(&mut session.call_execute_queue).join("\n");
        let src = SourceFile::new(code);
        let _ = run_program(&src, session);
    }
}

#[cfg(test)]
mod tests;
