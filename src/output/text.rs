use super::*;

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
    fn page_state(&self) -> &PageState {
        &self.inner.page
    }

    fn page_state_mut(&mut self) -> &mut PageState {
        &mut self.inner.page
    }

    fn page_header(&mut self) {
        self.inner.page_header();
    }

    fn write_table(&mut self, headers: &[String], aligns: &[Align], rows: &[Vec<String>]) {
        self.inner.write_table(headers, aligns, rows);
    }

    fn write_table_ext(
        &mut self,
        headers: &[String],
        aligns: &[Align],
        rows: &[Vec<String>],
        double: bool,
        totals: Option<&Vec<String>>,
    ) {
        self.inner
            .write_table_ext(headers, aligns, rows, double, totals);
    }

    fn write_line(&mut self, line: &str) {
        self.inner.write_line(line);
    }

    fn blank(&mut self) {
        self.inner.blank();
    }

    fn take_string(&mut self) -> String {
        // Remplace le writer interne par un writer vide de même LINESIZE et
        // rend la chaîne accumulée. Équivalent à l'ancien `into_string` qui
        // consommait `self`, mais utilisable derrière un trait object. Les
        // titres/footnotes actifs survivent au drain (un proc qui suit les
        // réutilise tant qu'aucun nouveau statement TITLE/FOOTNOTE ne les change).
        let page = self.inner.page.clone();
        let mut fresh = ListingWriter::new(page.ls);
        fresh.page = page;
        let old = std::mem::replace(&mut self.inner, fresh);
        old.into_string()
    }
}
