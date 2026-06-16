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

    /// Écrit une ligne de texte libre (justifiée à gauche, colonne 0).
    fn write_line(&mut self, line: &str);

    /// Émet une ligne vide.
    fn blank(&mut self);

    /// Pose le titre courant (TITLE1). `None` = défaut « The SAS System ».
    fn set_title(&mut self, title: Option<String>);

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
}

/// Destination texte par défaut : adaptateur au-dessus de [`ListingWriter`].
///
/// Délègue verbatim au rendu texte historique de M1–M21 ⇒ sortie
/// octet-identique. Aucune mise en forme n'est dupliquée ici.
pub struct TextListing {
    inner: ListingWriter,
}

impl TextListing {
    /// Crée une destination texte avec la LINESIZE donnée.
    pub fn new(ls: usize) -> Self {
        TextListing {
            inner: ListingWriter::new(ls),
        }
    }
}

impl OutputDestination for TextListing {
    fn page_header(&mut self) {
        self.inner.page_header();
    }

    fn write_table(&mut self, headers: &[String], aligns: &[Align], rows: &[Vec<String>]) {
        self.inner.write_table(headers, aligns, rows);
    }

    fn write_line(&mut self, line: &str) {
        self.inner.write_line(line);
    }

    fn blank(&mut self) {
        self.inner.blank();
    }

    fn set_title(&mut self, title: Option<String>) {
        self.inner.title = title;
    }

    fn set_ls(&mut self, ls: usize) {
        self.inner.ls = ls;
    }

    fn ls(&self) -> usize {
        self.inner.ls
    }

    fn into_string(&mut self) -> String {
        // Remplace le writer interne par un writer vide de même LINESIZE et
        // rend la chaîne accumulée. Équivalent à l'ancien `into_string` qui
        // consommait `self`, mais utilisable derrière un trait object.
        let ls = self.inner.ls;
        let title = self.inner.title.clone();
        let mut fresh = ListingWriter::new(ls);
        fresh.title = title;
        let old = std::mem::replace(&mut self.inner, fresh);
        old.into_string()
    }
}

/// Squelette commun des destinations no-op (HTML/RTF/PDF/Excel) tant que leur
/// rendu n'est pas implémenté (M22.4 / M23). Conserve néanmoins titre et
/// LINESIZE pour que le statement `ODS` puisse les configurer sans surprise.
macro_rules! stub_destination {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        pub struct $name {
            title: Option<String>,
            ls: usize,
        }

        impl $name {
            /// Crée la destination (no-op pour l'instant).
            pub fn new(ls: usize) -> Self {
                $name { title: None, ls }
            }
        }

        impl OutputDestination for $name {
            fn page_header(&mut self) {
                // no-op : rendu différé (M22.4 / M23).
            }

            fn write_table(
                &mut self,
                _headers: &[String],
                _aligns: &[Align],
                _rows: &[Vec<String>],
            ) {
                // no-op : rendu différé (M22.4 / M23).
            }

            fn write_line(&mut self, _line: &str) {
                // no-op : rendu différé (M22.4 / M23).
            }

            fn blank(&mut self) {
                // no-op : rendu différé (M22.4 / M23).
            }

            fn set_title(&mut self, title: Option<String>) {
                self.title = title;
            }

            fn set_ls(&mut self, ls: usize) {
                self.ls = ls;
            }

            fn ls(&self) -> usize {
                self.ls
            }

            fn into_string(&mut self) -> String {
                // Aucune sortie en mémoire pour l'instant.
                String::new()
            }
        }
    };
}

stub_destination!(
    HtmlDestination,
    "Destination HTML (tables CSS, fichier `.html`). Stub no-op ; remplie en M22.4."
);
stub_destination!(
    RtfDestination,
    "Destination RTF (séquences de contrôle Word). Stub no-op ; remplie en M23.1."
);
stub_destination!(
    PdfDestination,
    "Destination PDF (pagination, tables). Stub no-op ; remplie en M23.2 (feature `pdf`)."
);
stub_destination!(
    ExcelDestination,
    "Destination Excel (`ODS EXCEL`, feuilles par proc). Stub no-op ; remplie en M23.3."
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_listing_is_text_listing() {
        // Le listing par défaut d'une session est une `TextListing` derrière le
        // trait object. On vérifie qu'on peut écrire au travers du trait.
        let tmp = std::env::temp_dir();
        let session = crate::session::Session::new(None, tmp, true).expect("session");
        let dest: &dyn OutputDestination = session.listing.as_ref();
        // LINESIZE par défaut = 96 (SasOptions::default()).
        assert_eq!(dest.ls(), 96);
    }

    #[test]
    fn write_line_renders_text() {
        let mut d = TextListing::new(96);
        d.write_line("test");
        let out = d.into_string();
        assert_eq!(out, "test\n");
    }

    #[test]
    fn page_header_default_title_centered() {
        let mut d = TextListing::new(40);
        d.page_header();
        let out = d.into_string();
        assert!(out.contains("The SAS System"), "out: {out:?}");
        // Centré dans LS=40 : padding gauche de (40-14)/2 = 13 espaces.
        assert!(out.starts_with("             The SAS System"), "out: {out:?}");
    }

    #[test]
    fn set_title_overrides_default() {
        let mut d = TextListing::new(40);
        d.set_title(Some("Mon Titre".to_string()));
        d.page_header();
        let out = d.into_string();
        assert!(out.contains("Mon Titre"), "out: {out:?}");
        assert!(!out.contains("The SAS System"), "out: {out:?}");
    }

    #[test]
    fn write_table_matches_listing_writer() {
        // Le rendu de table passe verbatim par ListingWriter ⇒ octet-identique.
        let mut d = TextListing::new(40);
        d.page_header();
        d.write_table(
            &["Obs".into(), "x".into()],
            &[Align::Right, Align::Right],
            &[
                vec!["1".into(), "10".into()],
                vec!["2".into(), "200".into()],
            ],
        );
        let via_trait = d.into_string();

        let mut l = ListingWriter::new(40);
        l.page_header();
        l.write_table(
            &["Obs".into(), "x".into()],
            &[Align::Right, Align::Right],
            &[
                vec!["1".into(), "10".into()],
                vec!["2".into(), "200".into()],
            ],
        );
        let direct = l.into_string();

        assert_eq!(via_trait, direct);
    }

    #[test]
    fn blank_emits_empty_line() {
        let mut d = TextListing::new(40);
        d.write_line("a");
        d.blank();
        d.write_line("b");
        assert_eq!(d.into_string(), "a\n\nb\n");
    }

    #[test]
    fn into_string_leaves_destination_empty() {
        // Deux drains successifs ne dupliquent pas le contenu.
        let mut d = TextListing::new(40);
        d.write_line("once");
        assert_eq!(d.into_string(), "once\n");
        assert_eq!(d.into_string(), "");
    }

    #[test]
    fn html_stub_is_noop() {
        let mut h = HtmlDestination::new(96);
        h.page_header();
        h.write_line("ignored");
        h.write_table(&["a".into()], &[Align::Left], &[vec!["1".into()]]);
        h.blank();
        assert_eq!(h.into_string(), "");
        assert_eq!(h.ls(), 96);
    }

    #[test]
    fn stub_destinations_implement_trait() {
        // Toutes les destinations stub sont utilisables comme trait objects.
        let dests: Vec<Box<dyn OutputDestination>> = vec![
            Box::new(HtmlDestination::new(80)),
            Box::new(RtfDestination::new(80)),
            Box::new(PdfDestination::new(80)),
            Box::new(ExcelDestination::new(80)),
        ];
        for d in dests {
            assert_eq!(d.ls(), 80);
        }
    }
}
