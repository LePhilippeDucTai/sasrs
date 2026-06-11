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

#![allow(unused_variables, dead_code)]

use crate::error::Result;
use crate::session::Session;
use crate::source::SourceFile;

pub fn run_program(src: &SourceFile, session: &mut Session) -> Result<()> {
    todo!("cf. plan en tête de fichier")
}
