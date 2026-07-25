use super::*;

/// Objet hash de l'étape DATA (M17.1). Créé par `DECLARE HASH h(...)`, défini
/// par `defineKey`/`defineData`/`defineDone`, puis manipulé par find/add/etc.
/// (M17.2). Stocké dans `EvalCtx.hashes` (nom UPPERCASE → objet).
///
/// `rows` indexe les données par clé encodée (sémantique d'égalité SAS via
/// `hash_key` : `. == .`, char insensible aux blancs finaux). Chaque entrée est
/// une LISTE de jeux de données : sans `multidata`, une seule entrée par clé
/// (l'ajout remplace ou est rejeté selon `duplicate`) ; avec `multidata:'yes'`,
/// plusieurs jeux de données peuvent partager une clé.
#[derive(Debug, Clone, Default)]
pub struct HashObject {
    /// Option `ordered:` (`'yes'`/`'ascending'`/`'descending'`/`'no'`),
    /// normalisée en minuscules. `None` = non spécifiée (= `'no'`).
    pub ordered: Option<String>,
    /// Option `duplicate:` (`'replace'`/`'error'`/`'no'`), en minuscules.
    /// `None` = défaut (comportement SAS : la première valeur est conservée,
    /// l'ajout d'une clé existante est ignoré sans erreur).
    pub duplicate: Option<String>,
    /// Option `multidata:'yes'` : plusieurs jeux de données par clé.
    pub multidata: bool,
    /// Option `dataset:'lib.table'` : table à charger au `defineDone` (le
    /// chargement effectif est différé à M17.2 ; le nom est conservé ici).
    pub dataset: Option<String>,
    /// Colonnes clé (noms UPPERCASE, dans l'ordre de `defineKey`).
    pub keys: Vec<String>,
    /// Colonnes données (noms UPPERCASE, dans l'ordre de `defineData`).
    pub data_vars: Vec<String>,
    /// Données : clé encodée → liste de jeux de valeurs de données (un seul
    /// élément sans `multidata`). Parallèle à `data_vars`.
    pub rows: std::collections::HashMap<String, Vec<Vec<Value>>>,
    /// `defineDone()` a été appelé : l'objet est finalisé (idempotent).
    pub defined: bool,
    /// Ordre d'INSERTION des clés encodées (premier ajout). Préserve l'ordre
    /// de visite par défaut (sans `ordered:`) pour l'itérateur HITER et
    /// `output`. Une clé supprimée puis ré-ajoutée reprend une nouvelle place.
    pub insertion_order: Vec<String>,
    /// Colonnes du dataset chargé via `dataset:` au `defineDone` (M17.2),
    /// pré-lues à la compilation (`&mut Session` disponible) : nom de colonne
    /// UPPERCASE → valeurs décodées. `None` = pas d'option `dataset:`. Le
    /// chargement effectif (mapping keys/data_vars → rows) a lieu au
    /// `defineDone`, quand les clés/données sont connues.
    pub dataset_cols: Option<std::collections::HashMap<String, Vec<Value>>>,
    /// Nombre de lignes du dataset pré-lu (parallèle à `dataset_cols`).
    pub dataset_nrows: usize,
    /// Valeurs de clé DÉCODÉES par clé encodée (M17.2) : la clé encodée perd le
    /// type/la valeur exacte (collation) ; on conserve donc les `Value`
    /// d'origine pour `output` (reconstitution des colonnes clé) et le tri
    /// `ordered:` (via `sas_cmp`).
    pub key_values: std::collections::HashMap<String, Vec<Value>>,
    /// Curseur multidata courant (M17.2) : `(clé encodée, index dans la liste
    /// d'entrées)`, posé par `find`, avancé par `find_next`/`find_prev`.
    pub find_cursor: Option<(String, usize)>,
}

impl HashObject {
    /// Positionne le curseur multidata (find/find_next).
    pub fn set_find_cursor(&mut self, key: &str, idx: usize) {
        self.find_cursor = Some((key.to_string(), idx));
    }
}

/// Itérateur d'objet hash (M17.2), déclaré par `DECLARE HITER hi('h');`.
/// Lié à l'objet hash `hash` (nom UPPERCASE). `pos` est l'index courant dans
/// l'ordre de visite (calculé à la volée : `ordered:` → tri par clés via
/// `sas_cmp`, sinon ordre d'insertion). `None` = itérateur non positionné.
#[derive(Debug, Clone, Default)]
pub struct HashIter {
    /// Nom UPPERCASE de l'objet hash parcouru.
    pub hash: String,
    /// Position courante (index dans la séquence de visite aplatie). `None`
    /// avant tout `first`/`last` (ou après un `next`/`prev` hors limites).
    pub pos: Option<usize>,
}

/// Clé d'appariement canonique d'une liste de `Value` pour un objet hash
/// (M17.1). Encode la sémantique d'égalité SAS (`. == .`, char insensible aux
/// blancs finaux) — identique à la clé UPDATE/MODIFY. Sert de clé de `HashMap`.
pub fn hash_key(values: &[Value]) -> String {
    let mut s = String::new();
    for v in values {
        match v {
            Value::Num(n) => {
                s.push('N');
                s.push_str(&format!("{n:?}"));
            }
            Value::Missing(k) => {
                s.push('M');
                s.push_str(&k.display());
            }
            Value::Char(c) => {
                s.push('C');
                s.push_str(c.trim_end());
            }
        }
        s.push('\u{1}');
    }
    s
}
