use super::*;

/// Enregistrement maintenu par un hold `@`/`@@` (M14).
pub(super) struct HeldLine {
    pub(super) line: String,
    pub(super) cursor: usize,
    /// `@@` : survit aux itérations ; `@` : relâché à la prochaine itération.
    pub(super) double: bool,
}

/// État de sortie texte du PUT (M14.2). Mirroir de sortie du held-line de
/// l'INPUT : une destination courante, une ligne de sortie en construction
/// (`line`), un curseur de colonne 0-based, et le drapeau de hold `@`/`@@`.
pub(super) struct PutState {
    /// Destination courante (par défaut le LOG, conformément à SAS).
    pub(super) dest: PutDestKind,
    /// Ligne de sortie en construction (avant relâchement/flush).
    pub(super) line: String,
    /// Position d'écriture courante (colonne 0-based) dans `line`.
    pub(super) cursor: usize,
    /// Une ligne est-elle en cours de construction (au moins un PUT l'a
    /// commencée) ? Sert à distinguer une ligne vide explicite d'un état
    /// vierge au flush de fin d'étape.
    pub(super) started: bool,
    /// Hold simple `@` actif : la ligne n'est PAS relâchée en fin de PUT ;
    /// relâchée au début de l'itération suivante.
    pub(super) hold: bool,
    /// Hold double `@@` actif : la ligne survit aux itérations.
    pub(super) hold_double: bool,
    /// Lignes de sortie complètes, dans l'ordre de production, taguées par
    /// leur destination. Rejouées vers le LOG / le listing / les fichiers
    /// APRÈS la boucle implicite (exec.rs n'a pas `&mut session` en boucle).
    pub(super) out: Vec<(PutDestKind, String)>,
}

impl PutState {
    pub(super) fn new() -> Self {
        PutState {
            dest: PutDestKind::Log,
            line: String::new(),
            cursor: 0,
            started: false,
            hold: false,
            hold_double: false,
            out: Vec::new(),
        }
    }
}

/// État d'E/S TEXTE du Runner (M14/M14.2) : lecture INFILE/INPUT/DATALINES
/// (source, curseurs, hold `@`/`@@`) et sortie FILE/PUT. Regroupé car ces
/// champs ne servent que les statements texte (la sortie PUT reste toutefois
/// utilisable dans tous les modes d'exécution, UPDATE/MODIFY compris).
pub(super) struct TextIo {
    /// Source d'entrée TEXTE (M14 : INFILE/INPUT/DATALINES).
    pub(super) src: Option<TextInput>,
    /// Prochaine ligne brute (index dans `src.lines`) à charger.
    pub(super) next_line: usize,
    /// Nombre d'enregistrements (lignes) lus de la source texte.
    pub(super) read: usize,
    /// Enregistrement maintenu par `@`/`@@` : la ligne courante, le curseur
    /// (colonne 0-based) et un drapeau `double` (`@@` survit aux itérations ;
    /// `@` simple est relâché au début de l'itération suivante). `Some` quand
    /// un hold est actif.
    pub(super) held: Option<HeldLine>,
    /// État de sortie texte des PUT (M14.2 : FILE/PUT).
    pub(super) put: PutState,
}

impl TextIo {
    pub(super) fn new(src: Option<TextInput>) -> Self {
        TextIo {
            src,
            next_line: 0,
            read: 0,
            held: None,
            put: PutState::new(),
        }
    }
}

/// Curseurs du statement SET (concaténation / interclassement / POINT=).
/// Vides/inertes hors SET.
pub(super) struct SetCursor {
    /// Mode CONCATÉNATION (sans BY) : index du dataset en cours de lecture.
    pub(super) cur_ds: usize,
    /// Curseur PAR dataset : sans BY, prochaine ligne brute à charger (y
    /// compris celles rejetées par WHERE=) ; avec BY, position dans
    /// `filtered`.
    pub(super) cursors: Vec<usize>,
    /// Mode INTERCLASSEMENT (avec BY) : indices des lignes qui passent le
    /// WHERE= (pré-filtrage), par dataset. Sans WHERE=, toutes les lignes.
    pub(super) filtered: Vec<Vec<usize>>,
    /// Clés BY de la dernière observation servie : FIRST. et détection de
    /// désordre.
    pub(super) prev_keys: Option<Vec<Value>>,
}

impl SetCursor {
    pub(super) fn new(n_datasets: usize) -> Self {
        SetCursor {
            cur_ds: 0,
            cursors: vec![0; n_datasets],
            filtered: vec![Vec::new(); n_datasets],
            prev_keys: None,
        }
    }
}

/// M40.2 — état d'exécution d'un site SET SUPPLÉMENTAIRE (2ᵉ, 3ᵉ…
/// statement SET de l'étape) : ses datasets matérialisés, son curseur de
/// concaténation et ses compteurs de lignes lues, tous INDÉPENDANTS du
/// site 0 (`Runner::{input, set_cursor, rows_read}`).
pub(super) struct SiteState {
    /// Datasets du site (concaténation séquentielle ; jamais de BY/MERGE/
    /// POINT=, refusés à la compilation).
    pub(super) input: InputData,
    /// Curseur de concaténation propre au site.
    pub(super) cursor: SetCursor,
    /// Lignes lues au sens SAS (celles qui passent le WHERE=), par dataset.
    pub(super) rows_read: Vec<usize>,
    /// Index du flag END= de ce site dans `EvalCtx::end_flags` (si déclaré).
    pub(super) end_idx: Option<usize>,
}

/// État du mode MERGE (M3) : plan pré-calculé + curseur. Vide hors MERGE.
pub(super) struct MergeState {
    /// Séquence pré-calculée des observations de sortie (groupe par groupe).
    pub(super) plan: Vec<MergeObs>,
    /// Curseur dans `plan` (prochaine obs à servir).
    pub(super) cursor: usize,
}

impl MergeState {
    pub(super) fn new() -> Self {
        MergeState {
            plan: Vec::new(),
            cursor: 0,
        }
    }
}

/// État partagé d'un MODIFY+POINT= (M16.5). Le bras `DsStmt::Modify` de
/// `exec_stmt` y charge l'obs à l'index POINT= courant (et capture la
/// précédente). `cols` est le tampon de réécriture (parallèle à `var_slots`).
pub(super) struct ModifyState {
    /// Slot PDV de la variable d'index POINT=.
    pub(super) point_slot: usize,
    /// Tampon de réécriture : colonnes décodées, modifiées au fil des captures.
    pub(super) cols: Vec<Vec<Value>>,
    /// Slots PDV de chaque colonne (parallèle à `cols`).
    pub(super) var_slots: Vec<usize>,
    /// Ligne actuellement chargée (à capturer au prochain marqueur / en fin).
    pub(super) cur_row: Option<usize>,
    /// "WORK.A" pour les messages d'erreur POINT=.
    pub(super) display: String,
    /// Nombre total d'observations (bornes de l'index POINT=).
    pub(super) n_rows: usize,
    /// Erreur POINT= différée (index invalide), remontée par la boucle externe.
    pub(super) error: Option<String>,
    /// Lignes touchées (chargées au moins une fois) — compteur de lecture.
    pub(super) touched: Vec<bool>,
}

/// Une observation de sortie d'un MERGE, pré-calculée par `build_merge_plan`.
pub(super) struct MergeObs {
    /// Slots à remettre à MISSING AVANT les chargements (variables PROPRES
    /// des datasets absents du groupe) — non vide seulement à la 1re obs du
    /// groupe.
    pub(super) blank_slots: Vec<usize>,
    /// Chargements à appliquer dans l'ORDRE (gauche→droite du MERGE) : le
    /// dernier dataset qui contribue écrase les variables partagées.
    /// `(index dataset, ligne)`.
    pub(super) loads: Vec<(usize, usize)>,
    /// État IN= par dataset pour ce groupe (`true` = a participé).
    pub(super) in_active: Vec<bool>,
    /// FIRST./LAST. par variable BY (préfixe de clés), parallèle à
    /// `input.by`.
    pub(super) first: Vec<bool>,
    pub(super) last: Vec<bool>,
}
