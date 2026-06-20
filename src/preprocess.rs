//! Façade historique du processeur macro.
//!
//! Le code du processeur macro vit désormais dans le module `crate::macros`
//! (scission M32 du monolithe `preprocess.rs`). Ce fichier ne conserve que les
//! ré-exports publics afin de ne pas toucher les sites d'import existants
//! (`crate::preprocess::MacroEngine`, `crate::preprocess::RawSegmenter`, …).
//! Toute évolution du code se fait dans `src/macros/`.

pub use crate::macros::{
    MacroDef, MacroEngine, MacroError, MacroParam, MacroStage, RawSegmenter, TextStage,
};
