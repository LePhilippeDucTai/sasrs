//! M29.1 — Module graphique.
//!
//! Le sous-module `render` (implémentation `plotters`) n'est compilé qu'avec la
//! feature `graphics`. Sans cette feature, ce module est essentiellement vide :
//! l'infrastructure d'état ODS GRAPHICS vit dans [`crate::ods_graphics`] et reste
//! disponible dans tous les builds.

#[cfg(feature = "graphics")]
pub mod render;
