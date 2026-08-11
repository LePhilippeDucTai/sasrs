//! M38.3 — ODS OUTPUT généralisé : registre de capture, accumulation par
//! table ODS et matérialisation en datasets.
//!
//! Chaque table de résultats émise par un proc porte un nom d'objet ODS — les
//! noms SAS réels : `OneWayFreqs`, `Moments`, `BasicMeasures`, `Summary`,
//! `TTest`, … Le proc qui produit une table nommée en fournit la version
//! TABULAIRE TYPÉE (les colonnes numériques restent numériques — pas les
//! cellules formatées du listing) via [`Session::append_ods_output`], juste à
//! côté de son `write_table` listing. Le cycle de vie suit SAS :
//!
//! 1. `ods output Table=ds;` enregistre la demande
//!    ([`Session::set_ods_output`]). Registre vide par défaut → aucune capture,
//!    listing byte-identique.
//! 2. Pendant le proc suivant, chaque tranche produite est ACCUMULÉE
//!    ([`Session::append_ods_output`]) : plusieurs variables / requêtes TABLES /
//!    groupes BY s'empilent dans le même dataset, avec union « diagonale » des
//!    colonnes (une colonne absente d'une tranche est remplie de missings —
//!    c'est le comportement SAS pour `tables age sex;` dans OneWayFreqs).
//! 3. En fin de proc (`procs::execute_proc`, AVANT la NOTE de timing —
//!    position SAS), [`Session::flush_ods_output`] écrit chaque dataset
//!    accumulé et émet la NOTE « The data set … has N observations and M
//!    variables. ».
//! 4. À la frontière de step (executor, APRÈS la NOTE de timing — position
//!    SAS), [`Session::ods_output_step_boundary`] émet le WARNING
//!    « Output '…' was not created. … » pour chaque demande restée sans objet,
//!    puis PURGE le registre : comme dans SAS, la liste de sélection ODS
//!    OUTPUT ne survit pas au step de procédure qui la suit (`ods output
//!    close;` reste supporté et purge sans warning).
//!
//! Les procs à capture immédiate historique (MEANS `Summary`, TTEST `TTest`,
//! chemin M22.3) écrivent leur dataset eux-mêmes — leur NOTE et leur sortie
//! restent octet-identiques — et signalent la production via
//! [`Session::mark_ods_output_created`] pour ne pas déclencher le WARNING.
//!
//! ## Écarts SAS assumés (documentés)
//! - Le registre n'est purgé qu'aux frontières de PROC (pas aux frontières
//!   d'étape DATA) : les étapes DATA de sasrs ne produisent pas d'objets ODS.
//! - Le WARNING « was not created » est émis sur UNE ligne (le log sasrs ne
//!   réalise pas le repli de ligne du log SAS) ; le texte, la double espace
//!   après chaque point et la casse du nom tel que tapé sont fidèles.

use super::Session;
use crate::ast::DatasetRef;
use crate::dataset::SasDataset;
use crate::error::Result;
use crate::value::VarType;
use polars::prelude::*;
use std::collections::hash_map::Entry;

/// Une demande `ODS OUTPUT table=ds` enregistrée (valeur du registre
/// `Session::ods_output_map`, clé = nom de table ODS en UPPERCASE).
#[derive(Debug, Clone)]
pub struct OdsOutputRequest {
    /// Cible dataset (`libref.name`, WORK implicite).
    pub target: DatasetRef,
    /// Nom de table ODS tel que TAPÉ dans le statement (pour le WARNING SAS
    /// « Output '<nom>' was not created » qui préserve la casse d'origine).
    pub typed_name: String,
    /// Ordre d'enregistrement (NOTEs/WARNINGs déterministes, ordre statement).
    pub order: usize,
    /// La table a-t-elle été produite depuis l'enregistrement ? Posé par
    /// `flush_ods_output` (chemin générique) ou `mark_ods_output_created`
    /// (procs à capture immédiate M22.3).
    pub created: bool,
}

impl Session {
    /// M22.3 — enregistre des mappings `ODS OUTPUT table=ds`. Le nom de table
    /// ODS est stocké en UPPERCASE (matching insensible à la casse). Un mapping
    /// pour une table déjà enregistrée écrase l'ancien (sémantique SAS : le
    /// dernier `ODS OUTPUT` gagne).
    pub fn set_ods_output(&mut self, mappings: &[(String, DatasetRef)]) {
        for (table, dref) in mappings {
            let order = self.ods_output_seq;
            self.ods_output_seq += 1;
            self.ods_output_map.insert(
                table.to_ascii_uppercase(),
                OdsOutputRequest {
                    target: dref.clone(),
                    typed_name: table.clone(),
                    order,
                    created: false,
                },
            );
        }
    }

    /// M22.3 — purge tous les mappings `ODS OUTPUT` (équivalent
    /// `ODS OUTPUT CLOSE ;`), sans WARNING. Après cet appel la capture est de
    /// nouveau inactive et le listing redevient byte-identique au comportement
    /// par défaut.
    pub fn clear_ods_output(&mut self) {
        self.ods_output_map.clear();
        self.ods_output_pending.clear();
    }

    /// M22.3 — renvoie la cible dataset enregistrée pour une table ODS donnée
    /// (matching insensible à la casse), ou `None` si aucune capture n'est
    /// active pour cette table. Les procs consultent cette méthode AVANT
    /// d'écrire une table capturée.
    pub fn ods_output_target(&self, table: &str) -> Option<DatasetRef> {
        self.ods_output_map
            .get(&table.to_ascii_uppercase())
            .map(|req| req.target.clone())
    }

    /// M38.3 — la table ODS `table` est-elle demandée par un `ODS OUTPUT`
    /// actif ? Garde bon marché : les procs ne construisent la version typée
    /// d'une table que si sa capture est demandée.
    pub fn ods_output_active(&self, table: &str) -> bool {
        self.ods_output_map
            .contains_key(&table.to_ascii_uppercase())
    }

    /// M38.3 — marque la table ODS `table` comme produite, sans passer par le
    /// chemin générique. Réservé aux procs à capture immédiate (MEANS
    /// `Summary`, TTEST `TTest`) qui écrivent leur dataset eux-mêmes (M22.3) :
    /// le WARNING de fin de step ne doit pas les concerner. No-op si la table
    /// n'est pas demandée.
    pub fn mark_ods_output_created(&mut self, table: &str) {
        if let Some(req) = self.ods_output_map.get_mut(&table.to_ascii_uppercase()) {
            req.created = true;
        }
    }

    /// M38.3 — accumule une tranche de la table ODS `table` (chemin générique).
    /// No-op si la table n'est pas demandée par un `ODS OUTPUT` actif — un
    /// proc peut donc appeler sans garde, mais la garde
    /// [`ods_output_active`](Self::ods_output_active) évite de CONSTRUIRE la
    /// tranche pour rien. Les tranches successives (une par variable / requête
    /// TABLES / groupe BY) s'empilent par union diagonale des colonnes : les
    /// colonnes nouvelles s'ajoutent à droite (ordre de première apparition,
    /// comme SAS), les absentes se remplissent de missings.
    pub fn append_ods_output(&mut self, table: &str, part: SasDataset) -> Result<()> {
        let key = table.to_ascii_uppercase();
        if !self.ods_output_map.contains_key(&key) {
            return Ok(());
        }
        match self.ods_output_pending.entry(key) {
            Entry::Occupied(mut e) => union_append(e.get_mut(), part)?,
            Entry::Vacant(v) => {
                v.insert(part);
            }
        }
        Ok(())
    }

    /// M38.3 — écrit les tables accumulées comme datasets. Appelé par
    /// `procs::execute_proc` en fin de proc, AVANT la NOTE de timing (position
    /// SAS de la NOTE « The data set … »). Chaque écriture pose `_LAST_` et
    /// marque la demande comme produite. Ordre : ordre d'enregistrement des
    /// statements `ODS OUTPUT` (déterministe).
    pub fn flush_ods_output(&mut self) -> Result<()> {
        if self.ods_output_pending.is_empty() {
            return Ok(());
        }
        let mut keys: Vec<String> = self.ods_output_pending.keys().cloned().collect();
        keys.sort_by_key(|k| {
            self.ods_output_map
                .get(k)
                .map(|req| req.order)
                .unwrap_or(usize::MAX)
        });
        for key in keys {
            let out_ds = match self.ods_output_pending.remove(&key) {
                Some(ds) => ds,
                None => continue,
            };
            // Tranche orpheline (registre purgé entre-temps) : ignorée.
            let Some(req) = self.ods_output_map.get_mut(&key) else {
                continue;
            };
            req.created = true;
            let target = req.target.clone();
            let out_libref = target.libref_or_work();
            let out_table = target.name.to_uppercase();
            let display = format!("{out_libref}.{out_table}");
            let n_rows = out_ds.n_obs();
            let n_vars = out_ds.vars.len();
            self.libs.get(&out_libref)?.write(&out_table, &out_ds)?;
            self.last_dataset = Some(display.clone());
            self.log.note(&format!(
                "The data set {display} has {n_rows} observations and {n_vars} variables."
            ));
        }
        Ok(())
    }

    /// M38.3 — frontière de step de procédure (executor, APRÈS la NOTE de
    /// timing — position SAS du WARNING) : émet le WARNING SAS pour chaque
    /// demande `ODS OUTPUT` restée sans objet, puis purge le registre.
    /// Sémantique SAS : la liste de sélection ODS OUTPUT ne survit pas au step
    /// de procédure qui la suit.
    pub fn ods_output_step_boundary(&mut self) {
        if self.ods_output_map.is_empty() && self.ods_output_pending.is_empty() {
            return;
        }
        let mut missed: Vec<(usize, String)> = self
            .ods_output_map
            .values()
            .filter(|req| !req.created)
            .map(|req| (req.order, req.typed_name.clone()))
            .collect();
        missed.sort();
        for (_, name) in missed {
            // Texte SAS 9.4 verbatim (double espace après chaque point) ; une
            // seule ligne — voir l'écart documenté en tête de module.
            self.log.warning(&format!(
                "Output '{name}' was not created.  Make sure that the output object name, \
                 label, or path is spelled correctly.  Also, verify that the appropriate \
                 procedure options are used to produce the requested output object.  For \
                 example, verify that the NOPRINT option is not used."
            ));
        }
        self.ods_output_map.clear();
        self.ods_output_pending.clear();
    }
}

/// Empile `part` sous `acc` avec union diagonale des colonnes (matching de nom
/// insensible à la casse, comme les noms de variables SAS) :
/// - une colonne de `part` absente d'`acc` est ajoutée à droite d'`acc`,
///   remplie de missings sur les lignes déjà accumulées ;
/// - une colonne d'`acc` absente de `part` est remplie de missings sur les
///   nouvelles lignes ;
/// - pour une colonne caractère partagée, la longueur SAS retenue est le max
///   des deux tranches.
fn union_append(acc: &mut SasDataset, part: SasDataset) -> Result<()> {
    let acc_height = acc.df.height();
    // 1. Colonnes de `part` inconnues d'`acc` → ajout à droite, null-filled.
    for (j, vm) in part.vars.iter().enumerate() {
        if !acc
            .vars
            .iter()
            .any(|a| a.name.eq_ignore_ascii_case(&vm.name))
        {
            let dtype = part.df.get_columns()[j].dtype().clone();
            let filler = Series::full_null(vm.name.as_str().into(), acc_height, &dtype);
            acc.df.with_column(filler)?;
            acc.vars.push(vm.clone());
        }
    }
    // 2. Tranche alignée sur le schéma (ordonné) d'`acc`.
    let part_height = part.df.height();
    let mut cols: Vec<Column> = Vec::with_capacity(acc.vars.len());
    for (i, vm) in acc.vars.iter_mut().enumerate() {
        match part
            .vars
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(&vm.name))
        {
            Some(j) => {
                let mut col = part.df.get_columns()[j].clone();
                // Unifie la casse du nom sur la première apparition.
                col.rename(vm.name.as_str().into());
                cols.push(col);
                if vm.ty == VarType::Char {
                    vm.length = vm.length.max(part.vars[j].length);
                }
            }
            None => {
                let dtype = acc.df.get_columns()[i].dtype().clone();
                cols.push(Series::full_null(vm.name.as_str().into(), part_height, &dtype).into());
            }
        }
    }
    let aligned = DataFrame::new(cols)?;
    acc.df.vstack_mut(&aligned)?;
    Ok(())
}
