//! M38.4 — ODS SELECT/EXCLUDE : liste de sélection des objets ODS affichés.
//!
//! `ods select A B;` / `ods exclude A;` filtrent quels objets ODS (les tables
//! nommées de M38.3 : `OneWayFreqs`, `Moments`, `BasicMeasures`, `Summary`, …)
//! sont ÉMIS vers la destination de sortie. Le filtrage a lieu au point
//! d'émission : un objet filtré n'apparaît dans AUCUNE destination, mais le
//! LOG du proc (NOTEs de lecture, timing) reste inchangé — et les datasets
//! (OUT=, ODS OUTPUT) sont toujours produits.
//!
//! ## Cycle de vie (SAS 9.4, comportement par défaut sans PERSIST)
//!
//! Doc SAS (« ODS SELECT Statement », « Using Selection and Exclusion
//! Lists ») :
//! - une liste NOMINATIVE (`ods select Moments;`) est consommée à la fin du
//!   step de procédure suivant : « ODS removes from the list all output
//!   objects that were not specified with the PERSIST option » — la liste
//!   revient alors à son défaut `SELECT ALL` ;
//! - les listes `ALL`/`NONE` PERSISTENT : « if the list contains the argument
//!   ALL or NONE, it is not modified at the end of a procedure or DATA step ».
//!   C'est l'idiome SAS `ods exclude all; …plusieurs procs…; ods exclude
//!   none;` qui supprime la sortie de TOUS les steps intermédiaires ;
//! - chaque nouveau statement `ODS SELECT`/`ODS EXCLUDE` REMPLACE la liste
//!   entière ;
//! - équivalences : `ods select none;` ≡ `ods exclude all;` et
//!   `ods exclude none;` ≡ `ods select all;` (retour au défaut).
//!
//! ## Interaction avec ODS OUTPUT (M38.3)
//!
//! Doc SAS (« Excluding ODS Tables from Display », SAS/STAT UG) : l'exclusion
//! ne gouverne que l'AFFICHAGE — `ods output Table=ds;` capture une table même
//! exclue du listing. Les chemins de capture (`Session::append_ods_output`,
//! `ods_output_target`) sont donc indépendants de cette liste et JAMAIS
//! filtrés.
//!
//! ## Écarts SAS assumés (documentés)
//! - La liste est GLOBALE (une seule liste de session), pas par destination
//!   ouverte : sasrs route toute la sortie vers la destination courante
//!   unique, « overall list » et « destination list » coïncident donc en
//!   pratique. `ODS <dest> SELECT …` est accepté et appliqué globalement.
//! - Une liste NOMINATIVE ne filtre que les tables qui portent déjà un nom
//!   d'objet ODS (M38.3/M38.4 : FREQ une voie, UNIVARIATE, MEANS) ; les tables
//!   encore anonymes s'affichent toujours, même sous `ods select autre-nom;`.
//!   `ODS SELECT NONE`/`ODS EXCLUDE ALL` suppriment en revanche TOUTE la
//!   sortie listing des steps suivants, tables anonymes comprises (détour de
//!   la destination dans `procs::execute_proc`).
//! - La liste nominative n'est consommée qu'aux frontières de step de
//!   PROCÉDURE (les étapes DATA de sasrs ne produisent pas d'objets ODS —
//!   même divergence que le registre ODS OUTPUT).
//! - `nom(PERSIST)` n'est pas supporté (erreur de parse explicite).

use super::Session;

/// La liste de sélection ODS courante de la session (une seule, globale).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum OdsSelection {
    /// `SELECT ALL` / `EXCLUDE NONE` — tout s'affiche (défaut). Persiste.
    #[default]
    All,
    /// `SELECT NONE` / `EXCLUDE ALL` — rien ne s'affiche. Persiste.
    None,
    /// `SELECT nom …` — seuls les objets nommés s'affichent (noms en
    /// UPPERCASE). Consommée à la fin du step de procédure suivant.
    Select(Vec<String>),
    /// `EXCLUDE nom …` — les objets nommés sont supprimés (noms en UPPERCASE).
    /// Consommée à la fin du step de procédure suivant.
    Exclude(Vec<String>),
}

impl Session {
    /// M38.4 — applique un statement `ODS SELECT`/`ODS EXCLUDE` : REMPLACE la
    /// liste de sélection courante (sémantique SAS — chaque statement repart
    /// de zéro). `items` porte soit le mot-clé `ALL`/`NONE` seul, soit des
    /// noms d'objets ODS (le parser garantit qu'ils ne se mélangent pas).
    /// Silencieux, comme SAS (aucune NOTE de log).
    pub fn set_ods_selection(&mut self, exclude: bool, items: &[String]) {
        let keyword = (items.len() == 1)
            .then(|| items[0].to_ascii_lowercase())
            .filter(|k| k == "all" || k == "none");
        self.ods_selection = match keyword.as_deref() {
            // `select all` ≡ `exclude none` ; `select none` ≡ `exclude all`.
            Some("all") if !exclude => OdsSelection::All,
            Some("none") if exclude => OdsSelection::All,
            Some(_) => OdsSelection::None,
            None => {
                let upper: Vec<String> = items.iter().map(|s| s.to_ascii_uppercase()).collect();
                if exclude {
                    OdsSelection::Exclude(upper)
                } else {
                    OdsSelection::Select(upper)
                }
            }
        };
    }

    /// M38.4 — l'objet ODS `object` doit-il être AFFICHÉ (émis vers la
    /// destination de sortie) sous la liste de sélection courante ? Matching
    /// insensible à la casse. Les procs consultent cette méthode à chaque site
    /// de `write_table` nommé ; elle ne gouverne QUE l'affichage — jamais les
    /// captures ODS OUTPUT ni les datasets OUT=.
    pub fn ods_displays(&self, object: &str) -> bool {
        match &self.ods_selection {
            OdsSelection::All => true,
            OdsSelection::None => false,
            OdsSelection::Select(list) => list.iter().any(|n| n.eq_ignore_ascii_case(object)),
            OdsSelection::Exclude(list) => !list.iter().any(|n| n.eq_ignore_ascii_case(object)),
        }
    }

    /// M38.4 — la liste courante supprime-t-elle TOUT (`SELECT NONE` /
    /// `EXCLUDE ALL`) ? Dans ce cas `procs::execute_proc` détourne la
    /// destination de sortie vers un puits jetable pour toute la durée du proc
    /// (aucune page, aucun en-tête — comme SAS).
    pub fn ods_suppresses_all(&self) -> bool {
        self.ods_selection == OdsSelection::None
    }

    /// M38.4 — frontière de step de procédure (executor) : une liste
    /// NOMINATIVE est consommée (retour au défaut `SELECT ALL`) ; les listes
    /// `ALL`/`NONE` persistent (doc SAS : « if the list contains the argument
    /// ALL or NONE, it is not modified at the end of a procedure or DATA
    /// step »).
    pub fn ods_select_step_boundary(&mut self) {
        match self.ods_selection {
            OdsSelection::Select(_) | OdsSelection::Exclude(_) => {
                self.ods_selection = OdsSelection::All;
            }
            OdsSelection::All | OdsSelection::None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::make_session;

    fn strs(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// Défaut : liste `ALL`, tout s'affiche, rien n'est supprimé globalement.
    #[test]
    fn default_selection_displays_everything() {
        let session = make_session();
        assert_eq!(session.ods_selection, OdsSelection::All);
        assert!(session.ods_displays("OneWayFreqs"));
        assert!(session.ods_displays("anything"));
        assert!(!session.ods_suppresses_all());
    }

    /// `ods select A B;` : seuls A et B s'affichent, matching insensible à la
    /// casse (noms stockés en UPPERCASE).
    #[test]
    fn select_list_filters_by_name_case_insensitively() {
        let mut session = make_session();
        session.set_ods_selection(false, &strs(&["Moments", "quantiles"]));
        assert!(session.ods_displays("MOMENTS"));
        assert!(session.ods_displays("Moments"));
        assert!(session.ods_displays("Quantiles"));
        assert!(!session.ods_displays("BasicMeasures"));
        assert!(!session.ods_suppresses_all());
    }

    /// `ods exclude A;` : tout s'affiche sauf A.
    #[test]
    fn exclude_list_suppresses_named_objects_only() {
        let mut session = make_session();
        session.set_ods_selection(true, &strs(&["OneWayFreqs"]));
        assert!(!session.ods_displays("onewayfreqs"));
        assert!(session.ods_displays("Moments"));
        assert!(!session.ods_suppresses_all());
    }

    /// Équivalences des mots-clés : `select none` ≡ `exclude all` (tout
    /// supprimé) ; `select all` ≡ `exclude none` (retour au défaut).
    #[test]
    fn all_none_keywords_map_to_the_two_persistent_states() {
        let mut session = make_session();
        session.set_ods_selection(false, &strs(&["none"]));
        assert_eq!(session.ods_selection, OdsSelection::None);
        assert!(session.ods_suppresses_all());
        assert!(!session.ods_displays("Moments"));

        session.set_ods_selection(true, &strs(&["NONE"]));
        assert_eq!(session.ods_selection, OdsSelection::All);

        session.set_ods_selection(true, &strs(&["All"]));
        assert_eq!(session.ods_selection, OdsSelection::None);

        session.set_ods_selection(false, &strs(&["all"]));
        assert_eq!(session.ods_selection, OdsSelection::All);
    }

    /// Cycle de vie SAS : une liste nominative est consommée à la frontière de
    /// step (retour à `ALL`) ; `ALL`/`NONE` persistent.
    #[test]
    fn step_boundary_consumes_named_lists_but_not_all_none() {
        let mut session = make_session();

        session.set_ods_selection(false, &strs(&["Moments"]));
        session.ods_select_step_boundary();
        assert_eq!(session.ods_selection, OdsSelection::All);

        session.set_ods_selection(true, &strs(&["Moments"]));
        session.ods_select_step_boundary();
        assert_eq!(session.ods_selection, OdsSelection::All);

        // `exclude all` survit aux frontières de step (idiome SAS
        // `ods exclude all; …procs…; ods exclude none;`).
        session.set_ods_selection(true, &strs(&["all"]));
        session.ods_select_step_boundary();
        assert_eq!(session.ods_selection, OdsSelection::None);

        session.set_ods_selection(false, &strs(&["all"]));
        session.ods_select_step_boundary();
        assert_eq!(session.ods_selection, OdsSelection::All);
    }

    /// Chaque statement REMPLACE la liste entière (pas de fusion).
    #[test]
    fn each_statement_replaces_the_whole_list() {
        let mut session = make_session();
        session.set_ods_selection(false, &strs(&["Moments"]));
        session.set_ods_selection(false, &strs(&["Quantiles"]));
        assert!(!session.ods_displays("Moments"));
        assert!(session.ods_displays("Quantiles"));

        session.set_ods_selection(true, &strs(&["Quantiles"]));
        assert!(session.ods_displays("Moments"));
        assert!(!session.ods_displays("Quantiles"));
    }
}
