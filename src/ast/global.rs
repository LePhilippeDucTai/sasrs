use super::*;

#[derive(Debug, Clone, PartialEq)]
pub enum GlobalStmt {
    Libname {
        libref: String,
        engine: Option<String>,
        path: String,
    },
    LibnameClear {
        libref: String,
    },
    /// M35.2 — `FILENAME fileref 'chemin';` (ou chemin nu) : enregistre un
    /// fileref → chemin pour `%include fileref;`. Forme minimale : seuls le nom
    /// du fileref et un unique chemin (fichier ou répertoire) sont portés. Les
    /// variantes device/options (`TEMP`, pipes, URL, listes) sont
    /// acceptées-et-ignorées au parse (cf. `device`), sans enregistrement.
    Filename {
        fileref: String,
        /// Chemin (fichier ou répertoire) à enregistrer. `None` si la forme
        /// reconnue est un device/options sans chemin exploitable (ignorée).
        path: Option<String>,
        /// Mot-clé device éventuel (`TEMP`, `PIPE`, `URL`, …) en MAJUSCULES,
        /// pour la NOTE d'ignorance. `None` pour la forme `ref 'chemin';`.
        device: Option<String>,
    },
    Title {
        n: u8,
        text: Option<String>,
    },
    /// `FOOTNOTEn 'texte';` (N=1..9) — bas-de-page multi-niveaux. Même
    /// sémantique d'effacement que TITLE (gérée dans l'exécuteur). `text = None`
    /// pour la forme sans texte (`FOOTNOTEn;`) qui efface.
    Footnote {
        n: u8,
        text: Option<String>,
    },
    /// Parsed OPTIONS name=value / flag list; unknown options warn.
    Options(Vec<(String, Option<String>)>),
    /// M22.2 — statement `ODS` : ouvre/ferme une destination de sortie.
    ///
    /// Schéma large v1 :
    /// - `ODS LISTING ;`  → ouvre le listing texte (défaut)
    /// - `ODS HTML ;`     → ouvre la destination HTML
    /// - `ODS RTF|PDF|EXCEL ;` → stubs (parse no-op, rendu différé M23)
    /// - `ODS CLOSE <dest> ;` / `ODS <dest> CLOSE ;` → ferme la destination
    ///
    /// `file`/`style` (FILE=/STYLE=) sont parsés mais seulement stockés ici ;
    /// leur usage réel arrive en M22.4+ (écriture fichier / styles).
    Ods {
        /// Nom de destination en minuscules ("listing", "html", "rtf", …).
        destination: String,
        action: OdsAction,
        /// Option FILE= (chemin de sortie). Différé M22.4+.
        file: Option<String>,
        /// Option STYLE= (nom de style). Différé M22.4+.
        style: Option<String>,
    },
    /// M22.2 — options globales ODS portées par `OPTIONS` SAS classiques
    /// (CENTER/NOCENTER, DATE/NODATE, NUMBER/NONUMBER). Stockées sur la session
    /// et exposées aux destinations.
    OdsOptions {
        /// `false` = centré (défaut SAS), `true` = NOCENTER.
        nocenter: bool,
        /// `true` = afficher la date (défaut), `false` = NODATE.
        date: bool,
        /// `true` = numéro de page (défaut), `false` = NONUMBER.
        number: bool,
    },
    /// M22.3 — statement `ODS OUTPUT` : capture la sortie tabulaire d'un PROC
    /// sous forme de dataset SAS au lieu (ou en plus) du listing.
    ///
    /// Formes reconnues :
    /// - `ODS OUTPUT table=ds [table2=ds2 ...] ;` → enregistre des mappings
    ///   (nom de table ODS → cible dataset). Le nom de table ODS est
    ///   insensible à la casse (stocké UPPERCASE côté session).
    /// - `ODS OUTPUT CLOSE ;` → vide tous les mappings (désactive la capture).
    OdsOutput {
        /// Paires (nom-de-table-ODS, cible dataset). Vide si `close == true`.
        mappings: Vec<(String, DatasetRef)>,
        /// `true` pour `ODS OUTPUT CLOSE ;` (purge des mappings).
        close: bool,
    },
    /// M29.1 — statement `ODS GRAPHICS` : active/désactive la génération d'images
    /// et configure les paramètres de rendu.
    ///
    /// Formes reconnues :
    /// - `ODS GRAPHICS ON ;`
    /// - `ODS GRAPHICS OFF ;`
    /// - `ODS GRAPHICS ON / WIDTH=nnn HEIGHT=nnn IMAGEFMT=(PNG|SVG) ;`
    /// - `ODS GRAPHICS / IMAGENAME="fig" RESET=index ;` (sans ON/OFF : MAJ config)
    ///
    /// IMPORTANT : ces options sont PAR-STATEMENT. La NOTE de log émise à
    /// l'exécution reflète UNIQUEMENT ce que CE statement a porté (les
    /// dimensions ne sont affichées que si elles ont été fournies ici), même si
    /// la session conserve des valeurs antérieures. Voir `executor::exec_ods_graphics`.
    OdsGraphics(OdsGraphicsStmt),
}

/// M29.1 — bascule ON/OFF d'un statement `ODS GRAPHICS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OdsGraphicsToggle {
    /// `ODS GRAPHICS ON`.
    On,
    /// `ODS GRAPHICS OFF`.
    Off,
    /// `ODS GRAPHICS / ...` sans ON/OFF : met seulement à jour la config.
    None,
}

/// M29.1 — options portées par UN statement `ODS GRAPHICS` (per-statement).
///
/// Tous les champs `Option<…>` sont `None` si l'option n'a pas été fournie
/// dans CE statement. C'est cette absence/présence qui pilote la NOTE de log.
#[derive(Debug, Clone, PartialEq)]
pub struct OdsGraphicsStmt {
    /// ON / OFF / (aucun).
    pub toggle: OdsGraphicsToggle,
    /// `WIDTH=` fourni dans ce statement (pixels).
    pub width: Option<u32>,
    /// `HEIGHT=` fourni dans ce statement (pixels).
    pub height: Option<u32>,
    /// `IMAGEFMT=` fourni dans ce statement.
    pub imagefmt: Option<crate::ods_graphics::ImageFmt>,
    /// `IMAGENAME=` fourni dans ce statement (préfixe de nommage).
    pub imagename: Option<String>,
}

/// M22.2 — action d'un statement `ODS` sur une destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OdsAction {
    /// Ouvre la destination (forme par défaut : `ODS HTML ;`).
    Open,
    /// Ferme la destination (`ODS HTML CLOSE ;` / `ODS CLOSE ...`).
    Close,
    /// `ODS <dest> SELECT ...` — différé M22.3.
    Select,
    /// `ODS <dest> EXCLUDE ...` — différé M22.3.
    Exclude,
}
