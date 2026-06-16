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

    /// Finalise la destination : si elle cible un fichier, renvoie
    /// `Some((path, contenu))` pour que l'appelant écrive le fichier sur disque.
    /// La valeur par défaut (listing texte et stubs) renvoie `None`.
    fn finalize(&mut self) -> Option<(std::path::PathBuf, String)> {
        None
    }
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

// ---------------------------------------------------------------------------
// HtmlDestination — M22.4 : destination HTML réelle (tables CSS + fichier)
// ---------------------------------------------------------------------------

/// Destination HTML (tables CSS, fichier `.html`).
///
/// Génère du HTML valide avec une feuille de style CSS embarquée. La sortie
/// est accumulée en mémoire dans `buf` puis drainée par [`into_string`] soit
/// explicitement (via le trait [`OutputDestination`]), soit implicitement lors
/// d'un [`close_destination`] via [`finalize`].
///
/// Cycle de vie :
/// - `new(ls)` : pas de fichier cible → `finalize()` renvoie `None`.
/// - `with_file(ls, path)` : fichier cible → `finalize()` renvoie
///   `Some((path, html_complet))`.
pub struct HtmlDestination {
    buf: String,
    title: Option<String>,
    ls: usize,
    file: Option<std::path::PathBuf>,
    wrote_anything: bool,
}

impl HtmlDestination {
    /// Crée la destination HTML sans fichier cible (sortie en mémoire seulement).
    pub fn new(ls: usize) -> Self {
        HtmlDestination {
            buf: String::new(),
            title: None,
            ls,
            file: None,
            wrote_anything: false,
        }
    }

    /// Crée la destination HTML avec un fichier cible.
    pub fn with_file(ls: usize, file: std::path::PathBuf) -> Self {
        HtmlDestination {
            buf: String::new(),
            title: None,
            ls,
            file: Some(file),
            wrote_anything: false,
        }
    }

    /// Échappe les caractères HTML spéciaux.
    ///
    /// L'ordre est critique : `&` doit être traité EN PREMIER pour éviter de
    /// ré-échapper les séquences `&amp;` produites ensuite.
    fn html_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }
}

impl OutputDestination for HtmlDestination {
    fn page_header(&mut self) {
        let title = self
            .title
            .clone()
            .unwrap_or_else(|| "The SAS System".to_string());
        self.buf.push_str(&format!(
            "<h1 class=\"systitle\">{}</h1>\n",
            Self::html_escape(&title)
        ));
        self.wrote_anything = true;
    }

    fn write_table(&mut self, headers: &[String], aligns: &[Align], rows: &[Vec<String>]) {
        self.buf.push_str("<table class=\"sas\">\n<thead>\n<tr>");
        for (i, h) in headers.iter().enumerate() {
            let align_attr = match aligns.get(i).copied().unwrap_or(Align::Left) {
                Align::Right => " style=\"text-align:right\"",
                Align::Left => "",
            };
            self.buf.push_str(&format!(
                "<th{attr}>{text}</th>",
                attr = align_attr,
                text = Self::html_escape(h)
            ));
        }
        self.buf.push_str("</tr>\n</thead>\n<tbody>\n");
        for row in rows {
            self.buf.push_str("<tr>");
            for (i, cell) in row.iter().enumerate() {
                let align_attr = match aligns.get(i).copied().unwrap_or(Align::Left) {
                    Align::Right => " style=\"text-align:right\"",
                    Align::Left => "",
                };
                self.buf.push_str(&format!(
                    "<td{attr}>{text}</td>",
                    attr = align_attr,
                    text = Self::html_escape(cell)
                ));
            }
            self.buf.push_str("</tr>\n");
        }
        self.buf.push_str("</tbody>\n</table>\n");
        self.wrote_anything = true;
    }

    fn write_line(&mut self, line: &str) {
        self.buf
            .push_str(&format!("<p>{}</p>\n", Self::html_escape(line)));
        self.wrote_anything = true;
    }

    fn blank(&mut self) {
        // no-op : les paragraphes HTML séparent naturellement le contenu.
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

    /// Draine la sortie accumulée sous forme de document HTML complet.
    ///
    /// Si `buf` est vide (rien n'a été écrit), renvoie une chaîne vide
    /// (comportement idempotent identique à `TextListing::into_string`).
    /// Après cet appel `buf` est vide : un second appel renvoie `""`.
    fn into_string(&mut self) -> String {
        if self.buf.is_empty() {
            return String::new();
        }
        let body = std::mem::take(&mut self.buf);
        self.wrote_anything = false;
        format!(
            "<!DOCTYPE html>\n\
             <html>\n\
             <head>\n\
             <meta charset=\"utf-8\">\n\
             <style>\
table.sas{{border-collapse:collapse;}} \
table.sas th,table.sas td{{border:1px solid #888;padding:4px;}}\
</style>\n\
             </head>\n\
             <body>\n\
             {body}\
             </body>\n\
             </html>\n"
        )
    }

    /// Finalise la destination : si un fichier cible a été configuré, renvoie
    /// `Some((path, html_complet))` pour que l'appelant l'écrive sur disque.
    /// Sinon renvoie `None`.
    fn finalize(&mut self) -> Option<(std::path::PathBuf, String)> {
        let path = self.file.clone()?;
        let html = self.into_string();
        if html.is_empty() {
            // Rien à écrire (destination ouverte mais inutilisée).
            None
        } else {
            Some((path, html))
        }
    }
}

// ---------------------------------------------------------------------------
// Squelette commun des destinations no-op (RTF/PDF/Excel)
// ---------------------------------------------------------------------------

/// Squelette commun des destinations no-op (RTF/PDF/Excel) tant que leur
/// rendu n'est pas implémenté (M23). Conserve néanmoins titre et
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
                // no-op : rendu différé (M23).
            }

            fn write_table(
                &mut self,
                _headers: &[String],
                _aligns: &[Align],
                _rows: &[Vec<String>],
            ) {
                // no-op : rendu différé (M23).
            }

            fn write_line(&mut self, _line: &str) {
                // no-op : rendu différé (M23).
            }

            fn blank(&mut self) {
                // no-op : rendu différé (M23).
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

    // html_stub_is_noop retiré : HtmlDestination est désormais une vraie
    // destination (M22.4), remplacé par les tests html_* ci-dessous.

    #[test]
    fn stub_destinations_implement_trait() {
        // Les destinations stub RTF/PDF/Excel sont utilisables comme trait objects.
        let dests: Vec<Box<dyn OutputDestination>> = vec![
            Box::new(RtfDestination::new(80)),
            Box::new(PdfDestination::new(80)),
            Box::new(ExcelDestination::new(80)),
        ];
        for d in dests {
            assert_eq!(d.ls(), 80);
        }
    }

    // --- Tests M22.4 : HtmlDestination réelle ---

    #[test]
    fn html_table_renders_escaped_cells() {
        let mut h = HtmlDestination::new(96);
        h.write_table(
            &["Name".into(), "Value <x>".into()],
            &[Align::Left, Align::Right],
            &[
                vec!["a & b".into(), "42".into()],
                vec!["<tag>".into(), "99".into()],
            ],
        );
        let out = h.into_string();
        // Présence de la structure de table.
        assert!(out.contains("<table"), "pas de <table : {out}");
        assert!(out.contains("</table>"), "pas de </table> : {out}");
        // Échappement dans en-tête.
        assert!(out.contains("Value &lt;x&gt;"), "échappement header raté : {out}");
        // Échappement dans cellule.
        assert!(out.contains("a &amp; b"), "échappement & raté : {out}");
        assert!(out.contains("&lt;tag&gt;"), "échappement < raté : {out}");
        // Alignement droite sur la 2ᵉ colonne.
        assert!(out.contains("text-align:right"), "alignement right manquant : {out}");
    }

    #[test]
    fn html_into_string_wraps_document() {
        let mut h = HtmlDestination::new(96);
        h.write_line("hello");
        let out = h.into_string();
        // Structure HTML obligatoire.
        assert!(out.contains("<!DOCTYPE html>"), "DOCTYPE manquant : {out}");
        assert!(out.contains("<style"), "style manquant : {out}");
        assert!(out.contains("<body>"), "<body> manquant : {out}");
        assert!(out.contains("</body>"), "</body> manquant : {out}");
        assert!(out.contains("<p>hello</p>"), "<p> manquant : {out}");
        // Second drain → chaîne vide (idempotent).
        assert_eq!(h.into_string(), "", "second drain non vide");
    }

    #[test]
    fn html_without_file_finalize_none() {
        let mut h = HtmlDestination::new(96);
        h.write_line("test");
        // Sans fichier cible, finalize() renvoie None.
        assert!(h.finalize().is_none());
    }

    #[test]
    fn html_empty_into_string_is_empty() {
        // Rien d'écrit → into_string() renvoie "" (pas de document vide).
        let mut h = HtmlDestination::new(96);
        assert_eq!(h.into_string(), "");
    }

    #[test]
    fn html_page_header_uses_title() {
        let mut h = HtmlDestination::new(96);
        h.set_title(Some("Mon Rapport".to_string()));
        h.page_header();
        let out = h.into_string();
        assert!(out.contains("Mon Rapport"), "titre absent : {out}");
        assert!(out.contains("class=\"systitle\""), "classe systitle absente : {out}");
    }

    #[test]
    fn html_page_header_default_title() {
        let mut h = HtmlDestination::new(96);
        h.page_header();
        let out = h.into_string();
        assert!(out.contains("The SAS System"), "titre par défaut absent : {out}");
    }

    #[test]
    fn html_ls_accessor() {
        let h = HtmlDestination::new(80);
        assert_eq!(h.ls(), 80);
    }

    #[test]
    fn html_with_file_finalize_some() {
        let tmp = std::env::temp_dir().join("test_html_finalize.html");
        let mut h = HtmlDestination::with_file(96, tmp.clone());
        h.write_line("content");
        let result = h.finalize();
        assert!(result.is_some(), "finalize devrait renvoyer Some");
        let (path, html) = result.unwrap();
        assert_eq!(path, tmp);
        assert!(html.contains("<!DOCTYPE html>"), "HTML complet attendu");
        assert!(html.contains("<p>content</p>"), "contenu attendu");
        // Après finalize, buf est vide.
        assert_eq!(h.into_string(), "", "buf doit être vide après finalize");
    }
}
