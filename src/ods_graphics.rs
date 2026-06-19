//! M29.1 — Infrastructure ODS GRAPHICS.
//!
//! Cette infrastructure porte l'état de session pour la génération d'images
//! (`ODS GRAPHICS ON / OFF`) ainsi que les paramètres de rendu (dimensions,
//! format d'image, répertoire de sortie, préfixe de nommage).
//!
//! # Invariant byte-identical
//!
//! Tout le code de CE module est compilé dans le build par défaut (sans la
//! feature `graphics`) : il ne fait QUE porter de l'état et émettre des NOTE
//! de log. Aucune image n'est produite sans `--features graphics` — c'est le
//! moteur de rendu (`crate::graphics::render`, gated) qui matérialise les
//! fichiers. PROC SGPLOT (M29.2) consultera cet état ; tant qu'aucun PROC
//! graphique n'existe, activer/désactiver ODS GRAPHICS est inerte sur la
//! sortie (juste des NOTE).

use std::path::PathBuf;

/// Format d'image de sortie pour ODS GRAPHICS.
///
/// Vit dans ce module NON gardé par la feature afin que le parsing et l'état
/// de session restent disponibles dans le build par défaut. Le moteur de
/// rendu (gardé par `#[cfg(feature = "graphics")]`) l'importe d'ici.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFmt {
    /// PNG (défaut SAS).
    Png,
    /// SVG (vectoriel).
    Svg,
}

impl ImageFmt {
    /// Extension de fichier (sans le point) correspondant au format.
    pub fn extension(self) -> &'static str {
        match self {
            ImageFmt::Png => "png",
            ImageFmt::Svg => "svg",
        }
    }
}

impl Default for ImageFmt {
    fn default() -> Self {
        ImageFmt::Png
    }
}

/// État ODS GRAPHICS de la session.
///
/// `enabled` est un état GLOBAL persistant : `ODS GRAPHICS ON` l'active jusqu'à
/// un `ODS GRAPHICS OFF` explicite (il survit aux `RUN;` et aux bornes de PROC).
/// Les dimensions et le format persistent eux aussi entre statements.
#[derive(Debug, Clone)]
pub struct OdsGraphics {
    /// `ODS GRAPHICS ON` → true ; `ODS GRAPHICS OFF` → false. Défaut : false.
    pub enabled: bool,
    /// Largeur de l'image en pixels. Défaut SAS : 800.
    pub width: u32,
    /// Hauteur de l'image en pixels. Défaut SAS : 600.
    pub height: u32,
    /// Format d'image. Défaut : PNG.
    pub image_format: ImageFmt,
    /// Répertoire de sortie pour les images générées.
    pub output_dir: PathBuf,
    /// Préfixe optionnel pour le nommage des fichiers (`IMAGENAME=`).
    pub file_stem: Option<String>,
}

impl OdsGraphics {
    /// Crée un état ODS GRAPHICS avec les défauts SAS, les images étant écrites
    /// dans `output_dir` (typiquement le `base_dir` de la session).
    pub fn new(output_dir: PathBuf) -> Self {
        OdsGraphics {
            enabled: false,
            width: 800,
            height: 600,
            image_format: ImageFmt::default(),
            output_dir,
            file_stem: None,
        }
    }
}

impl Default for OdsGraphics {
    fn default() -> Self {
        OdsGraphics::new(PathBuf::from("."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sas_defaults() {
        let g = OdsGraphics::default();
        assert!(!g.enabled);
        assert_eq!(g.width, 800);
        assert_eq!(g.height, 600);
        assert_eq!(g.image_format, ImageFmt::Png);
    }

    #[test]
    fn imagefmt_extension() {
        assert_eq!(ImageFmt::Png.extension(), "png");
        assert_eq!(ImageFmt::Svg.extension(), "svg");
    }
}
