use super::*;
use crate::dataset::{SasDataset, VarMeta};
use polars::df;

// ── ARRAY (M2, lot 3) ────────────────────────────────────────────────

#[test]
fn array_elements_enter_pdv_in_order_and_registry_is_filled() {
    let mut s = session();
    let prog = compile_src("data o; array a{3} x y z; b = 1; run;", &mut s).unwrap();
    let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
    // Les éléments entrent au PDV au point de l'ARRAY, avant b.
    assert_eq!(names, vec!["x", "y", "z", "b"]);
    assert_eq!(prog.arrays.get("A").map(|d| &d.slots), Some(&vec![0, 1, 2]));
    assert_eq!(prog.pdv.vars()[0].ty, VarType::Num);
    // Le nom de l'array n'est PAS une variable du PDV.
    assert!(prog.pdv.slot("a").is_none());
}

#[test]
fn array_star_size_deduced_and_char_length_applied() {
    let mut s = session();
    let prog = compile_src("data o; array c{*} $ 5 c1 c2; run;", &mut s).unwrap();
    assert_eq!(prog.arrays.get("C").map(|d| &d.slots), Some(&vec![0, 1]));
    for v in prog.pdv.vars() {
        assert_eq!(v.ty, VarType::Char);
        assert_eq!(v.length, 5);
    }
}

#[test]
fn array_auto_named_elements() {
    let mut s = session();
    let prog = compile_src("data o; array a{3}; a{1} = 1; run;", &mut s).unwrap();
    let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["a1", "a2", "a3"]);
    assert_eq!(prog.arrays.get("A").map(|d| &d.slots), Some(&vec![0, 1, 2]));
}

#[test]
fn array_size_mismatch_errors() {
    let mut s = session();
    let err = compile_src("data o; array a{3} x y; run;", &mut s)
        .err()
        .unwrap();
    assert!(err.to_string().contains("does not match"), "got: {err}");
    let err = compile_src("data o; array a{2} x y z; run;", &mut s)
        .err()
        .unwrap();
    assert!(err.to_string().contains("does not match"), "got: {err}");
}

#[test]
fn array_star_without_vars_errors() {
    let mut s = session();
    let err = compile_src("data o; array a{*}; run;", &mut s)
        .err()
        .unwrap();
    assert!(err.to_string().contains("zero elements"), "got: {err}");
}

#[test]
fn array_redeclaration_errors() {
    let mut s = session();
    let err = compile_src("data o; array a{2} x y; array a{2} u v; run;", &mut s)
        .err()
        .unwrap();
    assert!(
        err.to_string().contains("already been defined"),
        "got: {err}"
    );
}

#[test]
fn array_indexed_assignment_marks_elements_initialized() {
    let mut s = session();
    let prog = compile_src(
        "data o; array a{3} x y z; do i = 1 to 3; a{i} = i; end; run;",
        &mut s,
    )
    .unwrap();
    // x, y, z assignés via l'indice : pas de NOTE uninitialized.
    assert!(prog.uninitialized.is_empty());
}

#[test]
fn array_indexed_rvalue_infers_element_type() {
    let mut s = session();
    let prog = compile_src(
        "data o; array c{2} $ 4 u v; s = c{1}; t = c(2); n = a(1); array a{2} p q; run;",
        &mut s,
    );
    // `a` est déclaré APRÈS son usage en forme parenthèses : Call normal
    // (fonction inconnue à l'évaluation) — la compilation passe et
    // infère Num. On vérifie surtout s et t.
    let prog = prog.unwrap();
    let var = |n: &str| &prog.pdv.vars()[prog.pdv.slot(n).unwrap()];
    assert_eq!((var("s").ty, var("s").length), (VarType::Char, 4));
    assert_eq!((var("t").ty, var("t").length), (VarType::Char, 4));
    assert_eq!(var("n").ty, VarType::Num);
}

#[test]
fn undeclared_array_lvalue_errors() {
    let mut s = session();
    // Forme accolades.
    let err = compile_src("data o; nosuch{1} = 0; run;", &mut s)
        .err()
        .unwrap();
    assert!(
        err.to_string().contains("Undeclared array referenced"),
        "got: {err}"
    );
    // Forme parenthèses : validée array à la COMPILATION.
    let err = compile_src("data o; nosuch(1) = 0; run;", &mut s)
        .err()
        .unwrap();
    assert!(
        err.to_string().contains("Undeclared array referenced"),
        "got: {err}"
    );
}

#[test]
fn undeclared_array_rvalue_errors() {
    let mut s = session();
    let err = compile_src("data o; x = nosuch{1}; run;", &mut s)
        .err()
        .unwrap();
    assert!(
        err.to_string().contains("Undeclared array referenced"),
        "got: {err}"
    );
}

#[test]
fn dim_of_array_does_not_create_variable() {
    let mut s = session();
    let prog = compile_src("data o; array a{3} x y z; n = dim(a); run;", &mut s).unwrap();
    // Pas de variable `a` au PDV, et n est bien là.
    assert!(prog.pdv.slot("a").is_none());
    assert!(prog.pdv.slot("n").is_some());
    let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["x", "y", "z", "n"]);
}

#[test]
fn bare_array_name_reference_errors() {
    let mut s = session();
    // Un nom d'array n'est pas une variable : référence nue illégale.
    let err = compile_src("data o; array a{2} x y; z = a; run;", &mut s)
        .err()
        .unwrap();
    assert!(err.to_string().contains("Illegal reference"), "got: {err}");
    let err = compile_src("data o; array a{2} x y; a = 1; run;", &mut s)
        .err()
        .unwrap();
    assert!(err.to_string().contains("Illegal reference"), "got: {err}");
}

#[test]
fn put_width_parsing() {
    let mut s = session();
    // Chiffres finaux du nom du format (`best12` → 12), sinon 200 — cas de
    // base inchangé depuis avant M43.1 (aucun format utilisateur en jeu).
    let prog = compile_src(
        "data o; a = put(1, best12.); b = put(1, date9.); c = put(1, words.); \
         d = put(1, dollar10.2); e = put(1, percent8.1); f = put(1, comma12.); run;",
        &mut s,
    )
    .unwrap();
    let len = |n: &str| prog.pdv.vars()[prog.pdv.slot(n).unwrap()].length;
    assert_eq!(len("a"), 12);
    assert_eq!(len("b"), 9);
    // Format inconnu sans chiffres de largeur → fallback générique 200.
    assert_eq!(len("c"), 200);
    // Forme w.d : la largeur est `w`, pas le nombre de décimales.
    assert_eq!(len("d"), 10);
    assert_eq!(len("e"), 8);
    assert_eq!(len("f"), 12);
}

// ── M43.1 — `put_width` doit refléter MIN=/MAX= des formats VALUE ────

#[test]
fn put_width_user_format_min_widens_literal_token_width() {
    // narrowfmt (min=8) : la largeur littérale du token (`narrowfmt4.` → 4)
    // est TROP ÉTROITE pour la sortie réelle (MIN=8 l'élargit à l'exécution).
    // Avant le fix, put_width restait aveuglément à 4 → troncature au
    // runtime. Après le fix, la longueur PDV inférée doit être 8.
    let mut s = session();
    define_format(
        &mut s,
        "proc format; value narrowfmt (min=8 max=10) low-<21='Minor' 21-high='Adult'; run;",
    );
    let prog = compile_src(
        "data o; x = put(5, narrowfmt8.); y = put(5, narrowfmt4.); z = put(5, narrowfmt.); run;",
        &mut s,
    )
    .unwrap();
    let len = |n: &str| prog.pdv.vars()[prog.pdv.slot(n).unwrap()].length;
    // Largeur explicite déjà dans [MIN,MAX] : inchangée.
    assert_eq!(len("x"), 8);
    // Largeur explicite 4 < MIN=8 : clampée à 8, pas tronquée à 4.
    assert_eq!(len("y"), 8);
    // Pas de largeur explicite : label le plus long ("Minor", 5) clampé par
    // MIN=8 → 8 (PAS le fallback générique 200).
    assert_eq!(len("z"), 8);
}

#[test]
fn put_width_user_format_max_narrows_literal_token_width() {
    // widefmt (max=5) : la largeur littérale du token (`widefmt20.` → 20)
    // est TROP LARGE (MAX=5 la rétrécit à l'exécution).
    let mut s = session();
    define_format(
        &mut s,
        "proc format; value widefmt (max=5) 1='AB' 2='ABCDE' other='Z'; run;",
    );
    let prog = compile_src("data o; y = put(1, widefmt20.); run;", &mut s).unwrap();
    let len = |n: &str| prog.pdv.vars()[prog.pdv.slot(n).unwrap()].length;
    assert_eq!(len("y"), 5);
}

#[test]
fn put_width_user_format_without_options_matches_legacy_behavior() {
    // Format utilisateur SANS MIN=/MAX=/DEFAULT= (cas historique, majorité
    // des formats existants) : inféré exactement comme avant M43.1, que le
    // format soit résolu dans le catalogue ou non.
    let mut s = session();
    define_format(
        &mut s,
        "proc format; value sexfmt 1='Male' 2='Female' other='Unknown'; run;",
    );
    let prog = compile_src(
        "data o; a = put(1, sexfmt4.); b = put(1, sexfmt.); run;",
        &mut s,
    )
    .unwrap();
    let len = |n: &str| prog.pdv.vars()[prog.pdv.slot(n).unwrap()].length;
    // Largeur explicite : inchangée (spec.w brut).
    assert_eq!(len("a"), 4);
    // Pas de largeur explicite, pas d'options M43.1 : fallback générique 200
    // (comportement historique — sortie non bornée, imprévisible à la
    // compilation).
    assert_eq!(len("b"), 200);
}

#[test]
fn put_width_builtin_format_unaffected_by_catalog_lookup() {
    // Un format BUILTIN (jamais dans le catalogue utilisateur) reste
    // entièrement gouverné par les chiffres littéraux du token, même en
    // présence d'un format utilisateur avec MIN=/MAX= dans le même catalogue.
    let mut s = session();
    define_format(
        &mut s,
        "proc format; value narrowfmt (min=8 max=10) low-<21='Minor' 21-high='Adult'; run;",
    );
    let prog = compile_src("data o; y = put(1, best12.); run;", &mut s).unwrap();
    let len = |n: &str| prog.pdv.vars()[prog.pdv.slot(n).unwrap()].length;
    assert_eq!(len("y"), 12);
}

// ── Options de dataset + OUTPUT ciblé (M2, lot 4) ────────────────────

#[test]
fn input_keep_filters_pdv() {
    let mut s = session();
    write_class(&s, "inp");
    let prog = compile_src("data o; set inp(keep=name); run;", &mut s).unwrap();
    // Seule Name entre au PDV (Age filtrée AVANT le PDV).
    let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["Name"]);
    let input = &prog.input.as_ref().unwrap().datasets[0];
    assert_eq!(input.columns.len(), 1);
    assert_eq!(input.var_slots, vec![0]);
    assert_eq!(prog.outputs[0].kept_slots, vec![0]);
}

#[test]
fn input_drop_filters_pdv() {
    let mut s = session();
    write_class(&s, "inp");
    let prog = compile_src("data o; set inp(drop=age); run;", &mut s).unwrap();
    let names: Vec<&str> = prog.pdv.vars().iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["Name"]);
}

#[test]
fn input_rename_renames_pdv_slot() {
    let mut s = session();
    write_class(&s, "inp");
    let prog = compile_src(
        "data o; set inp(rename=(age=years)); x = years; run;",
        &mut s,
    )
    .unwrap();
    assert!(prog.pdv.slot("years").is_some());
    assert!(prog.pdv.slot("age").is_none());
    // Le slot renommé reste from_input (pas de reset par itération).
    let slot = prog.pdv.slot("years").unwrap();
    assert!(prog.pdv.vars()[slot].from_input);
}

#[test]
fn input_where_is_stored_not_filtered_at_compile() {
    let mut s = session();
    write_class(&s, "inp");
    let prog = compile_src("data o; set inp(where=(age > 13)); run;", &mut s).unwrap();
    let input = &prog.input.as_ref().unwrap().datasets[0];
    // Pas de filtrage à la compilation : toutes les lignes présentes.
    assert_eq!(input.n_rows, 3);
    assert!(input.where_.is_some());
}

#[test]
fn input_where_unknown_variable_errors() {
    let mut s = session();
    write_class(&s, "inp");
    let err = compile_src("data o; set inp(where=(nosuch > 1)); run;", &mut s)
        .err()
        .unwrap();
    assert!(
        err.to_string()
            .contains("Variable nosuch is not on file WORK.INP."),
        "got: {err}"
    );
}

#[test]
fn output_option_keep_drop_combined_with_statements() {
    let mut s = session();
    // PDV : x y z. Statement keep x y ; option drop=y → x seul.
    let prog = compile_src(
        "data o(drop=y); x = 1; y = 2; z = 3; keep x y; run;",
        &mut s,
    )
    .unwrap();
    assert_eq!(prog.outputs[0].kept_slots, vec![0]);
    assert_eq!(prog.outputs[0].out_names, vec!["x".to_string()]);
}

#[test]
fn output_option_keep_is_per_output() {
    let mut s = session();
    let prog = compile_src("data a(keep=x) b; x = 1; y = 2; run;", &mut s).unwrap();
    assert_eq!(prog.outputs[0].kept_slots, vec![0]);
    assert_eq!(prog.outputs[1].kept_slots, vec![0, 1]);
}

#[test]
fn output_rename_changes_written_name_not_slot() {
    let mut s = session();
    let prog = compile_src("data o(rename=(x=xx)); x = 1; y = 2; run;", &mut s).unwrap();
    // Le slot PDV garde son nom ; seul le nom d'écriture change.
    assert_eq!(prog.pdv.vars()[0].name, "x");
    assert_eq!(
        prog.outputs[0].out_names,
        vec!["xx".to_string(), "y".to_string()]
    );
    assert_eq!(prog.outputs[0].kept_slots, vec![0, 1]);
}

#[test]
fn where_on_output_dataset_errors() {
    let mut s = session();
    let err = compile_src("data o(where=(x > 1)); x = 1; run;", &mut s)
        .err()
        .unwrap();
    assert_eq!(
        err.to_string(),
        "WHERE= is not a valid data set option for output data sets."
    );
}

#[test]
fn targeted_output_unknown_dataset_errors() {
    let mut s = session();
    let err = compile_src("data a b; x = 1; output c; run;", &mut s)
        .err()
        .unwrap();
    assert_eq!(
        err.to_string(),
        "Output dataset WORK.C is not in the DATA statement output list."
    );
}

#[test]
fn targeted_output_known_dataset_compiles() {
    let mut s = session();
    let prog = compile_src("data a b; x = 1; output a; output a b; run;", &mut s).unwrap();
    assert!(prog.has_explicit_output);
}

#[test]
fn option_variable_never_referenced_errors() {
    let mut s = session();
    write_class(&s, "inp");
    // En entrée : keep= d'une variable absente de l'input.
    let err = compile_src("data o; set inp(keep=nosuch); run;", &mut s)
        .err()
        .unwrap();
    assert!(
        err.to_string()
            .contains("in the DROP, KEEP, or RENAME list has never been referenced"),
        "got: {err}"
    );
    // En entrée : rename= d'une variable absente.
    let err = compile_src("data o; set inp(rename=(nosuch=x)); run;", &mut s)
        .err()
        .unwrap();
    assert!(
        err.to_string().contains("has never been referenced"),
        "got: {err}"
    );
    // En sortie : drop= d'une variable absente du PDV.
    let err = compile_src("data o(drop=nosuch); x = 1; run;", &mut s)
        .err()
        .unwrap();
    assert!(
        err.to_string().contains("has never been referenced"),
        "got: {err}"
    );
}

#[test]
fn incompatible_types_across_set_datasets_error() {
    let mut s = session();
    write_class(&s, "a"); // Age numérique
    // Dataset où Age est CARACTÈRE.
    let df = df!("Age" => ["x", "y"]).unwrap();
    let vars = vec![VarMeta {
        name: "Age".into(),
        ty: VarType::Char,
        length: 1,
        format: None,
        label: None,
    }];
    s.libs
        .get("WORK")
        .unwrap()
        .write("cage", &SasDataset { df, vars })
        .unwrap();
    let err = compile_src("data o; set a cage; run;", &mut s)
        .err()
        .unwrap();
    assert_eq!(
        err.to_string(),
        "Variable Age has been defined as both character and numeric."
    );
}

#[test]
fn by_without_set_errors() {
    let mut s = session();
    let err = compile_src("data o; by x; x = 1; run;", &mut s)
        .err()
        .unwrap();
    assert_eq!(
        err.to_string(),
        "No SET, MERGE, UPDATE, or MODIFY statement."
    );
}

#[test]
fn by_variable_missing_from_one_dataset_errors() {
    let mut s = session();
    write_class(&s, "a"); // Age, Name
    write_weights(&s, "b"); // Age, Weight (pas de Name)
    let err = compile_src("data o; set a b; by name; run;", &mut s)
        .err()
        .unwrap();
    assert_eq!(
        err.to_string(),
        "BY variable NAME is not on input data set WORK.B."
    );
    // Variable BY absente de TOUS les inputs.
    let err = compile_src("data o; set a; by nosuch; run;", &mut s)
        .err()
        .unwrap();
    assert!(
        err.to_string()
            .contains("BY variable nosuch is not on input data set"),
        "got: {err}"
    );
}

#[test]
fn renamed_but_dropped_input_variable_is_ignored() {
    let mut s = session();
    write_class(&s, "inp");
    // age est dropée : le rename la concernant est ignoré (pas d'erreur,
    // pas de variable years).
    let prog = compile_src("data o; set inp(drop=age rename=(age=years)); run;", &mut s).unwrap();
    assert!(prog.pdv.slot("years").is_none());
    assert!(prog.pdv.slot("age").is_none());
    assert!(prog.pdv.slot("name").is_some());
}
