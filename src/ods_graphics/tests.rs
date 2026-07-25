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
