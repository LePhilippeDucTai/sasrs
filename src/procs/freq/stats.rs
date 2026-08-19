use super::*;

/// Fisher's exact test. Full exact two-sided p-value for 2x2 tables (sum of
/// hypergeometric probabilities ≤ that of the observed table), plus the
/// left/right one-sided tails and the observed table probability. General
/// r×c tables (M44.1) take the Freeman-Halton path in `fisher_rxc`.
pub(super) fn fisher_block(
    session: &mut Session,
    freq: &[Vec<usize>],
    row_tot: &[usize],
    col_tot: &[usize],
    grand: usize,
) {
    let nr = row_tot.len();
    let nc = col_tot.len();
    session.listing.blank();
    session.listing.write_line("Fisher's Exact Test");
    session.listing.blank();

    if grand == 0 {
        session
            .listing
            .write_line("Fisher's Exact Test is not computable for this table.");
        return;
    }
    if nr != 2 || nc != 2 {
        fisher_rxc(session, freq, row_tot, col_tot, grand);
        return;
    }

    // Margins are fixed. With r1 = row_tot[0], c1 = col_tot[0], n = grand, the
    // count a = freq[0][0] determines the whole table. a ranges over
    // [max(0, r1+c1-n), min(r1, c1)]. The hypergeometric probability of a is
    // C(r1,a)·C(r2,c1-a)/C(n,c1).
    let r1 = row_tot[0] as i64;
    let r2 = row_tot[1] as i64;
    let c1 = col_tot[0] as i64;
    let n = grand as i64;
    let a_obs = freq[0][0] as i64;

    let ln_p = |a: i64| -> f64 {
        let b = c1 - a; // freq[1][0]
        ln_choose(r1 as u64, a as u64) + ln_choose(r2 as u64, b as u64)
            - ln_choose(n as u64, c1 as u64)
    };

    let lo = 0.max(r1 + c1 - n);
    let hi = r1.min(c1);
    let p_obs = ln_p(a_obs).exp();

    let mut p_left = 0.0_f64; // P(A <= a_obs)
    let mut p_right = 0.0_f64; // P(A >= a_obs)
    let mut p_two = 0.0_f64; // sum of probs <= p_obs (with tolerance)
    let tol = 1e-7;
    for a in lo..=hi {
        let p = ln_p(a).exp();
        if a <= a_obs {
            p_left += p;
        }
        if a >= a_obs {
            p_right += p;
        }
        if p <= p_obs * (1.0 + tol) {
            p_two += p;
        }
    }
    let clamp = |p: f64| p.clamp(0.0, 1.0);

    let headers = vec!["Statistic".to_string(), "Value".to_string()];
    let aligns = vec![Align::Left, Align::Right];
    let rows = vec![
        vec!["Cell (1,1) Frequency (F)".to_string(), format!("{a_obs}")],
        vec!["Left-sided Pr <= F".to_string(), fmt_chisq_p(clamp(p_left))],
        vec![
            "Right-sided Pr >= F".to_string(),
            fmt_chisq_p(clamp(p_right)),
        ],
        vec![
            "Table Probability (P)".to_string(),
            fmt_chisq_p(clamp(p_obs)),
        ],
        vec!["Two-sided Pr <= P".to_string(), fmt_chisq_p(clamp(p_two))],
    ];
    session.listing.write_table(&headers, &aligns, &rows);
}

// ───────────────── M44.1 — Freeman-Halton (r×c Fisher exact) ─────────────────

/// Relative tolerance used when comparing a candidate table's probability to
/// the observed one for two-sided accumulation. Identical to the 2x2 path so
/// the general path reproduces it bit-for-bit on 2x2 input.
const FISHER_TOL: f64 = 1e-7;

/// Combinatorial guard: maximum number of margin-consistent tables the exact
/// enumeration may visit before falling back to Monte-Carlo estimation.
///
/// Rationale: each visited table costs O(r·c) lookups into a precomputed
/// ln-factorial array plus one `exp`. Measured in this environment, hitting
/// the guard (500 000 tables enumerated, then the full 10 000-sample
/// Monte-Carlo fallback on a 4x4/n=80 table) completes in under 0.2 s even
/// in the debug test profile, so the worst case stays comfortably sub-second
/// for `cargo test` while covering every small-to-moderate table exactly
/// (e.g. any 3x3 with n up to ~60, most 2xC/Rx2 layouts). This is the
/// documented "résiduel grandes tables → MC" scope decision from PLAN.md.
const FISHER_MAX_TABLES: u64 = 500_000;

/// Monte-Carlo replication count for the fallback estimator. Matches the SAS
/// default for Monte Carlo exact tests (EXACT / MC uses N=10000 by default).
const FISHER_MC_SAMPLES: u64 = 10_000;

/// Fixed PRNG seed for the Monte-Carlo fallback. The project convention
/// (`Session::deterministic`, frozen macro vars, snapshot tests) is that
/// output must be identical across runs, so the seed is a constant rather
/// than wall-clock derived. Arbitrary odd constant.
const FISHER_MC_SEED: u64 = 0x5A17_5A17_2026_0819;

/// Minimal deterministic 64-bit LCG for the Monte-Carlo fallback. Same Knuth
/// MMIX constants and same 53-bit uniform construction as the DATA-step RNG
/// (`datastep::functions::random`), which is tied to `EvalCtx` and therefore
/// not directly reusable from proc-level code.
struct Lcg(u64);

impl Lcg {
    fn uniform(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005_u64)
            .wrapping_add(1_442_695_040_888_963_407_u64);
        // Top 53 bits → integer in 0..2^53; +0.5 keeps the value strictly
        // inside (0, 1).
        let bits = self.0 >> 11;
        (bits as f64 + 0.5) / (1_u64 << 53) as f64
    }
}

/// Outcome of the r×c Fisher computation (separated from rendering so tests
/// can assert on the raw numbers).
pub(super) struct FisherRxcResult {
    /// Probability of the observed table (always exact).
    pub(super) p_obs: f64,
    /// Two-sided p-value: exact sum, or a Monte-Carlo estimate.
    pub(super) p_two: f64,
    /// Exact path only: total probability mass enumerated (must be ≈ 1).
    pub(super) p_sum: f64,
    /// Tables enumerated (exact) or samples drawn (Monte-Carlo).
    pub(super) count: u64,
    /// True when `p_two` is a Monte-Carlo estimate rather than an exact sum.
    pub(super) monte_carlo: bool,
}

/// Shared context for the exact enumeration walk.
struct FisherEnum<'a> {
    /// ln k! for k in 0..=n, so per-cell cost is one array lookup.
    lf: &'a [f64],
    row_tot: &'a [usize],
    /// Threshold: a table with probability ≤ this counts toward the
    /// two-sided sum (observed probability times the 1+1e-7 tolerance).
    p_thresh: f64,
    p_two: f64,
    p_sum: f64,
    count: u64,
    guard: u64,
}

/// Recursive exact enumeration of every r×c table with the fixed margins.
///
/// Cells are filled in row-major order. At cell (row, col) the value is
/// bounded below by `row_rem - Σ col_rem[col+1..]` (later columns must be
/// able to absorb the rest of the row) and above by
/// `min(row_rem, col_rem[col])`. The LAST cell of each row is forced by the
/// row remainder, and the LAST row is entirely forced by the column
/// remainders, so only (r-1)·(c-1) cells actually branch. `lp` carries the
/// running log-probability contribution (margin constant minus ln-factorials
/// of the cells placed so far).
///
/// Returns `false` as soon as the guard is exceeded (abort → Monte-Carlo).
fn fisher_enum_rec(
    ctx: &mut FisherEnum<'_>,
    row: usize,
    col: usize,
    row_rem: usize,
    col_rem: &mut [usize],
    lp: f64,
) -> bool {
    let nr = ctx.row_tot.len();
    let nc = col_rem.len();

    // Last row: forced to the column remainders — a complete table.
    if row == nr - 1 {
        let mut lp_final = lp;
        for &c in col_rem.iter() {
            lp_final -= ctx.lf[c];
        }
        ctx.count += 1;
        if ctx.count > ctx.guard {
            return false;
        }
        let p = lp_final.exp();
        ctx.p_sum += p;
        if p <= ctx.p_thresh {
            ctx.p_two += p;
        }
        return true;
    }

    // Last cell of a non-final row: forced to the row remainder.
    if col == nc - 1 {
        // Feasible by construction: earlier lower bounds guarantee
        // row_rem <= col_rem[col].
        col_rem[col] -= row_rem;
        let ok = fisher_enum_rec(
            ctx,
            row + 1,
            0,
            ctx.row_tot[row + 1],
            col_rem,
            lp - ctx.lf[row_rem],
        );
        col_rem[col] += row_rem;
        return ok;
    }

    // Free cell: branch over every feasible value.
    let rest: usize = col_rem[col + 1..].iter().sum();
    let lo = row_rem.saturating_sub(rest);
    let hi = row_rem.min(col_rem[col]);
    for v in lo..=hi {
        col_rem[col] -= v;
        let ok = fisher_enum_rec(ctx, row, col + 1, row_rem - v, col_rem, lp - ctx.lf[v]);
        col_rem[col] += v;
        if !ok {
            return false;
        }
    }
    true
}

/// Draw one r×c table from the conditional (fixed-margin) distribution by
/// sequential inversion sampling, returning its log-probability.
///
/// Row by row, cell by cell (same order as the enumeration), each free cell
/// is drawn from its exact conditional law, which is univariate
/// hypergeometric: distributing the row remainder `rr` over the remaining
/// columns with capacities `col_rem[col..]` (population `s`), the count
/// landing in column `col` has weight C(col_rem[col], x)·C(s-col_rem[col],
/// rr-x). Weights are built in log-space from the `lf` table, shifted by
/// their max before `exp` (log-sum-exp), cumulated, and inverted with one
/// uniform draw — a standard, easily verified scheme.
fn fisher_mc_sample(
    lf: &[f64],
    row_tot: &[usize],
    col_tot: &[usize],
    lp_margins: f64,
    rng: &mut Lcg,
) -> f64 {
    let nr = row_tot.len();
    let nc = col_tot.len();
    let mut col_rem = col_tot.to_vec();
    let mut lp = lp_margins;

    for &rt in row_tot.iter().take(nr - 1) {
        let mut rr = rt;
        let mut s: usize = col_rem.iter().sum();
        for cr in col_rem.iter_mut().take(nc - 1) {
            let k = *cr;
            let lo = rr.saturating_sub(s - k);
            let hi = rr.min(k);
            let x = if lo == hi {
                lo
            } else {
                // Log-weights of the hypergeometric support, max-shifted.
                let lws: Vec<f64> = (lo..=hi)
                    .map(|x| {
                        // ln C(k, x) + ln C(s-k, rr-x)
                        lf[k] - lf[x] - lf[k - x] + lf[s - k] - lf[rr - x] - lf[s - k - (rr - x)]
                    })
                    .collect();
                let m = lws.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                let ws: Vec<f64> = lws.iter().map(|&lw| (lw - m).exp()).collect();
                let total: f64 = ws.iter().sum();
                let target = rng.uniform() * total;
                let mut acc = 0.0;
                let mut pick = hi;
                for (i, &w) in ws.iter().enumerate() {
                    acc += w;
                    if acc >= target {
                        pick = lo + i;
                        break;
                    }
                }
                pick
            };
            lp -= lf[x];
            *cr -= x;
            rr -= x;
            s -= k;
        }
        // Last column of the row is forced.
        lp -= lf[rr];
        col_rem[nc - 1] -= rr;
    }
    // Last row is forced to the column remainders.
    for &c in col_rem.iter() {
        lp -= lf[c];
    }
    lp
}

/// Compute the Freeman-Halton exact test for an r×c table: exact full
/// enumeration when the table count stays within `guard`, deterministic
/// Monte-Carlo estimation (fixed seed) otherwise. Parameterized so tests can
/// exercise both paths cheaply; production callers use the module constants.
pub(super) fn fisher_rxc_compute(
    freq: &[Vec<usize>],
    row_tot: &[usize],
    col_tot: &[usize],
    grand: usize,
    guard: u64,
    mc_samples: u64,
    seed: u64,
) -> FisherRxcResult {
    // ln k! lookup for every value a cell or margin can take (≤ grand).
    let lf: Vec<f64> = (0..=grand as u64).map(ln_factorial).collect();

    // Margin constant: Σ ln r_i! + Σ ln c_j! − ln n!.
    let mut lp_margins = -lf[grand];
    for &r in row_tot {
        lp_margins += lf[r];
    }
    for &c in col_tot {
        lp_margins += lf[c];
    }

    // Observed table probability (exact in both modes).
    let mut lp_obs = lp_margins;
    for r in freq {
        for &x in r {
            lp_obs -= lf[x];
        }
    }
    let p_obs = lp_obs.exp();
    let p_thresh = p_obs * (1.0 + FISHER_TOL);

    // Exact path: enumerate every table unless the guard trips.
    let mut ctx = FisherEnum {
        lf: &lf,
        row_tot,
        p_thresh,
        p_two: 0.0,
        p_sum: 0.0,
        count: 0,
        guard,
    };
    let mut col_rem = col_tot.to_vec();
    let complete = fisher_enum_rec(&mut ctx, 0, 0, row_tot[0], &mut col_rem, lp_margins);
    if complete {
        return FisherRxcResult {
            p_obs,
            p_two: ctx.p_two.clamp(0.0, 1.0),
            p_sum: ctx.p_sum,
            count: ctx.count,
            monte_carlo: false,
        };
    }

    // Monte-Carlo fallback: deterministic seeded sampling from the same
    // conditional distribution; p̂ = (#samples with p ≤ threshold) / N.
    let mut rng = Lcg(seed);
    let mut hits = 0u64;
    for _ in 0..mc_samples {
        let lp = fisher_mc_sample(&lf, row_tot, col_tot, lp_margins, &mut rng);
        if lp.exp() <= p_thresh {
            hits += 1;
        }
    }
    FisherRxcResult {
        p_obs,
        p_two: hits as f64 / mc_samples as f64,
        p_sum: f64::NAN,
        count: mc_samples,
        monte_carlo: true,
    }
}

/// Render the Freeman-Halton block for a general r×c table. SAS's r×c Fisher
/// output reports the observed table probability and the single `Pr <= P`
/// statistic (the two-sided left/right split of the 2x2 layout does not
/// generalize). A Monte-Carlo estimate is explicitly labeled as such.
fn fisher_rxc(
    session: &mut Session,
    freq: &[Vec<usize>],
    row_tot: &[usize],
    col_tot: &[usize],
    grand: usize,
) {
    let res = fisher_rxc_compute(
        freq,
        row_tot,
        col_tot,
        grand,
        FISHER_MAX_TABLES,
        FISHER_MC_SAMPLES,
        FISHER_MC_SEED,
    );

    let headers = vec!["Statistic".to_string(), "Value".to_string()];
    let aligns = vec![Align::Left, Align::Right];
    let rows = vec![
        vec![
            "Table Probability (P)".to_string(),
            fmt_chisq_p(res.p_obs.clamp(0.0, 1.0)),
        ],
        vec!["Pr <= P".to_string(), fmt_chisq_p(res.p_two)],
    ];
    session.listing.write_table(&headers, &aligns, &rows);
    if res.monte_carlo {
        session.listing.write_line(&format!(
            "Note: Pr <= P is a Monte Carlo estimate based on {} samples (fixed seed).",
            res.count
        ));
    }
}

/// Cochran-Armitage trend test. Requires a 2-row (or 2-column) table; the
/// non-binary dimension supplies ordinal scores 1..k. Reports the Z statistic
/// with one- and two-sided normal-approximation p-values. Other shapes are
/// deferred with a graceful note.
pub(super) fn trend_block(
    session: &mut Session,
    freq: &[Vec<usize>],
    row_tot: &[usize],
    col_tot: &[usize],
    grand: usize,
) {
    let nr = row_tot.len();
    let nc = col_tot.len();
    session.listing.blank();
    session.listing.write_line("Cochran-Armitage Trend Test");
    session.listing.blank();

    if grand == 0 || (nr != 2 && nc != 2) || nr < 2 || nc < 2 {
        session
            .listing
            .write_line("The Cochran-Armitage Trend Test requires a 2xC or Rx2 table.");
        return;
    }

    // Orient so that there are 2 rows and `k` ordinal columns. If the table is
    // Rx2 instead, transpose roles (scores along rows).
    // We compute using the first row's counts (n_{1i}) against column totals.
    // T = Σ s_i (n_{1i} - r1 * c_i / N).
    // Var(T) = (r1*r2/N) * [ Σ c_i s_i² - (Σ c_i s_i)² / N ].
    let (cells_row1, marg): (Vec<f64>, Vec<f64>);
    let r1f: f64;
    let r2f: f64;
    if nr == 2 {
        cells_row1 = (0..nc).map(|c| freq[0][c] as f64).collect();
        marg = col_tot.iter().map(|&c| c as f64).collect();
        r1f = row_tot[0] as f64;
        r2f = row_tot[1] as f64;
    } else {
        // Rx2: treat columns as the binary dimension, rows as ordinal scores.
        cells_row1 = (0..nr).map(|r| freq[r][0] as f64).collect();
        marg = row_tot.iter().map(|&r| r as f64).collect();
        r1f = col_tot[0] as f64;
        r2f = col_tot[1] as f64;
    }
    let k = cells_row1.len();
    let scores: Vec<f64> = (1..=k).map(|i| i as f64).collect();
    let nf = grand as f64;

    let mut t = 0.0_f64;
    let mut sum_cs = 0.0_f64; // Σ c_i s_i
    let mut sum_cs2 = 0.0_f64; // Σ c_i s_i²
    for i in 0..k {
        t += scores[i] * (cells_row1[i] - r1f * marg[i] / nf);
        sum_cs += marg[i] * scores[i];
        sum_cs2 += marg[i] * scores[i] * scores[i];
    }
    let var = (r1f * r2f / nf) * (sum_cs2 - sum_cs * sum_cs / nf);

    if var <= 0.0 {
        session
            .listing
            .write_line("The Cochran-Armitage Trend Test is not computable for this table.");
        return;
    }
    let z = t / var.sqrt();
    // One-sided p toward the observed direction; two-sided = 2*one-sided.
    let p_one = 1.0 - probnorm(z.abs());
    let p_two = (2.0 * p_one).min(1.0);

    let headers = vec!["Statistic".to_string(), "Value".to_string()];
    let aligns = vec![Align::Left, Align::Right];
    let rows = vec![
        vec!["Statistic (Z)".to_string(), format!("{z:.4}")],
        vec![
            "One-sided Pr".to_string(),
            fmt_chisq_p(p_one.clamp(0.0, 1.0)),
        ],
        vec![
            "Two-sided Pr".to_string(),
            fmt_chisq_p(p_two.clamp(0.0, 1.0)),
        ],
    ];
    session.listing.write_table(&headers, &aligns, &rows);
}

/// MEASURES / RELRISK: odds ratio and the two cohort relative risks for a 2x2
/// table, each with a 95% confidence interval (Wald, on the log scale). Cells
/// containing zeros yield missing estimates rather than dividing by zero.
pub(super) fn measures_block(session: &mut Session, freq: &[Vec<usize>]) {
    session.listing.blank();
    session
        .listing
        .write_line("Estimates of the Relative Risk (Row1/Row2)");
    session.listing.blank();

    if freq.len() != 2 || freq[0].len() != 2 || freq[1].len() != 2 {
        session
            .listing
            .write_line("Relative risk estimates require a 2x2 table.");
        return;
    }

    let a = freq[0][0] as f64;
    let b = freq[0][1] as f64;
    let c = freq[1][0] as f64;
    let d = freq[1][1] as f64;

    let headers = vec![
        "Type of Study".to_string(),
        "Value".to_string(),
        "95% Confidence Limits".to_string(),
    ];
    let aligns = vec![Align::Left, Align::Right, Align::Left];
    let mut rows: Vec<Vec<String>> = Vec::new();

    // Helper rendering "lo   hi" or "." when an estimate is undefined.
    let limits = |lo: f64, hi: f64, ok: bool| -> String {
        if ok {
            format!("{lo:.4}   {hi:.4}")
        } else {
            ".".to_string()
        }
    };

    // Odds ratio = ad/bc; SE(ln OR) = sqrt(1/a+1/b+1/c+1/d).
    if a > 0.0 && b > 0.0 && c > 0.0 && d > 0.0 {
        let or = (a * d) / (b * c);
        let se = (1.0 / a + 1.0 / b + 1.0 / c + 1.0 / d).sqrt();
        let (lo, hi) = ((or.ln() - 1.96 * se).exp(), (or.ln() + 1.96 * se).exp());
        rows.push(vec![
            "Case-Control (Odds Ratio)".to_string(),
            format!("{or:.4}"),
            limits(lo, hi, true),
        ]);
    } else {
        rows.push(vec![
            "Case-Control (Odds Ratio)".to_string(),
            ".".to_string(),
            ".".to_string(),
        ]);
    }

    // Cohort (Col1 Risk): RR = [a/(a+b)] / [c/(c+d)].
    let r1 = a + b;
    let r2 = c + d;
    if r1 > 0.0 && r2 > 0.0 && a > 0.0 && c > 0.0 {
        let rr = (a / r1) / (c / r2);
        let se = (b / (a * r1) + d / (c * r2)).sqrt();
        let (lo, hi) = ((rr.ln() - 1.96 * se).exp(), (rr.ln() + 1.96 * se).exp());
        rows.push(vec![
            "Cohort (Col1 Risk)".to_string(),
            format!("{rr:.4}"),
            limits(lo, hi, true),
        ]);
    } else {
        rows.push(vec![
            "Cohort (Col1 Risk)".to_string(),
            ".".to_string(),
            ".".to_string(),
        ]);
    }

    // Cohort (Col2 Risk): RR = [b/(a+b)] / [d/(c+d)].
    if r1 > 0.0 && r2 > 0.0 && b > 0.0 && d > 0.0 {
        let rr = (b / r1) / (d / r2);
        let se = (a / (b * r1) + c / (d * r2)).sqrt();
        let (lo, hi) = ((rr.ln() - 1.96 * se).exp(), (rr.ln() + 1.96 * se).exp());
        rows.push(vec![
            "Cohort (Col2 Risk)".to_string(),
            format!("{rr:.4}"),
            limits(lo, hi, true),
        ]);
    } else {
        rows.push(vec![
            "Cohort (Col2 Risk)".to_string(),
            ".".to_string(),
            ".".to_string(),
        ]);
    }

    session.listing.write_table(&headers, &aligns, &rows);
}

/// AGREE: Cohen's simple kappa coefficient for a square table, with its
/// asymptotic standard error and a 95% confidence interval. Non-square tables
/// are rejected with a graceful note.
pub(super) fn agree_block(
    session: &mut Session,
    freq: &[Vec<usize>],
    row_tot: &[usize],
    col_tot: &[usize],
    grand: usize,
) {
    let nr = row_tot.len();
    let nc = col_tot.len();
    session.listing.blank();
    session.listing.write_line("Simple Kappa Coefficient");
    session.listing.blank();

    if nr != nc {
        session.listing.write_line("AGREE requires a square table.");
        return;
    }
    if grand == 0 {
        session
            .listing
            .write_line("Simple Kappa Coefficient is not computable for this table.");
        return;
    }

    let n = grand as f64;
    // Observed agreement Po = Σ p_ii ; expected Pe = Σ p_i+ · p_+i.
    let mut po = 0.0_f64;
    let mut pe = 0.0_f64;
    for i in 0..nr {
        po += freq[i][i] as f64 / n;
        pe += (row_tot[i] as f64 / n) * (col_tot[i] as f64 / n);
    }

    if (1.0 - pe).abs() < 1e-12 {
        session
            .listing
            .write_line("Simple Kappa Coefficient is not computable (perfect expected agreement).");
        return;
    }
    let kappa = (po - pe) / (1.0 - pe);

    // Asymptotic standard error under H1 (Fleiss et al.), the SAS ASE.
    // ASE = sqrt( [ A + B - C ] / [ (1-Pe)² · n ] ) with
    //   A = Σ p_ii [1 - (p_i+ + p_+i)(1 - kappa)]²
    //   B = (1-kappa)² Σ_{i≠j} p_ij (p_+i + p_j+)²
    //   C = (kappa - Pe(1-kappa))²
    let p = |i: usize, j: usize| freq[i][j] as f64 / n;
    let pr = |i: usize| row_tot[i] as f64 / n; // p_i+ (row marginal)
    let pc = |j: usize| col_tot[j] as f64 / n; // p_+j (col marginal)

    let mut term_a = 0.0_f64;
    for i in 0..nr {
        let s = 1.0 - (pr(i) + pc(i)) * (1.0 - kappa);
        term_a += p(i, i) * s * s;
    }
    let mut term_b = 0.0_f64;
    for i in 0..nr {
        for j in 0..nc {
            if i != j {
                let s = pc(i) + pr(j);
                term_b += p(i, j) * s * s;
            }
        }
    }
    term_b *= (1.0 - kappa) * (1.0 - kappa);
    let term_c = (kappa - pe * (1.0 - kappa)).powi(2);

    let var = (term_a + term_b - term_c) / ((1.0 - pe).powi(2) * n);
    let ase = if var > 0.0 { var.sqrt() } else { 0.0 };
    let lower = kappa - 1.96 * ase;
    let upper = kappa + 1.96 * ase;

    let headers = vec!["Statistic".to_string(), "Value".to_string()];
    let aligns = vec![Align::Left, Align::Right];
    let rows = vec![
        vec!["Kappa".to_string(), format!("{kappa:.4}")],
        vec!["ASE".to_string(), format!("{ase:.4}")],
        vec!["95% Lower Conf Limit".to_string(), format!("{lower:.4}")],
        vec!["95% Upper Conf Limit".to_string(), format!("{upper:.4}")],
    ];
    session.listing.write_table(&headers, &aligns, &rows);
}

/// Format a p-value SAS-style: `<.0001`, else 4 decimals (mirrors corr.rs).
pub(super) fn fmt_chisq_p(p: f64) -> String {
    if p < 0.0001 {
        "<.0001".to_string()
    } else {
        format!("{p:.4}")
    }
}

/// Print the "Statistics for Table of <row> by <col>" CHISQ block for a
/// two-way table: Pearson chi-square and the likelihood-ratio chi-square,
/// each with DF and an upper-tail p-value. Degenerate tables (grand total 0,
/// any zero margin, or DF <= 0) are skipped gracefully with a note.
pub(super) fn chisq_block(
    session: &mut Session,
    row_name: &str,
    col_name: &str,
    freq: &[Vec<f64>],
    row_tot: &[f64],
    col_tot: &[f64],
    grand: f64,
) {
    session.listing.blank();
    session
        .listing
        .write_line(&format!("Statistics for Table of {row_name} by {col_name}"));
    session.listing.blank();

    let nr = row_tot.len();
    let nc = col_tot.len();
    let df = (nr.saturating_sub(1)) * (nc.saturating_sub(1));

    // Guard against degenerate tables: no expected counts are defined.
    if grand <= 0.0
        || df == 0
        || row_tot.iter().any(|&t| t <= 0.0)
        || col_tot.iter().any(|&t| t <= 0.0)
    {
        session
            .listing
            .write_line("Chi-Square statistics are not computable for this table.");
        return;
    }

    let g = grand;
    let mut pearson = 0.0_f64;
    let mut lratio = 0.0_f64;
    for r in 0..nr {
        for c in 0..nc {
            let e = row_tot[r] * col_tot[c] / g;
            let n = freq[r][c];
            if e > 0.0 {
                let d = n - e;
                pearson += d * d / e;
            }
            if n > 0.0 && e > 0.0 {
                lratio += n * (n / e).ln();
            }
        }
    }
    lratio *= 2.0;

    let df_f = df as f64;
    let p_pearson = chisq_sf(pearson, df_f);
    let p_lratio = chisq_sf(lratio, df_f);

    let headers = vec![
        "Statistic".to_string(),
        "DF".to_string(),
        "Value".to_string(),
        "Prob".to_string(),
    ];
    let aligns = vec![Align::Left, Align::Right, Align::Right, Align::Right];
    let rows = vec![
        vec![
            "Chi-Square".to_string(),
            format!("{df}"),
            format!("{pearson:.4}"),
            fmt_chisq_p(p_pearson),
        ],
        vec![
            "Likelihood Ratio Chi-Square".to_string(),
            format!("{df}"),
            format!("{lratio:.4}"),
            fmt_chisq_p(p_lratio),
        ],
    ];
    session.listing.write_table(&headers, &aligns, &rows);
}
