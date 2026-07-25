//! ODS routing layer (M22.1) — `OutputDestination` trait + destinations.
//!
//! M22 introduit la couche de routage ODS (Output Delivery System). Toute
//! sortie de résultats (titres de page, tables de procs, lignes de texte) passe
//! désormais par le trait [`OutputDestination`], ce qui permet de router la même
//! sortie vers plusieurs destinations (listing texte, HTML, RTF, PDF, Excel).
//!
//! ## Périmètre M22.1
//! - [`OutputDestination`] : trait de destination (page_header / write_table /
//!   write_line / blank + accès au titre et à la LINESIZE, drain final).
//! - [`TextListing`] : destination texte par défaut. C'est un mince *adaptateur*
//!   au-dessus de [`crate::listing::ListingWriter`] (le rendu texte prouvé
//!   octet-identique de M1–M21) — AUCUNE logique de mise en forme n'est
//!   réécrite ici, on délègue verbatim. Invariant CRITIQUE : le listing texte
//!   par défaut reste **octet-identique** aux snapshots m1–m21.
//! - [`HtmlDestination`], [`RtfDestination`], [`PdfDestination`],
//!   [`ExcelDestination`] : stubs no-op qui implémentent le trait, remplis en
//!   M22.4 / M23.
//!
//! ## Choix de signature de `write_table`
//! Le plan M22 esquisse `write_table(df: &DataFrame, vars: &[VarMeta])`. En
//! pratique les ~31 sites d'appel existants (PROC PRINT/MEANS/CORR/REPORT/…)
//! fournissent déjà des cellules **pré-formatées** (en-têtes, alignements,
//! lignes de chaînes) — c'est ce contrat, prouvé octet-identique, que le trait
//! expose ici. La variante DataFrame pourra être ajoutée plus tard pour les
//! destinations riches (HTML/Excel) sans casser ce chemin. Réexporté pour les
//! destinations : [`Align`].


mod text;
mod html;
mod rtf;
mod excel;
mod pdf;

pub use excel::ExcelDestination;
pub use html::HtmlDestination;
pub use pdf::PdfDestination;
pub use rtf::RtfDestination;
pub use text::TextListing;


pub use crate::listing::Align;

use crate::listing::ListingWriter;

/// Une destination de sortie ODS. Reçoit les résultats déjà mis en forme
/// (cellules de table, lignes de texte) et les matérialise selon son format.
///
/// Le listing texte ([`TextListing`]) est la destination par défaut ; les
/// destinations HTML/RTF/PDF/Excel partagent le même trait et seront branchées
/// par le statement `ODS` (M22.2+).
pub trait OutputDestination {
    /// En-tête de page au début de la sortie d'un proc (titre centré + ligne
    /// blanche). Insère une ligne blanche de séparation si du contenu a déjà
    /// été écrit.
    fn page_header(&mut self);

    /// Rend une table de résultats : en-têtes, alignement par colonne, lignes
    /// de cellules (toutes pré-formatées en chaînes par l'appelant).
    fn write_table(&mut self, headers: &[String], aligns: &[Align], rows: &[Vec<String>]);

    /// Variante PROC PRINT (M33.6) : double-interligne optionnel et ligne de
    /// totaux optionnelle. L'implémentation par défaut (destinations ODS et
    /// stubs) ignore ces extensions et rend la table normalement, suivie de la
    /// ligne de totaux si présente (sans alignement colonne). Seule la
    /// destination texte ([`TextListing`]) surcharge cette méthode pour aligner
    /// les totaux sous leurs colonnes — l'invariant byte-identique du listing
    /// par défaut est ainsi préservé.
    fn write_table_ext(
        &mut self,
        headers: &[String],
        aligns: &[Align],
        rows: &[Vec<String>],
        _double: bool,
        totals: Option<&Vec<String>>,
    ) {
        self.write_table(headers, aligns, rows);
        if let Some(t) = totals {
            self.write_line(&t.join("  "));
        }
    }

    /// Écrit une ligne de texte libre (justifiée à gauche, colonne 0).
    fn write_line(&mut self, line: &str);

    /// Émet une ligne vide.
    fn blank(&mut self);

    /// Pose les titres actifs (TITLE1..TITLE9), dans l'ordre des niveaux, gaps
    /// retirés. Vide = défaut « The SAS System ».
    fn set_titles(&mut self, titles: &[String]);

    /// Pose les footnotes actives (FOOTNOTE1..FOOTNOTE9), dans l'ordre des
    /// niveaux, gaps retirés. Vide = aucune footnote. Implémentation par défaut
    /// no-op (destinations qui ne rendent pas encore les footnotes).
    fn set_footnotes(&mut self, _footnotes: &[String]) {}

    /// Pose le titre courant (TITLE1). `None` = défaut « The SAS System ».
    /// Compatibilité : délègue à [`set_titles`](Self::set_titles).
    fn set_title(&mut self, title: Option<String>) {
        match title {
            None => self.set_titles(&[]),
            Some(t) => self.set_titles(std::slice::from_ref(&t)),
        }
    }

    /// Pose la LINESIZE (LS=) servant à centrer la sortie.
    fn set_ls(&mut self, ls: usize);

    /// Lit la LINESIZE courante (certains procs en ont besoin pour leur propre
    /// mise en page).
    fn ls(&self) -> usize;

    /// Draine la sortie accumulée sous forme de chaîne, laissant la destination
    /// vide. Pour le listing texte c'est le contenu rendu ; pour les
    /// destinations fichier (à venir) ce sera typiquement vide (déjà écrit sur
    /// disque). Remplace l'ancien `ListingWriter::into_string` (qui consommait
    /// `self`, impossible derrière un `Box<dyn …>`).
    fn into_string(&mut self) -> String;

    /// Finalise la destination : si elle cible un fichier, renvoie
    /// `Some((path, contenu))` pour que l'appelant écrive le fichier sur disque.
    /// La valeur par défaut (listing texte et stubs) renvoie `None`.
    fn finalize(&mut self) -> Option<(std::path::PathBuf, String)> {
        None
    }

    /// Finalise pour les formats binaires (Excel, PDF) : retourne les octets
    /// du fichier à écrire sur disque. Défaut : None (format texte ou pas de
    /// fichier). Les destinations binaires DOIVENT implémenter cette méthode
    /// plutôt que `finalize()`.
    fn finalize_to_bytes(&mut self) -> Option<(std::path::PathBuf, Vec<u8>)> {
        None
    }

    /// Étiquette du type de destination pour les messages de log (NOTE "Writing
    /// <label> file: …"). Chaque destination surcharge cette méthode.
    fn dest_type_label(&self) -> &'static str {
        "HTML Body"
    }
}

#[cfg(test)]
mod tests;
