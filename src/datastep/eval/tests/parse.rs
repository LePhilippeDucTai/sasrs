use super::super::*;
use super::*;
use crate::ast::{BinaryOp, Expr};
use crate::value::{MissingKind, Value};

// ── LAG / DIF : files FIFO PAR SITE D'APPEL ──────────────────────────
//
// On réutilise le MÊME `Expr::Call` (donc le même `args.as_ptr()`) et le
// MÊME `EvalCtx` à travers les appels successifs, en mutant la valeur de
// `x` dans le PDV entre chaque exécution — c'est exactement ce que fait la
// boucle implicite du DATA step sur un site lexical donné.

/// Reconnaissance du suffixe numérique.
#[test]
fn parse_lag_dif_recognises_suffix() {
    assert_eq!(parse_lag_dif("LAG"), Some((1, false)));
    assert_eq!(parse_lag_dif("lag"), Some((1, false)));
    assert_eq!(parse_lag_dif("LAG2"), Some((2, false)));
    assert_eq!(parse_lag_dif("DIF"), Some((1, true)));
    assert_eq!(parse_lag_dif("Dif3"), Some((3, true)));
    // Pas un LAG/DIF.
    assert_eq!(parse_lag_dif("LOG"), None);
    // Suffixe non numérique → pas reconnu (laisse passer LAGUERRE & co).
    assert_eq!(parse_lag_dif("LAGX"), None);
}

/// parse_lag_dif : LAG sans chiffre vaut n=1, DIF idem.
#[test]
fn parse_lag_dif_bare_is_one() {
    assert_eq!(parse_lag_dif("LAG"), Some((1, false)));
    assert_eq!(parse_lag_dif("DIF"), Some((1, true)));
}

/// parse_lag_dif : un suffixe à plusieurs chiffres est lu en entier.
#[test]
fn parse_lag_dif_multidigit_suffix() {
    assert_eq!(parse_lag_dif("LAG10"), Some((10, false)));
    assert_eq!(parse_lag_dif("DIF250"), Some((250, true)));
}

/// parse_lag_dif : insensible à la casse, mixte inclus.
#[test]
fn parse_lag_dif_case_insensitive() {
    assert_eq!(parse_lag_dif("Lag3"), Some((3, false)));
    assert_eq!(parse_lag_dif("dIf4"), Some((4, true)));
}

/// parse_lag_dif : ne capture PAS les fonctions homonymes au préfixe
/// (LAGUERRE n'existe pas mais une fonction LOGxxx ne doit pas matcher).
#[test]
fn parse_lag_dif_rejects_non_matching() {
    assert_eq!(parse_lag_dif("LOG"), None);
    assert_eq!(parse_lag_dif("DIFX"), None);
    assert_eq!(parse_lag_dif("LAG2A"), None);
    assert_eq!(parse_lag_dif("X"), None);
}

#[test]
fn lag1_returns_value_from_previous_call() {
    let mut pdv = pdv_with(vec![(num_var("x"), Value::Num(10.0))]);
    let slot = pdv.slot("x").unwrap();
    let e = Expr::Call {
        name: "LAG".to_string(),
        args: vec![var("x")],
    };
    let mut ctx = EvalCtx::default();

    // Appel 1 : x = 10 → LAG renvoie missing (rien encore en file).
    pdv.set(slot, Value::Num(10.0));
    assert_eq!(eval(&e, &pdv, &mut ctx), Value::missing());
    // Appel 2 : x = 20 → LAG renvoie la valeur de l'appel 1 (10).
    pdv.set(slot, Value::Num(20.0));
    assert_eq!(eval(&e, &pdv, &mut ctx), Value::Num(10.0));
    // Appel 3 : x = 30 → LAG renvoie la valeur de l'appel 2 (20).
    pdv.set(slot, Value::Num(30.0));
    assert_eq!(eval(&e, &pdv, &mut ctx), Value::Num(20.0));
}

/// LAG sur une longue séquence : la file ne garde jamais plus de n éléments
/// (invariant interne), et le retard est constant.
#[test]
fn lag1_long_sequence_constant_delay() {
    let e = call("LAG");
    let inputs: Vec<Value> = (1..=20).map(|i| Value::Num(i as f64)).collect();
    let mut expected = vec![Value::missing()];
    expected.extend((1..20).map(|i| Value::Num(i as f64)));
    run_site(&e, &inputs, &expected);
}

#[test]
fn lag2_returns_value_from_two_calls_ago() {
    let mut pdv = pdv_with(vec![(num_var("x"), Value::Num(0.0))]);
    let slot = pdv.slot("x").unwrap();
    let e = Expr::Call {
        name: "LAG2".to_string(),
        args: vec![var("x")],
    };
    let mut ctx = EvalCtx::default();

    let seq = [1.0, 2.0, 3.0, 4.0];
    // n=2 : missing, missing, puis valeur d'il y a 2 exécutions.
    let expected = [
        Value::missing(),
        Value::missing(),
        Value::Num(1.0),
        Value::Num(2.0),
    ];
    for (v, exp) in seq.iter().zip(expected.iter()) {
        pdv.set(slot, Value::Num(*v));
        assert_eq!(&eval(&e, &pdv, &mut ctx), exp);
    }
}

#[test]
fn dif1_computes_difference_with_missing_first() {
    let mut pdv = pdv_with(vec![(num_var("x"), Value::Num(0.0))]);
    let slot = pdv.slot("x").unwrap();
    let e = Expr::Call {
        name: "DIF".to_string(),
        args: vec![var("x")],
    };
    let mut ctx = EvalCtx::default();

    // x : 10, 25, 5 → DIF : ., 15, -20.
    let seq = [10.0, 25.0, 5.0];
    let expected = [Value::missing(), Value::Num(15.0), Value::Num(-20.0)];
    for (v, exp) in seq.iter().zip(expected.iter()) {
        pdv.set(slot, Value::Num(*v));
        assert_eq!(&eval(&e, &pdv, &mut ctx), exp);
    }
}

#[test]
fn dif1_missing_current_yields_missing() {
    let mut pdv = pdv_with(vec![(num_var("x"), Value::Num(0.0))]);
    let slot = pdv.slot("x").unwrap();
    let e = Expr::Call {
        name: "DIF".to_string(),
        args: vec![var("x")],
    };
    let mut ctx = EvalCtx::default();

    pdv.set(slot, Value::Num(10.0));
    assert_eq!(eval(&e, &pdv, &mut ctx), Value::missing()); // 1er : pas de lag
    // x manquant → DIF missing même si un lag existe.
    pdv.set(slot, Value::missing());
    assert_eq!(eval(&e, &pdv, &mut ctx), Value::missing());
    // x de nouveau présent, mais le lag (.) est manquant → missing.
    pdv.set(slot, Value::Num(7.0));
    assert_eq!(eval(&e, &pdv, &mut ctx), Value::missing());
    // x = 8, lag = 7 → 1.
    pdv.set(slot, Value::Num(8.0));
    assert_eq!(eval(&e, &pdv, &mut ctx), Value::Num(1.0));
}

#[test]
fn two_lag_sites_have_independent_queues() {
    // CRUX (pitfall #8) : deux sites LAG lexicaux DISTINCTS sur la MÊME
    // variable ont des files INDÉPENDANTES, indexées par args.as_ptr().
    let mut pdv = pdv_with(vec![(num_var("x"), Value::Num(0.0))]);
    let slot = pdv.slot("x").unwrap();
    let site_a = Expr::Call {
        name: "LAG".to_string(),
        args: vec![var("x")],
    };
    let site_b = Expr::Call {
        name: "LAG".to_string(),
        args: vec![var("x")],
    };
    // Les deux sites doivent avoir des pointeurs d'args distincts.
    if let (Expr::Call { args: a, .. }, Expr::Call { args: b, .. }) = (&site_a, &site_b) {
        assert_ne!(a.as_ptr() as usize, b.as_ptr() as usize);
    }
    let mut ctx = EvalCtx::default();

    // Appel 1 du site A avec x = 100.
    pdv.set(slot, Value::Num(100.0));
    assert_eq!(eval(&site_a, &pdv, &mut ctx), Value::missing());

    // Premier appel du site B avec x = 200 : DOIT renvoyer missing
    // (file propre au site B vide), PAS 100 (qui appartient au site A).
    pdv.set(slot, Value::Num(200.0));
    assert_eq!(eval(&site_b, &pdv, &mut ctx), Value::missing());

    // Deuxième appel du site A avec x = 300 → renvoie 100 (sa file à lui).
    pdv.set(slot, Value::Num(300.0));
    assert_eq!(eval(&site_a, &pdv, &mut ctx), Value::Num(100.0));

    // Deuxième appel du site B avec x = 400 → renvoie 200 (sa file à lui).
    pdv.set(slot, Value::Num(400.0));
    assert_eq!(eval(&site_b, &pdv, &mut ctx), Value::Num(200.0));

    // Deux files distinctes ont bien été créées.
    assert_eq!(ctx.lag_queues.len(), 2);
}

/// LAG3 : missing sur les 3 premiers appels, puis la valeur d'il y a 3.
#[test]
fn lag3_returns_value_from_three_calls_ago() {
    run_site(
        &call("LAG3"),
        &[
            Value::Num(1.0),
            Value::Num(2.0),
            Value::Num(3.0),
            Value::Num(4.0),
            Value::Num(5.0),
        ],
        &[
            Value::missing(),
            Value::missing(),
            Value::missing(),
            Value::Num(1.0),
            Value::Num(2.0),
        ],
    );
}

/// DIF2 : x - LAG2(x). Missing tant que LAG2 n'a pas de valeur.
#[test]
fn dif2_computes_second_difference() {
    // x : 1,2,4,7,11 → DIF2 : .,.,4-1=3,7-2=5,11-4=7
    run_site(
        &call("DIF2"),
        &[
            Value::Num(1.0),
            Value::Num(2.0),
            Value::Num(4.0),
            Value::Num(7.0),
            Value::Num(11.0),
        ],
        &[
            Value::missing(),
            Value::missing(),
            Value::Num(3.0),
            Value::Num(5.0),
            Value::Num(7.0),
        ],
    );
}

/// LAG propage un x manquant dans sa file : le missing ressort au bon rang.
#[test]
fn lag_propagates_missing_input() {
    // x : 5, ., 9 → LAG1 : ., 5, .
    run_site(
        &call("LAG"),
        &[Value::Num(5.0), Value::missing(), Value::Num(9.0)],
        &[Value::missing(), Value::Num(5.0), Value::missing()],
    );
}

/// LAG conserve la valeur missing SPÉCIALE telle quelle (.A reste .A).
#[test]
fn lag_preserves_special_missing() {
    let special = Value::Missing(MissingKind::Letter(0)); // .A
    run_site(
        &call("LAG"),
        &[special.clone(), Value::Num(3.0)],
        &[Value::missing(), special.clone()],
    );
}

/// LAG d'une expression (pas une simple variable) : argument évalué UNE fois,
/// file FIFO sur la valeur calculée.
#[test]
fn lag_of_expression() {
    // site = LAG(x + 1) ; x : 1,2,3 → arg : 2,3,4 → LAG : .,2,3
    let e = Expr::Call {
        name: "LAG".to_string(),
        args: vec![bin(BinaryOp::Add, var("x"), num(1.0))],
    };
    run_site(
        &e,
        &[Value::Num(1.0), Value::Num(2.0), Value::Num(3.0)],
        &[Value::missing(), Value::Num(2.0), Value::Num(3.0)],
    );
}

/// LAG sur valeurs négatives et fractionnaires (pas de troncature).
#[test]
fn lag_handles_negative_and_fractional() {
    run_site(
        &call("LAG"),
        &[Value::Num(-1.5), Value::Num(2.25)],
        &[Value::missing(), Value::Num(-1.5)],
    );
}

/// LAG et DIF au MÊME site lexical seraient impossibles (noms différents),
/// mais LAG(x) et DIF(x) sont deux sites distincts → files distinctes,
/// le DIF n'emprunte pas la file du LAG.
#[test]
fn lag_and_dif_sites_do_not_share_queue() {
    let mut pdv = pdv_with(vec![(num_var("x"), Value::Num(0.0))]);
    let slot = pdv.slot("x").unwrap();
    let lag = call("LAG");
    let dif = call("DIF");
    let mut ctx = EvalCtx::default();

    // Appel 1.
    pdv.set(slot, Value::Num(10.0));
    assert_eq!(eval(&lag, &pdv, &mut ctx), Value::missing());
    assert_eq!(eval(&dif, &pdv, &mut ctx), Value::missing());
    // Appel 2 : chacun amorcé sur sa propre file.
    pdv.set(slot, Value::Num(15.0));
    assert_eq!(eval(&lag, &pdv, &mut ctx), Value::Num(10.0));
    assert_eq!(eval(&dif, &pdv, &mut ctx), Value::Num(5.0));
    assert_eq!(ctx.lag_queues.len(), 2);
}

/// DIF : si la valeur retardée est manquante (poussée plus tôt), le résultat
/// est manquant même quand x courant est présent.
#[test]
fn dif_missing_lagged_yields_missing() {
    // x : ., 10 → DIF1 : . (rien en file), . (lag=., x=10)
    run_site(
        &call("DIF"),
        &[Value::missing(), Value::Num(10.0)],
        &[Value::missing(), Value::missing()],
    );
}

/// DIF : un missing spécial dans x courant rend le résultat manquant.
#[test]
fn dif_special_missing_current_yields_missing() {
    let special = Value::Missing(MissingKind::Letter(25)); // .Z
    run_site(
        &call("DIF"),
        &[Value::Num(4.0), special],
        &[Value::missing(), Value::missing()],
    );
}

/// DIF d'une constante : x - lag(x) = 0 dès que la file est amorcée.
#[test]
fn dif_of_constant_is_zero_after_warmup() {
    run_site(
        &call("DIF"),
        &[Value::Num(7.0), Value::Num(7.0), Value::Num(7.0)],
        &[Value::missing(), Value::Num(0.0), Value::Num(0.0)],
    );
}

/// DIF sur valeurs fractionnaires : différence exacte.
#[test]
fn dif_fractional() {
    run_site(
        &call("DIF"),
        &[Value::Num(1.5), Value::Num(4.0)],
        &[Value::missing(), Value::Num(2.5)],
    );
}

/// Trois sites LAG distincts → trois files indépendantes, indexées ptr.
#[test]
fn three_lag_sites_independent() {
    let mut pdv = pdv_with(vec![(num_var("x"), Value::Num(0.0))]);
    let slot = pdv.slot("x").unwrap();
    let a = call("LAG");
    let b = call("LAG");
    let c = call("LAG2");
    let mut ctx = EvalCtx::default();

    pdv.set(slot, Value::Num(1.0));
    assert_eq!(eval(&a, &pdv, &mut ctx), Value::missing());
    assert_eq!(eval(&b, &pdv, &mut ctx), Value::missing());
    assert_eq!(eval(&c, &pdv, &mut ctx), Value::missing());

    pdv.set(slot, Value::Num(2.0));
    assert_eq!(eval(&a, &pdv, &mut ctx), Value::Num(1.0));
    assert_eq!(eval(&b, &pdv, &mut ctx), Value::Num(1.0));
    assert_eq!(eval(&c, &pdv, &mut ctx), Value::missing());

    pdv.set(slot, Value::Num(3.0));
    assert_eq!(eval(&c, &pdv, &mut ctx), Value::Num(1.0));

    assert_eq!(ctx.lag_queues.len(), 3);
}
