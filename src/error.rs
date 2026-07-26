//! Type d'erreur unique de l'interpréteur.
//!
//! # Ce que chaque variante veut dire
//!
//! - `Parse` — le source SAS est mal formé. Porte le [`Span`] du token fautif.
//! - `Runtime` — le programme est bien formé mais l'exécution refuse
//!   (table absente, option incompatible, variable inconnue…). C'est de loin
//!   la plus fréquente.
//! - `Io` / `Polars` — remontées telles quelles des couches sous-jacentes,
//!   préfixées pour qu'on sache d'où elles viennent dans le log.
//! - `Numerical` / `InvalidInput` — réservées à `stat::linalg` (matrice
//!   singulière, dimensions incompatibles). Leur PRÉFIXE de message est leur
//!   raison d'être : « numerical error: … » et « invalid input: … » sont ce
//!   que l'utilisateur lit dans le log.
//!
//! # Pourquoi `Parse.span` n'est pas imprimé (MQ9.4, vérifié)
//!
//! La revue de code l'a signalé comme un champ « write-only » à supprimer :
//! une trentaine de sites le remplissent, et tous les consommateurs terminaux
//! (`lib::run`, `executor`) font `e.to_string()`, qui n'affiche que `msg`.
//!
//! C'est exact mais ce N'EST PAS une raison de le retirer :
//!
//! 1. le log SAS 9.4 n'affiche NI ligne NI colonne NI caret sur les erreurs de
//!    syntaxe — les imprimer serait une infidélité, pas une amélioration ;
//! 2. les tests de `procs::common` VÉRIFIENT le span pour s'assurer qu'une
//!    erreur pointe le bon token ; c'est le seul garde-fou contre un helper de
//!    parsing qui signalerait la mauvaise position ;
//! 3. le jour où une destination ODS voudra un diagnostic riche, l'information
//!    est là — la reconstruire après coup serait impossible.
//!
//! Autrement dit le span est du contrat de test et de la donnée conservée à
//! dessein, pas du code mort. Décision : garder, documenter, ne pas imprimer.

use crate::token::Span;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SasError {
    /// Source mal formé ; `span` situe le token fautif (voir l'en-tête du
    /// module : il est volontairement absent du log).
    #[error("{msg}")]
    Parse { msg: String, span: Span },

    /// Programme bien formé, exécution refusée.
    #[error("{msg}")]
    Runtime { msg: String },

    #[error("file error: {0}")]
    Io(#[from] std::io::Error),

    #[error("data engine error: {0}")]
    Polars(#[from] polars::error::PolarsError),

    /// Échec numérique de `stat::linalg` (matrice singulière, non définie
    /// positive…). Le préfixe « numerical error: » est visible dans le log.
    #[error("numerical error: {0}")]
    Numerical(String),

    /// Entrée mal dimensionnée pour `stat::linalg` (matrice vide, non carrée,
    /// dimensions incompatibles). Préfixe « invalid input: » visible.
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

impl SasError {
    pub fn parse(msg: impl Into<String>, span: Span) -> Self {
        SasError::Parse {
            msg: msg.into(),
            span,
        }
    }

    pub fn runtime(msg: impl Into<String>) -> Self {
        SasError::Runtime { msg: msg.into() }
    }
}

pub type Result<T> = std::result::Result<T, SasError>;
