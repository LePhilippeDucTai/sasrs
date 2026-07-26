/// Source d'un statement INFILE (M14) : un fichier sur disque (chemin
/// littéral) ou les lignes inline d'un bloc DATALINES/CARDS.
#[derive(Debug, Clone, PartialEq)]
pub enum InfileSource {
    /// `infile 'chemin';` — lecture d'un fichier texte.
    Path(String),
    /// `infile datalines;` / `infile cards;` — la source est le bloc
    /// DATALINES inline de l'étape.
    Datalines,
}

/// Options d'un statement INFILE (M14). Tous les champs sont optionnels ;
/// `None`/`false` = option absente.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InfileOptions {
    /// `DELIMITER=`/`DLM=` : caractère(s) séparateur(s) de la lecture en
    /// liste. `None` = défaut (l'espace). Une chaîne peut porter plusieurs
    /// délimiteurs (chacun de ses caractères en est un).
    pub delimiter: Option<String>,
    /// `DSD` : délimiteur-sensible — deux délimiteurs consécutifs encadrent
    /// une valeur manquante, les guillemets protègent les délimiteurs, le
    /// délimiteur par défaut devient la virgule.
    pub dsd: bool,
    /// `FIRSTOBS=` : numéro (1-based) de la première ligne lue.
    pub firstobs: Option<usize>,
    /// `OBS=` : numéro (1-based) de la dernière ligne lue.
    pub obs: Option<usize>,
    /// `MISSOVER` : un INPUT qui dépasse la fin de ligne laisse les
    /// variables restantes à missing (pas de passage à la ligne suivante).
    pub missover: bool,
    /// `TRUNCOVER` : comme MISSOVER, mais une valeur partielle en fin de
    /// ligne est tout de même lue.
    pub truncover: bool,
    /// `STOPOVER` : un INPUT qui dépasse la fin de ligne est une erreur qui
    /// arrête l'étape.
    pub stopover: bool,
    /// `LRECL=` : longueur d'enregistrement (parsée et conservée ;
    /// no-op fonctionnel — toutes les lignes sont lues entières).
    pub lrecl: Option<usize>,
}

/// Un item du statement INPUT (M14). L'ordre des items dans la liste
/// reflète l'ordre textuel ; chaque item est consommé séquentiellement.
#[derive(Debug, Clone, PartialEq)]
pub enum InputItem {
    /// Une variable à lire. Le MODE de lecture dépend des champs :
    /// - `cols = Some((a, b))` : mode COLONNE — colonnes 1-based a..=b.
    /// - `informat = Some(tok)` : mode FORMATÉ — l'informat est appliqué au
    ///   champ. Avec `list_modifier = true` (`:`), la largeur sert seulement
    ///   d'informat sur un jeton délimité (mode liste).
    /// - sinon : mode LISTE — jeton délimité par espaces/délimiteurs.
    Var {
        name: String,
        /// `$` : variable caractère.
        is_char: bool,
        /// Colonnes 1-based inclusives `a-b` (mode colonne).
        cols: Option<(usize, usize)>,
        /// Token d'informat (`date9.`, `8.2`, `$char10.`...).
        informat: Option<String>,
        /// `:` modificateur — informat appliqué en mode liste (jeton
        /// délimité, pas colonnes fixes).
        list_modifier: bool,
    },
    /// `@n` : pointeur de colonne absolu (place le curseur en colonne n).
    ColumnPointer(usize),
    /// `+n` : avance le curseur de n colonnes.
    SkipColumns(usize),
    /// `/` : passe à la ligne d'entrée suivante.
    NextLine,
    /// `@` final : maintient l'enregistrement pour le prochain INPUT de la
    /// MÊME itération (line hold simple).
    HoldLine,
    /// `@@` final : maintient l'enregistrement à TRAVERS les itérations
    /// (plusieurs « observations » par ligne).
    HoldLineDouble,
}

/// Destination d'un statement FILE (M14.2) : la sortie courante des PUT.
/// Par défaut (aucun FILE), un PUT écrit dans le LOG (comportement SAS).
#[derive(Debug, Clone, PartialEq)]
pub enum PutDest {
    /// `file 'chemin';` — un fichier texte externe (créé/tronqué à la
    /// première écriture de l'étape).
    Path(String),
    /// `file log;` — le journal SAS (destination par défaut).
    Log,
    /// `file print;` — la sortie « listing » (PROC PRINT-like).
    Print,
}

/// Un item du statement PUT (M14.2). Miroir de sortie d'`InputItem` :
/// l'ordre reflète l'ordre textuel, chaque item est rendu séquentiellement
/// dans la ligne de sortie courante.
#[derive(Debug, Clone, PartialEq)]
pub enum PutItem {
    /// Une variable écrite avec son format d'affichage (ou BESTw./$w. par
    /// défaut). `format = Some(tok)` applique un format explicite
    /// (`put x 8.2;`, `put d date9.;`).
    Var {
        name: String,
        format: Option<String>,
    },
    /// `put name=;` — écrit `name=VALEUR` (forme nommée).
    NamedVar(String),
    /// `put 'texte';` — une chaîne littérale écrite verbatim.
    Literal(String),
    /// `@n` : pointeur de colonne absolu (place le curseur en colonne n,
    /// 1-based).
    ColumnPointer(usize),
    /// `+n` : avance le curseur de n colonnes.
    SkipColumns(usize),
    /// `/` : passe à la ligne de sortie suivante (saut de ligne dans le
    /// même PUT).
    NextLine,
    /// `@` final : maintient la ligne de sortie (supprime le relâchement
    /// automatique ; le prochain PUT continue la même ligne physique).
    HoldLine,
    /// `@@` final : maintient la ligne de sortie à TRAVERS les itérations.
    HoldLineDouble,
    /// `put _all_;` — écrit `nom=valeur` pour chaque variable du PDV.
    All,
}
