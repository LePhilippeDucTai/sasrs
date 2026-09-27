//! Façade publique du crate : la surface stable `sasrs::api` (ADR 0002).
//!
//! [`Session`] encapsule une session SAS vivante et réutilisable : plusieurs
//! [`Session::submit`] partagent le même WORK, les mêmes librefs, le catalogue
//! de formats et le moteur macro ; [`Session::register_dataset`] injecte un
//! `DataFrame` Polars dans une bibliothèque SAS ; [`Session::dataset`] relit
//! une table avec ses métadonnées ; [`Session::close`] finalise les
//! destinations ODS, draine log/listing et détruit la session (WORK temporaire
//! supprimé au drop).
//!
//! La fonction libre [`crate::run`] est réimplémentée sur cette façade : un
//! seul chemin d'exécution sert le binaire et la bibliothèque. Les erreurs de
//! la façade sont typées ([`ApiError`]) ; le code retour suit la convention
//! historique : sans demande explicite, 0 = propre, 1 = warnings, 2 = erreurs
//! (une demande explicite `%ABORT RETURN n` prime, avec plancher 1 si des
//! erreurs ont été comptées).

use crate::executor;
use crate::log::LogWriter;
use crate::session::Session as InnerSession;
use crate::source::SourceFile;
use polars::prelude::DataFrame;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

// Ré-exports de commodité : la façade expose les métadonnées de variables
// sans que le consommateur ait à référencer les modules internes du crate.
pub use crate::dataset::{SasDataset, VarMeta};
pub use crate::value::VarType;

/// Options de création d'une [`Session`] (ADR 0002).
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Répertoire WORK ; `None` = répertoire temporaire détruit à la
    /// fermeture de la session.
    pub work_dir: Option<PathBuf>,
    /// Base de résolution des chemins LIBNAME/INFILE relatifs
    /// (défaut : répertoire courant du processus).
    pub base_dir: Option<PathBuf>,
    /// Fige les temps (et toute sortie non reproductible) pour des
    /// résultats déterministes. Recommandé pour les tests.
    pub deterministic: bool,
    /// Active le fast-path vectorisé OPTIONNEL des étapes DATA simples
    /// (cf. `datastep::fastpath`). OFF par défaut.
    pub vectorize: bool,
}

/// Erreurs typées de la façade `sasrs::api` (ADR 0002).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// L'initialisation de la session a échoué (création de WORK, …).
    Init(String),
    /// Le libref demandé n'est pas assigné dans la session.
    UnknownLibrary { libref: String },
    /// La table demandée n'existe pas dans la bibliothèque.
    UnknownDataset { libref: String, name: String },
    /// L'enregistrement de la table a échoué (coercition ou écriture).
    Register {
        libref: String,
        name: String,
        message: String,
    },
    /// La lecture de la table a échoué.
    Read {
        libref: String,
        name: String,
        message: String,
    },
    /// Les métadonnées fournies ne correspondent pas aux colonnes du
    /// `DataFrame` (nombre ou noms).
    MetadataMismatch {
        columns: usize,
        provided: usize,
        detail: String,
    },
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Init(message) => write!(f, "{message}"),
            ApiError::UnknownLibrary { libref } => {
                write!(f, "Libref {libref} is not assigned.")
            }
            ApiError::UnknownDataset { libref, name } => {
                write!(f, "Table {libref}.{name} does not exist.")
            }
            ApiError::Register {
                libref,
                name,
                message,
            } => write!(f, "Could not register {libref}.{name}: {message}"),
            ApiError::Read {
                libref,
                name,
                message,
            } => write!(f, "Could not read {libref}.{name}: {message}"),
            ApiError::MetadataMismatch {
                columns,
                provided,
                detail,
            } => write!(
                f,
                "Metadata mismatch: {detail} ({provided} provided, {columns} columns)"
            ),
        }
    }
}

impl std::error::Error for ApiError {}

/// Résultat d'une soumission : log, listing, compteurs et code retour.
#[derive(Debug, Clone)]
pub struct Submission {
    /// Log SAS produit par cette soumission (numérotation du source
    /// repartant à 1 à chaque `submit`).
    pub log: String,
    /// Listing (ou dernière destination ODS textuelle) finalisé à l'issue
    /// de la soumission.
    pub listing: String,
    /// 0 = propre, 1 = warnings, 2 = erreurs ; une demande explicite
    /// (`%ABORT RETURN n`) prime, avec plancher 1 si des erreurs ont été
    /// comptées pendant cette soumission.
    pub exit_code: i32,
    /// Erreurs comptées pendant CETTE soumission.
    pub errors: u32,
    /// Warnings comptés pendant CETTE soumission.
    pub warnings: u32,
}

/// Rapport de fermeture de la session : reliquat de log/listing (ex. NOTEs
/// de finalisation ODS) et bilan global.
#[derive(Debug, Clone)]
pub struct CloseReport {
    /// Reliquat du log depuis la dernière soumission.
    pub log: String,
    /// Reliquat du listing depuis la dernière soumission.
    pub listing: String,
    /// Code retour GLOBAL de la session (compteurs cumulés + demande
    /// explicite éventuelle).
    pub exit_code: i32,
    /// Erreurs comptées sur toute la session.
    pub errors: u32,
    /// Warnings comptés sur toute la session.
    pub warnings: u32,
}

/// Session SAS vivante derrière la façade publique (ADR 0002).
///
/// Les soumissions successives partagent l'état de session : WORK, librefs,
/// catalogue de formats, vues SQL et moteur macro (`%let`/`&var`).
pub struct Session {
    inner: InnerSession,
    deterministic: bool,
    total_errors: u32,
    total_warnings: u32,
    /// Demande de code retour explicite la plus récente non encore
    /// reflétée dans un bilan de fermeture.
    requested_exit_code: Option<i32>,
}

impl Session {
    /// Crée une session. `Err(ApiError::Init)` si le répertoire WORK ne peut
    /// pas être créé ou si l'initialisation panique.
    pub fn new(options: Options) -> Result<Self, ApiError> {
        let base_dir = options
            .base_dir
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let base_dir = absolutize(base_dir);
        let created = catch_unwind(AssertUnwindSafe(|| {
            InnerSession::new(options.work_dir, base_dir, options.deterministic)
        }));
        let mut inner = match created {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Err(ApiError::Init(e.to_string())),
            Err(payload) => return Err(ApiError::Init(internal_error(payload.as_ref()))),
        };
        inner.vectorize = options.vectorize;
        Ok(Session {
            inner,
            deterministic: options.deterministic,
            total_errors: 0,
            total_warnings: 0,
            requested_exit_code: None,
        })
    }

    /// Soumet un programme SAS complet et rend son log, son listing, les
    /// compteurs et le code retour — sans consommer la session : WORK et
    /// moteur macro persistent pour les soumissions suivantes.
    ///
    /// Un panic de l'interpréteur est capturé et journalisé comme
    /// `ERROR: internal error: …` (la session reste utilisable), comme le
    /// fait la fonction libre [`crate::run`].
    pub fn submit(&mut self, code: &str) -> Submission {
        let (errors_before, warnings_before) = (self.inner.log.errors, self.inner.log.warnings);
        let src = SourceFile::new(code.to_string());
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
            executor::run_program(&src, &mut self.inner)
        })) {
            self.inner.log.error(&internal_error(payload.as_ref()));
        }
        // Finalisation de la destination courante : chaque soumission rend
        // un listing complet (même protocole que la fin de `run`).
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| self.inner.finish_destination())) {
            self.inner.log.error(&internal_error(payload.as_ref()));
        }
        let listing = self.inner.take_completed_listing();
        let requested = self.inner.take_requested_exit_code();
        if requested.is_some() {
            self.requested_exit_code = requested;
        }
        // Draine le log SANS consommer la session : l'écrivain est remplacé
        // à l'identique (déterminisme conservé), les compteurs du drain sont
        // rapportés à cette soumission puis cumulés. L'écrivain retiré est
        // possédé par la façade : `into_string` le consomme pour le buffer.
        let writer = std::mem::replace(&mut self.inner.log, LogWriter::new(self.deterministic));
        let errors = writer.errors - errors_before;
        let warnings = writer.warnings - warnings_before;
        let log = writer.into_string();
        self.total_errors += errors;
        self.total_warnings += warnings;
        let exit_code = match requested {
            Some(code) => code.max(i32::from(errors > 0)),
            None if errors > 0 => 2,
            None if warnings > 0 => 1,
            None => 0,
        };
        Submission {
            log,
            listing,
            exit_code,
            errors,
            warnings,
        }
    }

    /// Injecte un `DataFrame` Polars comme table `libref.name` dans une
    /// bibliothèque SAS. Le frame est coercé vers le modèle de types SAS
    /// (numérique f64 / caractère string) ; les métadonnées optionnelles
    /// (`Vec<VarMeta>`, une entrée par colonne, mêmes noms à la casse près)
    /// remplacent celles inférées et sont persistées dans le sidecar
    /// (ADR 0001). La table devient `_LAST_`.
    pub fn register_dataset(
        &mut self,
        libref: &str,
        name: &str,
        df: DataFrame,
        metadata: Option<Vec<VarMeta>>,
    ) -> Result<(), ApiError> {
        let libref_u = libref.to_ascii_uppercase();
        let name_u = name.to_ascii_uppercase();
        if !is_valid_name(&libref_u) || !is_valid_name(&name_u) {
            return Err(ApiError::Register {
                libref: libref_u,
                name: name_u,
                message: "libref and member names must be non-empty SAS names".to_string(),
            });
        }
        let (mut ds, notes) = SasDataset::from_dataframe(df).map_err(|e| ApiError::Register {
            libref: libref_u.clone(),
            name: name_u.clone(),
            message: e.to_string(),
        })?;
        for note in &notes {
            self.inner.log.forward(note);
        }
        if let Some(vars) = metadata {
            if vars.len() != ds.vars.len() {
                return Err(ApiError::MetadataMismatch {
                    columns: ds.vars.len(),
                    provided: vars.len(),
                    detail: "one VarMeta per column expected".to_string(),
                });
            }
            for (v, col) in vars.iter().zip(ds.vars.iter()) {
                if !v.name.eq_ignore_ascii_case(&col.name) {
                    return Err(ApiError::MetadataMismatch {
                        columns: ds.vars.len(),
                        provided: vars.len(),
                        detail: format!(
                            "metadata names {:?} do not match column {:?}",
                            v.name, col.name
                        ),
                    });
                }
            }
            // Les noms canoniques restent ceux des colonnes du frame.
            ds.vars = vars
                .into_iter()
                .zip(ds.vars.iter())
                .map(|(mut v, col)| {
                    v.name = col.name.clone();
                    v
                })
                .collect();
        }
        let provider = self
            .inner
            .libs
            .get(&libref_u)
            .map_err(|_| ApiError::UnknownLibrary {
                libref: libref_u.clone(),
            })?;
        provider
            .write(&name_u, &ds)
            .map_err(|e| ApiError::Register {
                libref: libref_u.clone(),
                name: name_u.clone(),
                message: e.to_string(),
            })?;
        self.inner.last_dataset = Some(format!("{libref_u}.{name_u}"));
        self.inner.log.note(&format!(
            "Dataset {libref_u}.{name_u} was registered from the host, with {} observations and {} variables.",
            ds.n_obs(),
            ds.n_vars()
        ));
        Ok(())
    }

    /// Relit la table `libref.name` : données (`DataFrame` déjà coercé au
    /// modèle SAS) et métadonnées (`Vec<VarMeta>` restaurées du sidecar).
    pub fn dataset(
        &mut self,
        libref: &str,
        name: &str,
    ) -> Result<(DataFrame, Vec<VarMeta>), ApiError> {
        let libref_u = libref.to_ascii_uppercase();
        let name_u = name.to_ascii_uppercase();
        let provider = self
            .inner
            .libs
            .get(&libref_u)
            .map_err(|_| ApiError::UnknownLibrary {
                libref: libref_u.clone(),
            })?;
        if !provider.exists(&name_u) {
            return Err(ApiError::UnknownDataset {
                libref: libref_u,
                name: name_u,
            });
        }
        let (ds, notes) = provider.read(&name_u).map_err(|e| ApiError::Read {
            libref: libref_u,
            name: name_u,
            message: e.to_string(),
        })?;
        for note in &notes {
            self.inner.log.forward(note);
        }
        Ok((ds.df, ds.vars))
    }

    /// Ferme la session : finalise les destinations ODS ouvertes, draine le
    /// reliquat de log/listing, calcule le code retour global, puis détruit
    /// la session — le répertoire WORK temporaire est supprimé au drop du
    /// gestionnaire de bibliothèques.
    pub fn close(mut self) -> CloseReport {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| self.inner.finish_destination())) {
            self.inner.log.error(&internal_error(payload.as_ref()));
        }
        let listing = self.inner.take_completed_listing();
        let writer = std::mem::replace(&mut self.inner.log, LogWriter::new(self.deterministic));
        let errors = writer.errors;
        let warnings = writer.warnings;
        let log = writer.into_string();
        self.total_errors += errors;
        self.total_warnings += warnings;
        let exit_code = match self.requested_exit_code {
            Some(code) => code.max(i32::from(self.total_errors > 0)),
            None if self.total_errors > 0 => 2,
            None if self.total_warnings > 0 => 1,
            None => 0,
        };
        CloseReport {
            log,
            listing,
            exit_code,
            errors: self.total_errors,
            warnings: self.total_warnings,
        }
    }
}

fn absolutize(p: PathBuf) -> PathBuf {
    if p.is_absolute() {
        p
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    }
}

fn is_valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn internal_error(payload: &(dyn std::any::Any + Send)) -> String {
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("unknown panic payload");
    format!("internal error: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_default_mirrors_run_options() {
        let o = Options::default();
        assert!(o.work_dir.is_none());
        assert!(o.base_dir.is_none());
        assert!(!o.deterministic);
        assert!(!o.vectorize);
    }

    #[test]
    fn api_error_messages_are_stable() {
        assert_eq!(
            ApiError::UnknownLibrary { libref: "X".into() }.to_string(),
            "Libref X is not assigned."
        );
        assert_eq!(
            ApiError::UnknownDataset {
                libref: "WORK".into(),
                name: "A".into()
            }
            .to_string(),
            "Table WORK.A does not exist."
        );
    }

    #[test]
    fn valid_name_checks() {
        assert!(is_valid_name("WORK"));
        assert!(is_valid_name("A_1"));
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("A-B"));
        assert!(!is_valid_name("a.b"));
    }
}
