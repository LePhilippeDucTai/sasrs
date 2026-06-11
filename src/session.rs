use crate::library::LibraryManager;
use crate::listing::ListingWriter;
use crate::log::LogWriter;
use std::path::PathBuf;

/// System options (OPTIONS statement). M1 honors LS=; everything else is
/// parsed and ignored with a WARNING.
pub struct SasOptions {
    pub ls: usize,
}

impl Default for SasOptions {
    fn default() -> Self {
        // SAS 9.4 listing default linesize.
        SasOptions { ls: 96 }
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
}

impl Session {
    pub fn new(
        work_dir: Option<PathBuf>,
        base_dir: PathBuf,
        deterministic: bool,
    ) -> crate::error::Result<Self> {
        let options = SasOptions::default();
        Ok(Session {
            libs: LibraryManager::new(work_dir)?,
            log: LogWriter::new(deterministic),
            listing: ListingWriter::new(options.ls),
            options,
            base_dir,
            last_dataset: None,
            deterministic,
        })
    }
}
