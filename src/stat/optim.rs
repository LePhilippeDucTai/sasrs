//! Optimiseurs sans dérivée partagés par les procédures à vraisemblance.
//!
//! `nelder_mead` (simplexe) suivi de `polish_coord` (descente par coordonnées)
//! est le schéma utilisé pour maximiser la (RE)ML des modèles mixtes : le
//! simplexe approche l'optimum, le polissage nettoie le palier résiduel que
//! Nelder-Mead laisse sur les surfaces plates.
//!
//! Ces deux routines étaient dupliquées entre PROC MIXED et PROC GLIMMIX
//! (corps identiques, seul le tuple de retour de `nelder_mead` différait) ;
//! elles sont centralisées ici.

use std::cmp::Ordering;

/// Ordonne deux valeurs d'objectif, un NaN étant TOUJOURS le pire.
///
/// MQ9.1 — c'était `partial_cmp(..).unwrap()`, seul site du dépôt à ne pas
/// gérer le cas `None` (les 25 autres comparaisons de flottants utilisent
/// `unwrap_or(Ordering::Equal)`). Une vraisemblance divergente sur les données
/// de l'utilisateur — cas banal en PROC MIXED / GLIMMIX — produit un objectif
/// NaN et tuait le process, au lieu de laisser la proc conclure à une
/// non-convergence. Trier le NaN en dernier garantit en prime qu'un sommet
/// NaN n'est jamais retenu comme meilleur point.
fn cmp_objective(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
    }
}

/// One run of Nelder-Mead from `start` with per-dimension initial step `step`.
/// Minimises `eval` over `np`-dimensional unconstrained space. Returns the best
/// point found, its function value, the number of iterations consumed, and
/// whether the simplex converged (function-value spread and vertex spread both
/// below tolerance).
pub(crate) fn nelder_mead<F: Fn(&[f64]) -> f64>(
    eval: &F,
    start: &[f64],
    step: f64,
    max_iter: usize,
    ftol: f64,
    xtol: f64,
) -> (Vec<f64>, f64, usize, bool) {
    let np = start.len();

    // Build initial simplex.
    let mut simplex: Vec<Vec<f64>> = Vec::with_capacity(np + 1);
    let mut fvals: Vec<f64> = Vec::with_capacity(np + 1);
    simplex.push(start.to_vec());
    fvals.push(eval(start));
    for d in 0..np {
        let mut pt = start.to_vec();
        pt[d] += step;
        let f = eval(&pt);
        simplex.push(pt);
        fvals.push(f);
    }

    let (alpha, gamma, rho_c, sigma) = (1.0_f64, 2.0_f64, 0.5_f64, 0.5_f64);
    let mut iters = 0usize;
    let mut converged = false;
    while iters < max_iter {
        iters += 1;
        // Order by function value.
        let mut order: Vec<usize> = (0..=np).collect();
        order.sort_by(|&a, &b| cmp_objective(fvals[a], fvals[b]));
        let s: Vec<Vec<f64>> = order.iter().map(|&i| simplex[i].clone()).collect();
        let f: Vec<f64> = order.iter().map(|&i| fvals[i]).collect();
        simplex = s;
        fvals = f;

        // Convergence: both the function-value spread AND the simplex extent
        // (max vertex distance from the best vertex) must be small.
        let fspread = (fvals[np] - fvals[0]).abs();
        let mut xspread = 0.0_f64;
        for pt in simplex.iter().take(np + 1) {
            let mut d2 = 0.0;
            for d in 0..np {
                let dx = pt[d] - simplex[0][d];
                d2 += dx * dx;
            }
            xspread = xspread.max(d2.sqrt());
        }
        if fspread < ftol * (1.0 + fvals[0].abs()) && xspread < xtol {
            converged = true;
            break;
        }

        // Centroid of all but worst.
        let mut centroid = vec![0.0; np];
        for pt in simplex.iter().take(np) {
            for d in 0..np {
                centroid[d] += pt[d] / np as f64;
            }
        }
        // Reflection.
        let worst = &simplex[np];
        let refl: Vec<f64> = (0..np)
            .map(|d| centroid[d] + alpha * (centroid[d] - worst[d]))
            .collect();
        let fr = eval(&refl);
        if fr < fvals[0] {
            // Expansion.
            let exp: Vec<f64> = (0..np)
                .map(|d| centroid[d] + gamma * (refl[d] - centroid[d]))
                .collect();
            let fe = eval(&exp);
            if fe < fr {
                simplex[np] = exp;
                fvals[np] = fe;
            } else {
                simplex[np] = refl;
                fvals[np] = fr;
            }
        } else if fr < fvals[np - 1] {
            simplex[np] = refl;
            fvals[np] = fr;
        } else {
            // Contraction.
            let con: Vec<f64> = (0..np)
                .map(|d| centroid[d] + rho_c * (worst[d] - centroid[d]))
                .collect();
            let fc = eval(&con);
            if fc < fvals[np] {
                simplex[np] = con;
                fvals[np] = fc;
            } else {
                // Shrink toward best.
                let best = simplex[0].clone();
                for i in 1..=np {
                    for d in 0..np {
                        simplex[i][d] = best[d] + sigma * (simplex[i][d] - best[d]);
                    }
                    fvals[i] = eval(&simplex[i]);
                }
            }
        }
    }

    // Best vertex.
    let mut best_idx = 0;
    for i in 1..=np {
        if fvals[i] < fvals[best_idx] {
            best_idx = i;
        }
    }
    (simplex[best_idx].clone(), fvals[best_idx], iters, converged)
}

/// Coordinate-descent polish on the unconstrained parameters using
/// finite-difference secant steps on −2·logL. Refines each coordinate in turn
/// with a parabolic/secant minimiser, shrinking the step until it is below
/// `xstop`. This cleans up the residual flat-surface stall left by Nelder-Mead.
pub(crate) fn polish_coord<F: Fn(&[f64]) -> f64>(
    eval: &F,
    u: &mut [f64],
    fval: &mut f64,
    xstop: f64,
) {
    let np = u.len();
    let mut step = 1e-2_f64;
    for _ in 0..60 {
        let f_before = *fval;
        for d in 0..np {
            // Three-point parabolic line minimisation along coordinate d.
            let x0 = u[d];
            let h = step;
            let fm = {
                u[d] = x0 - h;
                let v = eval(u);
                u[d] = x0;
                v
            };
            let fp = {
                u[d] = x0 + h;
                let v = eval(u);
                u[d] = x0;
                v
            };
            let f0 = *fval;
            // Parabola through (x0-h,fm),(x0,f0),(x0+h,fp); vertex offset.
            let denom = fm - 2.0 * f0 + fp;
            let mut improved = false;
            if denom > 1e-300 {
                let delta = 0.5 * h * (fm - fp) / denom;
                // Clamp the proposed step to a few h to stay local.
                let delta = delta.clamp(-4.0 * h, 4.0 * h);
                let xc = x0 + delta;
                u[d] = xc;
                let fc = eval(u);
                if fc < *fval {
                    *fval = fc;
                    improved = true;
                } else {
                    u[d] = x0;
                }
            }
            if !improved {
                // Fall back to the better of the two probe points.
                if fm < *fval && fm <= fp {
                    u[d] = x0 - h;
                    *fval = fm;
                } else if fp < *fval {
                    u[d] = x0 + h;
                    *fval = fp;
                } else {
                    u[d] = x0;
                }
            }
        }
        // Shrink step when a full sweep stops improving.
        if (f_before - *fval).abs() < 1e-14 * (1.0 + fval.abs()) {
            step *= 0.25;
            if step < xstop {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests;
