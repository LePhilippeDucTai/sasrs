use crate::library::LibraryManager;
use crate::listing::ListingWriter;
use crate::log::LogWriter;
use std::path::PathBuf;

/// System options (OPTIONS statement). M1 honors LS=; everything else is
/// parsed and ignored with a WARNING.
pub struct SasOptions {
    pub ls: usize,
    /// FIRSTOBS= : 1-based number of the first observation to read from each
    /// input data set. Default 1.
    pub firstobs: usize,
    /// OBS= : the number of the LAST observation to process (1-based, an upper
    /// bound on the observation count read). `None` = no limit (OBS=MAX).
    pub obs: Option<usize>,
}

impl Default for SasOptions {
    fn default() -> Self {
        // SAS 9.4 listing default linesize.
        SasOptions {
            ls: 96,
            firstobs: 1,
            obs: None,
        }
    }
}

/// Everything a step needs to execute: libraries, output writers, options.
pub struct Session {
    pub libs: LibraryManager,
    pub log: LogWriter,
    pub listing: ListingWriter,
    pub options: SasOptions,
    /// Directory against which relative LIBNAME paths resolve.
    pub base_dir: PathBuf,
    /// _LAST_: most recently created dataset, e.g. "WORK.A" — the default
    /// input of procs without DATA=.
    pub last_dataset: Option<String>,
    pub deterministic: bool,
    /// User-defined format catalog (populated by PROC FORMAT).
    pub format_catalog: crate::formats::FormatCatalog,
    /// Processeur macro de la session (M11) : table des symboles `%let`/`&var`.
    /// Sous le build par défaut c'est une identité pure (cf. `MacroEngine`).
    pub macro_engine: crate::preprocess::MacroEngine,
    /// Opt-in : autorise le fast-path vectorisé des étapes DATA simples
    /// (`datastep::fastpath`). OFF par défaut — le chemin ligne-à-ligne reste
    /// la référence ; le fast-path ne s'active que pour les étapes que
    /// `fastpath::eligible` prouve équivalentes.
    pub vectorize: bool,
    /// CALL EXECUTE (M15.6) : file de code SAS produit par `call execute(...)`
    /// pendant une étape DATA, mis en attente pour exécution APRÈS l'étape.
    /// L'exécuteur (`executor::exec_data_step`) draine cette file et rejoue le
    /// code concaténé comme un programme SAS à part entière, une fois l'étape
    /// terminée (sémantique SAS : les instructions CALL EXECUTE s'exécutent
    /// après le RUN de l'étape qui les a générées).
    pub call_execute_queue: Vec<String>,
    /// Test-only (M17.1) : instantané des objets hash de la DERNIÈRE étape DATA
    /// exécutée (nom UPPERCASE → objet final). Peuplé par `datastep::execute`
    /// sous `cfg(test)` pour observer keys/data_vars/options/defined depuis les
    /// tests unitaires (les méthodes find/add qui exposeraient l'état arrivent
    /// en M17.2). Aucun coût ni champ en build de production.
    #[cfg(test)]
    pub debug_hashes: std::collections::HashMap<String, crate::datastep::HashObject>,
}

impl Session {
    pub fn new(
        work_dir: Option<PathBuf>,
        base_dir: PathBuf,
        deterministic: bool,
    ) -> crate::error::Result<Self> {
        let options = SasOptions::default();
        // M19.2 : le processeur macro résout les chemins relatifs de
        // `%include 'fichier';` contre le répertoire de base de la session
        // (même base que LIBNAME/INFILE). `SASAUTOS` reste vide par défaut.
        let mut macro_engine = crate::preprocess::MacroEngine::new(deterministic);
        macro_engine.set_include_base_dir(base_dir.clone());
        Ok(Session {
            libs: LibraryManager::new(work_dir)?,
            log: LogWriter::new(deterministic),
            listing: ListingWriter::new(options.ls),
            options,
            base_dir,
            last_dataset: None,
            deterministic,
            format_catalog: crate::formats::FormatCatalog::default(),
            macro_engine,
            vectorize: false,
            call_execute_queue: Vec::new(),
            #[cfg(test)]
            debug_hashes: std::collections::HashMap::new(),
        })
    }

    /// Résout un chemin de fichier externe (INFILE/FILE, PROC IMPORT/EXPORT) :
    /// absolu → tel quel ; relatif → joint à `base_dir` (le répertoire de
    /// travail de la session, comme pour les chemins LIBNAME). Garantit un
    /// comportement cohérent et déterministe (les fixtures relatives résolvent
    /// sous le tempdir du harnais, pas sous le CWD du processus).
    pub fn resolve_path(&self, path: &str) -> PathBuf {
        let p = PathBuf::from(path);
        if p.is_absolute() {
            p
        } else {
            self.base_dir.join(p)
        }
    }
}
