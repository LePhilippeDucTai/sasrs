use super::*;
use crate::missing::{decode_nan, encode_special};
use crate::testkit::*;
use crate::value::MissingKind;
use polars::df;

/// Garantie centrale des missings spéciaux : le NaN-payload survit
/// BIT À BIT à write_parquet → read_parquet (parquet stocke les
/// doubles tels quels ; Polars ne canonicalise pas le NaN). Si ce
/// test casse un jour (canonicalisation), c'est un blocage à
/// remonter — pas à contourner par un encodage parallèle.
#[test]
fn parquet_roundtrip_preserves_special_missing_nan_payloads() {
    let kinds = [
        MissingKind::Letter(0),  // .A
        MissingKind::Underscore, // ._
        MissingKind::Letter(25), // .Z
    ];
    let vals: Vec<Option<f64>> = kinds
        .iter()
        .map(|k| Some(encode_special(*k)))
        .chain([None, Some(1.5)]) // `.` ordinaire = null, et un nombre.
        .collect();
    let df = df!("x" => &vals).unwrap();
    let ds = SasDataset {
        df,
        vars: vec![num_meta("x")],
    };

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.parquet");
    ds.write_parquet(&path).unwrap();
    let (back, notes) = SasDataset::read_parquet(&path).unwrap();
    assert!(notes.is_empty(), "unexpected coercion notes: {notes:?}");

    let col = back.df.column("x").unwrap().f64().unwrap();
    // `.` ordinaire : null Polars — et UN SEUL null dans la colonne
    // (les spéciaux ne sont PAS des nulls).
    assert_eq!(col.null_count(), 1);
    assert_eq!(col.get(3), None);
    // Spéciaux : des NaN (pas des nulls) dont le payload est intact.
    for (i, kind) in kinds.iter().enumerate() {
        let v = col.get(i).expect("special missing must not be null");
        assert!(v.is_nan());
        assert_eq!(
            v.to_bits(),
            encode_special(*kind).to_bits(),
            "parquet canonicalized the NaN payload for {kind:?}"
        );
        assert_eq!(decode_nan(v), *kind);
    }
    // Et un nombre ordinaire passe inchangé.
    assert_eq!(col.get(4), Some(1.5));
}
