use super::super::*;
use super::*;

// ───── M45.2 : option `/ NORMAL` des instructions graphiques ─────────────

#[test]
fn plot_normal_option_is_captured() {
    for (src, expect) in [
        ("proc univariate; var x; histogram x / normal; run;", true),
        ("proc univariate; var x; histogram x; run;", false),
        ("proc univariate; var x; qqplot x / normal; run;", true),
        ("proc univariate; var x; probplot x / normal; run;", true),
        ("proc univariate; var x; cdfplot x / normal; run;", true),
        ("proc univariate; var x; ppplot x; run;", false),
        // Sans variable explicite : l'option reste lue.
        ("proc univariate; var x; histogram / normal; run;", true),
        // Casse indifférente.
        ("proc univariate; var x; histogram x / NORMAL; run;", true),
    ] {
        let ast = parse_univ(src).unwrap();
        assert_eq!(ast.plots.len(), 1, "{src}");
        assert_eq!(ast.plots[0].normal, expect, "{src}");
    }
}

#[test]
fn plot_normal_option_order_and_other_options_tolerated() {
    // Ordre libre, autres options ignorées, liste parenthésée acceptée — la
    // règle pragmatique documentée (on cherche le mot-clé, on ne parse pas la
    // grammaire complète des options).
    for src in [
        "proc univariate; var x; histogram x / midpoints=1 to 10 normal; run;",
        "proc univariate; var x; histogram x / normal midpoints=1 to 10; run;",
        "proc univariate; var x; histogram x / normal(mu=3 sigma=2); run;",
        "proc univariate; var x; histogram x / noprint normal cfill=blue; run;",
    ] {
        let ast = parse_univ(src).unwrap();
        assert_eq!(ast.plots.len(), 1, "{src}");
        assert!(ast.plots[0].normal, "{src}");
    }

    // Une option INCONNUE seule ne doit pas déclencher l'ajustement.
    let ast = parse_univ("proc univariate; var x; histogram x / midpoints=1 to 10; run;").unwrap();
    assert!(!ast.plots[0].normal);
}

#[test]
fn plot_option_scan_does_not_overrun_the_statement() {
    // Le scan des options s'arrête au `;` : les instructions suivantes sont
    // parsées normalement (non-régression du parcours du corps du PROC).
    let ast =
        parse_univ("proc univariate; var x y; histogram x / normal; qqplot y; weight w; run;")
            .unwrap();
    assert_eq!(ast.plots.len(), 2);
    assert!(ast.plots[0].normal);
    assert!(!ast.plots[1].normal);
    assert_eq!(ast.plots[1].var.as_deref(), Some("y"));
    assert_eq!(ast.weight.as_deref(), Some("w"));
    assert_eq!(ast.var, vec!["x".to_string(), "y".to_string()]);
}

// ───── paramètres ajustés (μ̂, σ̂) ────────────────────────────────────────

#[test]
fn fitted_normal_params_unweighted_matches_moments() {
    // x = [2,4,4,4,5,5,7,9] : moyenne 5, Σ(x-5)² = 9+1+1+1+0+0+4+16 = 32,
    // s² = 32/7 (VARDEF=DF) ⇒ s = √(32/7) = 2.1380899…
    let values: Vec<Value> = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]
        .iter()
        .map(|&v| Value::Num(v))
        .collect();
    let rows: Vec<usize> = (0..values.len()).collect();
    let (mu, sigma) = fitted_normal_params(&values, None, &rows).unwrap();
    assert!((mu - 5.0).abs() < 1e-12, "mu = {mu}");
    assert!(
        (sigma - (32.0_f64 / 7.0).sqrt()).abs() < 1e-12,
        "sigma = {sigma}"
    );
}

#[test]
fn fitted_normal_params_weighted_uses_weighted_moments() {
    // Le jeu de m33 : x=[1,2,3,4], w=[1,2,3,4] ⇒ μ̂ = 3, σ̂² = 10/3.
    let values: Vec<Value> = [1.0, 2.0, 3.0, 4.0]
        .iter()
        .map(|&v| Value::Num(v))
        .collect();
    let weights: Vec<Value> = [1.0, 2.0, 3.0, 4.0]
        .iter()
        .map(|&v| Value::Num(v))
        .collect();
    let rows: Vec<usize> = (0..4).collect();
    let (mu, sigma) = fitted_normal_params(&values, Some(&weights), &rows).unwrap();
    assert!((mu - 3.0).abs() < 1e-12, "mu = {mu}");
    assert!(
        (sigma * sigma - 10.0 / 3.0).abs() < 1e-12,
        "sigma^2 = {}",
        sigma * sigma
    );
}

#[test]
fn fitted_normal_params_degenerate_cases() {
    let rows: Vec<usize> = (0..2).collect();
    // Une seule observation utilisable → σ̂ indéfini.
    let one = vec![Value::Num(1.0), Value::missing()];
    assert!(fitted_normal_params(&one, None, &rows[..1]).is_none());
    assert!(fitted_normal_params(&one, None, &rows).is_none());
    // Toutes les valeurs égales → σ̂ = 0 (l'appelant n'émet alors rien).
    let flat: Vec<Value> = vec![Value::Num(3.0); 4];
    let all: Vec<usize> = (0..4).collect();
    let (mu, sigma) = fitted_normal_params(&flat, None, &all).unwrap();
    assert_eq!((mu, sigma), (3.0, 0.0));
}

// ───── rendu (overlay) — uniquement sous `--features graphics` ───────────

#[cfg(feature = "graphics")]
mod overlay {
    use super::*;
    use crate::procs::univariate::plot_graphics::normal_overlay as overlay_of;

    fn plot(kind: UnivariatePlotKind, normal: bool) -> UnivariatePlot {
        UnivariatePlot {
            kind,
            var: Some("x".into()),
            normal,
        }
    }

    #[test]
    fn no_overlay_without_the_normal_option() {
        let xs: Vec<f64> = (0..20).map(|i| i as f64).collect();
        let data: Vec<(f64, f64)> = xs.iter().map(|&x| (x, 0.0)).collect();
        assert!(
            overlay_of(
                &plot(UnivariatePlotKind::Histogram, false),
                &xs,
                &data,
                9.5,
                5.9
            )
            .is_none()
        );
    }

    #[test]
    fn histogram_curve_peaks_at_mu_and_sums_to_about_100_percent() {
        // Densité normale ramenée à l'échelle « Percent » : la somme des
        // ordonnées sur les `bins` classes vaut ~100 (l'aire totale d'une
        // densité). On échantillonne 100 points sur l'étendue, donc la somme
        // des ordonnées × (pas d'échantillonnage / largeur de classe) ≈ 100.
        let xs: Vec<f64> = (0..=100).map(|i| i as f64 * 0.2).collect(); // 0..20
        let data: Vec<(f64, f64)> = xs.iter().map(|&x| (x, 0.0)).collect();
        let (mu, sigma) = (10.0, 3.0);
        let ov = overlay_of(
            &plot(UnivariatePlotKind::Histogram, true),
            &xs,
            &data,
            mu,
            sigma,
        )
        .unwrap();
        assert!(ov.line && !ov.marker);
        assert_eq!(ov.data.len(), 100);

        // Le sommet de la courbe est en x = μ̂ (au pas d'échantillonnage près).
        let peak = ov
            .data
            .iter()
            .cloned()
            .fold(
                (f64::NAN, f64::NEG_INFINITY),
                |a, p| if p.1 > a.1 { p } else { a },
            );
        let step = 20.0 / 99.0;
        assert!((peak.0 - mu).abs() <= step, "peak at x = {}", peak.0);

        // Aire ≈ 100 % : Σ y_i · (pas / largeur de classe), largeur = 20/10 = 2.
        let area: f64 = ov.data.iter().map(|(_, y)| y * step / 2.0).sum();
        assert!((area - 100.0).abs() < 1.0, "area = {area}");
    }

    #[test]
    fn qqplot_reference_line_passes_through_mu_and_mu_plus_sigma() {
        // Droite y = μ̂ + σ̂·x sur l'étendue des quantiles théoriques tracés.
        let xs: Vec<f64> = (0..20).map(|i| i as f64).collect();
        let data: Vec<(f64, f64)> = vec![(-2.0, 0.0), (0.0, 5.0), (2.0, 9.0)];
        let (mu, sigma) = (5.0, 2.5);
        let ov = overlay_of(
            &plot(UnivariatePlotKind::QqPlot, true),
            &xs,
            &data,
            mu,
            sigma,
        )
        .unwrap();
        assert_eq!(ov.data.len(), 2);
        assert_eq!(ov.data[0], (-2.0, mu - 2.0 * sigma));
        assert_eq!(ov.data[1], (2.0, mu + 2.0 * sigma));
        // Interpolée en x = 0 et x = 1 : passe par (0, μ̂) et (1, μ̂+σ̂).
        let slope = (ov.data[1].1 - ov.data[0].1) / (ov.data[1].0 - ov.data[0].0);
        let at = |x: f64| ov.data[0].1 + slope * (x - ov.data[0].0);
        assert!((at(0.0) - mu).abs() < 1e-12);
        assert!((at(1.0) - (mu + sigma)).abs() < 1e-12);
    }

    #[test]
    fn cdfplot_and_ppplot_carry_no_overlay() {
        // Limite M45.2 documentée : l'option est acceptée (la table de
        // paramètres sort) mais aucune courbe n'est superposée.
        let xs: Vec<f64> = (0..20).map(|i| i as f64).collect();
        let data: Vec<(f64, f64)> = xs.iter().map(|&x| (x, x)).collect();
        for kind in [UnivariatePlotKind::CdfPlot, UnivariatePlotKind::PpPlot] {
            assert!(
                overlay_of(&plot(kind, true), &xs, &data, 9.5, 5.9).is_none(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn degenerate_inputs_produce_no_overlay() {
        let data: Vec<(f64, f64)> = vec![(0.0, 1.0), (1.0, 1.0)];
        // σ̂ = 0 (valeurs toutes égales).
        let flat = vec![1.0; 8];
        assert!(
            overlay_of(
                &plot(UnivariatePlotKind::Histogram, true),
                &flat,
                &data,
                1.0,
                0.0
            )
            .is_none()
        );
        // Étendue nulle sur l'histogramme (hi == lo) → pas de largeur de classe.
        assert!(
            overlay_of(
                &plot(UnivariatePlotKind::Histogram, true),
                &flat,
                &data,
                1.0,
                1.0
            )
            .is_none()
        );
        // Moins de 2 valeurs.
        assert!(
            overlay_of(
                &plot(UnivariatePlotKind::QqPlot, true),
                &[3.0],
                &data,
                3.0,
                1.0
            )
            .is_none()
        );
    }
}
