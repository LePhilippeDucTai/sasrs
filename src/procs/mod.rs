//! Registre des PROCs : chaque PROC possède son propre parser (grammaire
//! contextuelle SAS) et son exécuteur.
//!
//! # Plan du fichier — voir PLAN.md
//!
//! `parse_proc(name, ts)` est appelé par `parser::next_block()` APRÈS
//! consommation de `proc <name>` ; le sous-parser consomme tout jusqu'à
//! `run;` (ou `quit;` pour les run-group procs : SQL, DATASETS).
//! PROC inconnue → ERROR "Procedure XXX not found." + récupération.
//!
//! `execute_proc` dispatche, entoure d'un `StepTimer`, et écrit la NOTE
//! de fin "NOTE: PROCEDURE XXX used (Total process time): ...".
//!
//! Convention commune : `data=` absent → `session.last_dataset` (_LAST_) ;
//! aucun dataset créé dans la session → ERROR (comme SAS _LAST_ vide).

pub mod anova;
pub mod append;
pub mod catalog;
pub mod cluster;
pub mod common;
pub mod compare;
pub mod contents;
pub mod corr;
pub mod datasets;
pub mod discrim;
pub mod distance;
pub mod export;
pub mod factor;
pub mod fastclus;
pub mod format;
pub mod freq;
pub mod gchart;
pub mod genmod;
pub mod glimmix;
pub mod glm;
pub mod gplot;
pub mod iml;
pub mod import;
pub mod lincom;
pub mod logistic;
pub mod means;
pub mod mixed;
pub mod npar1way;
pub mod options;
pub mod plot;
pub mod princomp;
pub mod print;
pub mod printto;
pub mod rank;
pub mod reg;
pub mod report;
pub mod sgplot;
pub mod sort;
pub mod tabulate;
pub mod transpose;
pub mod ttest;
pub mod univariate;

use crate::error::{Result, SasError};
use crate::log::StepTimer;
use crate::parser::StatementStream;
use crate::session::Session;

/// Registre déclaratif des PROCs (MQ4.6). Une invocation unique génère les
/// trois artefacts jusqu'ici maintenus à la main :
///
/// - l'enum `ProcAst` (un variant par proc) ;
/// - `parse_proc` : dispatch du nom de proc vers `module::parse(ts)` ;
/// - `execute_proc` : dispatch du variant vers `module::execute(a, session)`,
///   entouré du `StepTimer` et de la NOTE de fin.
///
/// Une proc RÉGULIÈRE tient en une ligne de la section `regular` :
/// `"mot_clé" => Variant(module::AstType),`. Les cas irréguliers (alias
/// MEANS/SUMMARY avec post-traitement, SQL dont parser/exécuteur vivent dans
/// `crate::sql`) passent par les sections `special_*`, écrites en clair dans
/// l'invocation. Les identifiants passés à `params:` nomment les paramètres
/// des fonctions générées, si bien que les bras `special_*` (côté invocation)
/// peuvent y faire référence malgré l'hygiène des macros.
macro_rules! procs_registry {
    (
        params: ($name:ident, $ts:ident, $ast:ident, $session:ident);
        regular: { $( $kw:literal => $variant:ident($module:ident :: $asty:ident), )* }
        special_variants: { $( $svariant:ident($sty:ty), )* }
        special_parse: { $( $spat:pat => $sbody:block )* }
        special_execute: { $( $epat:pat => $ebody:expr, )* }
    ) => {
        pub enum ProcAst {
            $( $variant($module::$asty), )*
            $( $svariant($sty), )*
        }

        /// Parse a PROC block. Called AFTER `proc <name>` has been consumed.
        /// Dispatches to the sub-parser of the proc, by name.
        ///
        /// Nom de proc inconnu : `skip_to_semi()` puis
        /// `Err("Procedure XXX not found.")` — l'appelant
        /// (`parser::parse_block`) saute ensuite jusqu'à la frontière d'étape.
        pub fn parse_proc($name: &str, $ts: &mut StatementStream) -> Result<ProcAst> {
            match $name.to_ascii_lowercase().as_str() {
                $( $kw => Ok(ProcAst::$variant($module::parse($ts)?)), )*
                $( $spat => $sbody )*
                _ => {
                    // Proc inconnue : finir le statement courant ; le caller
                    // (parser::parse_block) saute ensuite jusqu'à la frontière.
                    $ts.skip_to_semi();
                    Err(SasError::runtime(format!(
                        "Procedure {} not found.",
                        $name.to_uppercase()
                    )))
                }
            }
        }

        /// Execute a previously parsed PROC AST. Wraps with a StepTimer and writes
        /// the "NOTE: PROCEDURE XXX used (Total process time):" note.
        pub fn execute_proc($name: &str, $ast: &ProcAst, $session: &mut Session) -> Result<()> {
            let timer = StepTimer::start();

            let result = match $ast {
                $( ProcAst::$variant(a) => $module::execute(a, $session), )*
                $( $epat => $ebody, )*
            };

            // Write timing NOTE even on success (SAS always prints this).
            // On error the caller may still want the timing, but we follow SAS: only
            // write it on success.
            if result.is_ok() {
                $session
                    .log
                    .step_used(&format!("PROCEDURE {}", $name.to_uppercase()), &timer);
            }

            result
        }
    };
}

procs_registry! {
    params: (name, ts, ast, session);
    regular: {
        "print" => Print(print::PrintAst),
        "sort" => Sort(sort::SortAst),
        "freq" => Freq(freq::FreqAst),
        "transpose" => Transpose(transpose::TransposeAst),
        "univariate" => Univariate(univariate::UnivariateAst),
        "contents" => Contents(contents::ContentsAst),
        "corr" => Corr(corr::CorrAst),
        "rank" => Rank(rank::RankAst),
        "datasets" => Datasets(datasets::DatasetsAst),
        "append" => Append(append::AppendAst),
        "format" => Format(format::FormatAst),
        "tabulate" => Tabulate(tabulate::TabulateAst),
        "report" => Report(report::ReportAst),
        "import" => Import(import::ImportAst),
        "export" => Export(export::ExportAst),
        "compare" => Compare(compare::CompareAst),
        "printto" => Printto(printto::PrinttoAst),
        "options" => Options(options::OptionsAst),
        "catalog" => Catalog(catalog::CatalogAst),
        "ttest" => TTest(ttest::TTestAst),
        "npar1way" => Npar1way(npar1way::NparAst),
        "reg" => Reg(reg::RegAst),
        "anova" => Anova(anova::AnovaAst),
        "glm" => Glm(glm::GlmAst),
        "genmod" => Genmod(genmod::GenmodAst),
        "logistic" => Logistic(logistic::LogisticAst),
        "princomp" => Princomp(princomp::PrincompAst),
        "factor" => Factor(factor::FactorAst),
        "distance" => Distance(distance::DistanceAst),
        "cluster" => Cluster(cluster::ClusterAst),
        "fastclus" => Fastclus(fastclus::FastclusAst),
        "discrim" => Discrim(discrim::DiscrimAst),
        "mixed" => Mixed(mixed::MixedAst),
        "glimmix" => Glimmix(glimmix::GlimmixAst),
        "iml" => Iml(iml::ImlProgram),
        "sgplot" => Sgplot(sgplot::SgplotAst),
        "plot" => Plot(plot::PlotAst),
        "gplot" => Gplot(gplot::GplotAst),
        "gchart" => Gchart(gchart::GchartAst),
    }
    special_variants: {
        Means(means::MeansAst),
        Sql(crate::sql::ast::SqlProgram),
    }
    special_parse: {
        // PROC MEANS / PROC SUMMARY share a parser/executor. PROC SUMMARY is
        // PROC MEANS with NOPRINT defaulted to true: it suppresses the report
        // and is normally paired with an OUTPUT statement. (Simplification:
        // we always force noprint=true for SUMMARY; an explicit PRINT option
        // on SUMMARY is uncommon and not re-enabled here.)
        "means" | "summary" => {
            let mut ast = means::parse(ts)?;
            if name.eq_ignore_ascii_case("summary") {
                ast.summary = true;
                ast.noprint = true;
            }
            Ok(ProcAst::Means(ast))
        }
        // PROC SQL : parser et exécuteur vivent dans `crate::sql`, pas dans un
        // module de `procs/`.
        "sql" => {
            let prog = crate::sql::parser::parse_sql_program(ts)?;
            Ok(ProcAst::Sql(prog))
        }
    }
    special_execute: {
        ProcAst::Means(a) => means::execute(a, session),
        ProcAst::Sql(a) => crate::sql::execute(a, session),
    }
}

#[cfg(test)]
mod tests;
