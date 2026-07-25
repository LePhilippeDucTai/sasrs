use super::*;

#[test]
fn special_missing_roundtrip() {
    for kind in [
        MissingKind::Underscore,
        MissingKind::Letter(0),
        MissingKind::Letter(25),
    ] {
        let f = encode_special(kind);
        assert!(f.is_nan());
        assert_eq!(decode_nan(f), kind);
    }
}

#[test]
fn plain_nan_decodes_as_dot() {
    assert_eq!(decode_nan(f64::NAN), MissingKind::Dot);
}

#[test]
fn value_to_num_maps_dot_to_null_and_specials_to_nan() {
    // `.` ordinaire ⇔ null Polars (None), JAMAIS un NaN.
    assert_eq!(value_to_num(&Value::Missing(MissingKind::Dot)), None);
    // Spéciaux ⇔ Some(NaN-payload), JAMAIS un null.
    for kind in [
        MissingKind::Underscore,
        MissingKind::Letter(0),
        MissingKind::Letter(25),
    ] {
        let f = value_to_num(&Value::Missing(kind)).expect("special must be Some");
        assert!(f.is_nan());
        assert_eq!(decode_nan(f), kind);
    }
    // Nombre ordinaire : passe tel quel.
    assert_eq!(value_to_num(&Value::Num(1.5)), Some(1.5));
}

#[test]
fn num_to_value_inverts_value_to_num() {
    for v in [
        Value::Num(2.0),
        Value::missing(),
        Value::Missing(MissingKind::Underscore),
        Value::Missing(MissingKind::Letter(0)),
        Value::Missing(MissingKind::Letter(25)),
    ] {
        assert_eq!(num_to_value(value_to_num(&v)), v);
    }
}

#[test]
fn nullify_specials_turns_special_nans_into_nulls() {
    let df = polars::df!(
        "x" => [Some(encode_special(MissingKind::Letter(0))), Some(1.0), None],
    )
    .unwrap();
    let out = nullify_specials(&df).unwrap();
    let col = out.column("x").unwrap().f64().unwrap();
    assert_eq!(col.null_count(), 2);
    assert_eq!(col.get(1), Some(1.0));
}
