use super::super::*;
use super::*;

// ── Parse tests ──────────────────────────────────────────────────────

#[test]
fn test_parse_poisson_log() {
    let ast = parse_genmod("proc genmod; model y = x / dist=poisson link=log; run;").unwrap();
    let m = ast.model.unwrap();
    assert_eq!(m.dist, Distribution::Poisson);
    assert_eq!(m.link, LinkFunction::Log);
}

#[test]
fn test_parse_binomial_logit() {
    // dist=binomial without explicit link → canonical Logit
    let ast = parse_genmod("proc genmod; model y = x / dist=binomial; run;").unwrap();
    let m = ast.model.unwrap();
    assert_eq!(m.dist, Distribution::Binomial);
    assert_eq!(m.link, LinkFunction::Logit);
}

#[test]
fn test_parse_normal_identity() {
    // dist=normal without explicit link → canonical Identity
    let ast = parse_genmod("proc genmod; model y = x / dist=normal; run;").unwrap();
    let m = ast.model.unwrap();
    assert_eq!(m.dist, Distribution::Normal);
    assert_eq!(m.link, LinkFunction::Identity);
}

#[test]
fn test_parse_descending() {
    let ast = parse_genmod("proc genmod; model y(descending) = x / dist=binomial; run;").unwrap();
    assert!(ast.model.unwrap().descending);
}

#[test]
fn test_parse_event() {
    let ast = parse_genmod("proc genmod; model y(event='1') = x / dist=binomial; run;").unwrap();
    assert_eq!(ast.model.unwrap().event, Some("1".to_string()));
}

#[test]
fn test_parse_gamma_ok() {
    // Parse should succeed (error deferred to execute)
    let ast = parse_genmod("proc genmod; model y = x / dist=gamma; run;");
    assert!(ast.is_ok(), "DIST=GAMMA parse should succeed");
    assert_eq!(ast.unwrap().model.unwrap().dist, Distribution::Gamma);
}

#[test]
fn test_parse_gamma_default_link_reciprocal() {
    // DIST=GAMMA without an explicit LINK= → canonical reciprocal.
    let ast = parse_genmod("proc genmod; model y = x / dist=gamma; run;").unwrap();
    let m = ast.model.unwrap();
    assert_eq!(m.dist, Distribution::Gamma);
    assert_eq!(m.link, LinkFunction::Reciprocal);
}

#[test]
fn test_parse_gamma_link_log() {
    let ast = parse_genmod("proc genmod; model y = x / dist=gamma link=log; run;").unwrap();
    let m = ast.model.unwrap();
    assert_eq!(m.dist, Distribution::Gamma);
    assert_eq!(m.link, LinkFunction::Log);
}

#[test]
fn test_parse_scale_noscale() {
    let ast = parse_genmod("proc genmod; model y = x / dist=normal noscale; run;").unwrap();
    assert!(ast.model.unwrap().noscale);
    let ast2 = parse_genmod("proc genmod; model y = x / dist=normal scale=2.5; run;").unwrap();
    assert_eq!(ast2.model.unwrap().scale, Some(2.5));
}

#[test]
fn test_execute_poisson_beta0() {
    let (mut session, ast) = make_poisson_session();
    let mut ast2 = ast.clone();
    ast2.model.as_mut().unwrap().noprint = true;

    // Run directly and check beta via log — use execute with noprint off
    // to exercise the path, but check values through listing
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    // β₀ ≈ ln(2) = 0.6931
    assert!(
        listing.contains("0.6931") || listing.contains("0.693"),
        "β₀ not found: {listing}"
    );
}

#[test]
fn test_execute_poisson_beta1() {
    let listing = run_poisson();
    // β₁ ≈ ln(5/2) = 0.9163
    assert!(
        listing.contains("0.9163") || listing.contains("0.916"),
        "β₁ not found: {listing}"
    );
}

#[test]
fn test_execute_poisson_se() {
    let listing = run_poisson();
    // SE(β₁) ≈ 0.4830
    assert!(
        listing.contains("0.4830") || listing.contains("0.483"),
        "SE(β₁) not found: {listing}"
    );
}

#[test]
fn test_execute_normal_beta() {
    let (mut session, ast) = make_normal_session();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    // β₀ = 2.0000, β₁ = 3.0000
    assert!(
        listing.contains("2.0000") || listing.contains("2.000"),
        "β₀ not found: {listing}"
    );
    assert!(
        listing.contains("3.0000") || listing.contains("3.000"),
        "β₁ not found: {listing}"
    );
}

#[test]
fn test_execute_normal_scale() {
    let (mut session, ast) = make_normal_session();
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();
    // Scale = sqrt(MSE) = sqrt(1.0) = 1.0000
    assert!(
        listing.contains("1.0000") || listing.contains("1.000"),
        "Scale not found: {listing}"
    );
}

#[test]
fn test_gamma_intercept_only_log_link() {
    // LINK=LOG intercept-only Gamma ⇒ β̂₀ = ln(ȳ); ȳ = 3.5.
    let est = gamma_intercept_estimate(LinkFunction::Log);
    let expected = (3.5_f64).ln();
    assert!(
        (est - expected).abs() < 1e-3,
        "log-link intercept {est} vs ln(3.5)={expected}"
    );
}

#[test]
fn test_gamma_intercept_only_reciprocal_link() {
    // Canonical reciprocal intercept-only Gamma ⇒ β̂₀ = 1/ȳ; ȳ = 3.5.
    let est = gamma_intercept_estimate(LinkFunction::Reciprocal);
    let expected = 1.0 / 3.5;
    assert!(
        (est - expected).abs() < 1e-3,
        "reciprocal-link intercept {est} vs 1/3.5={expected}"
    );
}

#[test]
fn test_gamma_pearson_dispersion() {
    // Independently verify the Pearson dispersion φ̂ for an intercept-only
    // reciprocal-link Gamma: μ̂ = ȳ for every obs, so
    //   φ̂ = (1/(n−1)) Σ (y−ȳ)²/ȳ².
    let (mut session, mut ast) = make_gamma_intercept_session(LinkFunction::Reciprocal);
    ast.model.as_mut().unwrap().noprint = false;
    execute(&ast, &mut session).unwrap();
    let listing = session.listing.take_string();

    let y = [1.0_f64, 2.0, 3.0, 4.0, 5.0, 6.0];
    let ybar = 3.5;
    let phi: f64 = y
        .iter()
        .map(|v| (v - ybar).powi(2) / (ybar * ybar))
        .sum::<f64>()
        / 5.0;
    // Reported Scale = 1/φ.
    let scale = 1.0 / phi;
    let scale_str = format!("{scale:.4}");
    assert!(
        listing.contains(&scale_str),
        "expected Scale={scale_str} (1/φ) in listing:\n{listing}"
    );
}
