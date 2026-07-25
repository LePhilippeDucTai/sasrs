use super::*;

/// Un dataset matérialisé d'un statement SET.
pub struct InputDataset {
    /// "WORK.B" pour les NOTEs du log.
    pub display: String,
    /// Colonnes décodées en `Value` (via `missing::num_to_value`), une
    /// seule passe de downcast par colonne — jamais de get_row. Seules les
    /// colonnes retenues par KEEP=/DROP= sont présentes ; une colonne
    /// renommée par RENAME= est au PDV sous son NOUVEAU nom.
    pub columns: Vec<Vec<Value>>,
    /// Slot PDV de chaque colonne (parallèle à `columns`).
    pub var_slots: Vec<usize>,
    pub n_rows: usize,
    /// `WHERE=` du SET : sans BY, évalué à l'EXÉCUTION après chargement de
    /// chaque ligne dans le PDV ; une ligne qui échoue est sautée SANS
    /// exécuter le reste de l'itération et ne compte PAS dans les
    /// observations lues (comme la NOTE SAS "There were N observations
    /// read"). Avec BY, le filtre est PRÉ-APPLIQUÉ par l'exécuteur avant
    /// l'interclassement (mêmes règles). NB : comme l'évaluation se fait
    /// sur le PDV, un WHERE= combiné à RENAME= référence les NOUVEAUX noms
    /// (divergence documentée — SAS applique WHERE= avant RENAME= en
    /// entrée).
    pub where_: Option<Expr>,
    /// Index dans `columns` de chaque variable BY (parallèle à
    /// `InputData::by`) ; vide sans BY. Chaque variable BY doit exister
    /// dans CHAQUE dataset du SET (vérifié à la compilation).
    pub by_cols: Vec<usize>,
}

/// Une clé du statement BY, résolue à la compilation.
pub struct ByVar {
    /// Nom canonique MAJUSCULE (sert les variables FIRST.x / LAST.x).
    pub name: String,
    /// Slot PDV de la variable.
    pub slot: usize,
    pub descending: bool,
}

/// Données d'entrée matérialisées du statement SET (M3 : un ou plusieurs
/// datasets, BY optionnel).
pub struct InputData {
    /// Les datasets, dans l'ordre du statement SET. Sans BY, ils sont lus
    /// en CONCATÉNATION (le premier en entier, puis le suivant) ; avec BY,
    /// en INTERCLASSEMENT par clés croissantes (cf. exec.rs).
    pub datasets: Vec<InputDataset>,
    /// Clés du BY (vide = pas de BY).
    pub by: Vec<ByVar>,
    /// MERGE (M3) : `true` = match-merge SAS par BY (au lieu de SET).
    /// L'exécuteur pré-calcule la séquence des obs de sortie groupe par
    /// groupe (cf. exec.rs).
    pub merge: bool,
    /// Variables IN= du MERGE : `(nom UPPERCASE, index dataset)`. Servies
    /// par `EvalCtx::in_flags` (jamais de slot PDV, comme FIRST./LAST.).
    pub in_flags: Vec<(String, usize)>,
    /// END= (M16.4) : nom UPPERCASE de la variable automatique temporaire
    /// (0 pendant l'itération, 1 après lecture de la DERNIÈRE obs du DERNIER
    /// dataset). Servie par `EvalCtx::end_flag`, jamais écrite en sortie.
    pub end_var: Option<String>,
    /// NOBS= (M16.4) : slot PDV de la variable numérique affectée AVANT la
    /// boucle au nombre TOTAL d'observations (somme des datasets du SET).
    pub nobs_slot: Option<usize>,
    /// POINT= (M16.4) : slot PDV de la variable d'index 1-based. Sa présence
    /// DÉSACTIVE la boucle implicite et l'output implicite : chaque SET lit
    /// l'obs à l'index courant (erreur si missing/invalide/hors bornes).
    pub point_slot: Option<usize>,
}

/// Options d'exécution d'une lecture texte (M14), reprises de l'INFILE.
pub struct TextOptions {
    pub delimiter: Option<String>,
    pub dsd: bool,
    pub firstobs: usize,
    pub obs: Option<usize>,
    /// Comportement en cas de ligne trop courte : 0 = défaut (passe à la
    /// ligne suivante en mode liste), 1 = MISSOVER, 2 = TRUNCOVER, 3 =
    /// STOPOVER.
    pub short: ShortMode,
}

/// Source d'entrée texte compilée (M14) : lignes brutes + spécification
/// INPUT résolue. Parallèle à `InputData` (le chemin SET).
pub struct TextInput {
    /// "the infile 'path'" pour la NOTE du log (fichier externe seulement).
    pub display: String,
    /// Lignes brutes (DATALINES inline ou contenu du fichier).
    pub lines: Vec<String>,
    pub options: TextOptions,
    /// `true` si la source est un FICHIER externe (`infile 'path'`). Pour les
    /// données instream DATALINES/CARDS, SAS n'émet PAS de NOTE "N records
    /// were read from the infile ..." (réservée aux fichiers physiques) :
    /// l'exécuteur s'en sert pour ne l'émettre que dans le cas fichier.
    pub is_file: bool,
}

/// Une sortie : où écrire et quels slots du PDV.
pub struct OutputSpec {
    pub libref: String,
    pub table: String,
    /// "WORK.A" pour les NOTEs.
    pub display: String,
    /// Slots PDV conservés, dans l'ordre PDV (= ordre des colonnes).
    /// Combinaison (intersection) des statements KEEP/DROP et des options
    /// de dataset KEEP=/DROP= de CETTE sortie.
    pub kept_slots: Vec<usize>,
    /// Nom d'écriture de chaque slot conservé (parallèle à `kept_slots`) :
    /// le nom PDV, ou le nouveau nom si RENAME= s'applique.
    pub out_names: Vec<String>,
}

/// Données d'entrée compilées d'un statement UPDATE (M16.5). Le maître et la
/// transaction sont matérialisés en colonnes décodées (comme `InputDataset`),
/// avec le slot PDV de chaque colonne. Les variables clé (`key_slots`) servent
/// l'appariement. `master_where` est filtré à l'exécution (sur le PDV chargé,
/// comme SET WHERE=). `by` (optionnel) restreint la fusion aux groupes BY.
pub struct UpdateData {
    /// Le maître, lu séquentiellement (pilote l'itération).
    pub master: InputDataset,
    /// La transaction, indexée par clé (recherche par `key_slots`).
    pub transaction: InputDataset,
    /// Slots PDV des variables clé (ordre du KEY=). Ces slots ne sont jamais
    /// écrasés par la transaction.
    pub key_slots: Vec<usize>,
    /// Slots PDV des variables de la transaction qui peuvent superposer le
    /// maître (toutes SAUF les clés). Une valeur transaction MANQUANTE ne
    /// superpose pas (sémantique « missing = no update »).
    pub overlay_slots: Vec<usize>,
    /// WHERE= du maître, évalué à l'exécution sur le PDV chargé.
    pub master_where: Option<Expr>,
    /// Clés BY (vide = pas de BY) — déclaratif, sert FIRST./LAST.
    pub by: Vec<ByVar>,
}

/// Données d'entrée compilées d'un statement MODIFY (M16.5). Le dataset est
/// matérialisé et RÉÉCRIT en place après l'étape. `key_slots` peut être vide
/// (lecture séquentielle). `point_slot`/`nobs_slot` reprennent la sémantique
/// d'accès direct du SET.
pub struct ModifyData {
    /// Le dataset à modifier (libref/table pour la réécriture).
    pub libref: String,
    pub table: String,
    /// "WORK.A" pour les NOTEs.
    pub display: String,
    /// Le dataset matérialisé en colonnes décodées + slots PDV.
    pub data: InputDataset,
    /// Slots PDV des variables clé (vide = lecture séquentielle).
    pub key_slots: Vec<usize>,
    /// POINT= : slot PDV de l'index 1-based (accès direct, comme SET POINT=).
    pub point_slot: Option<usize>,
    /// NOBS= : slot PDV affecté avant la boucle au nombre d'observations.
    pub nobs_slot: Option<usize>,
    /// Métadonnées de sortie (VarMeta) de CHAQUE slot PDV de `data.var_slots`,
    /// dans l'ordre, pour réécrire le dataset à l'identique (mêmes colonnes).
    pub out_vars: Vec<crate::dataset::VarMeta>,
}
