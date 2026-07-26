// ──────────────────────────────────────────────────────────────────────────────
// M15.5 — Random-variate generation
//
// RNG: 64-bit LCG using the constants from Knuth (MMIX):
//   state = state * 6364136223846793005 + 1442695040888963407  (mod 2^64)
// A uniform float in (0, 1) is obtained by taking the top 53 bits and
// dividing by 2^53; the result is guaranteed to be strictly in (0, 1) because
// values 0 and 2^53 cannot appear after the division.
//
// The Box–Muller transform is used for Normal variates; one invocation
// generates a pair and the spare is cached in `ctx.rng_spare`.
//
// CALL STREAMINIT is wired in exec.rs (exec_call_routine) and merely resets
// `ctx.rng_state` — it produces no return value.
// ──────────────────────────────────────────────────────────────────────────────

use super::*;

/// Advance the LCG and return the new state.
#[inline]
pub(super) fn lcg_next(state: u64) -> u64 {
    state
        .wrapping_mul(6_364_136_223_846_793_005_u64)
        .wrapping_add(1_442_695_040_888_963_407_u64)
}

/// Draw a uniform float strictly in (0, 1) from the LCG state.
#[inline]
pub(super) fn lcg_uniform(ctx: &mut EvalCtx) -> f64 {
    ctx.rng_state = lcg_next(ctx.rng_state);
    // Top 53 bits → integer in 0..2^53; adding 0.5 avoids exact 0 and 1.
    let bits = ctx.rng_state >> 11; // 64 - 53 = 11
    (bits as f64 + 0.5) / (1_u64 << 53) as f64
}

/// Draw a standard Normal(0,1) variate via Box–Muller, caching the spare.
pub(super) fn rng_normal(ctx: &mut EvalCtx) -> f64 {
    if let Some(spare) = ctx.rng_spare.take() {
        return spare;
    }
    // Generate a pair.
    let u1 = lcg_uniform(ctx);
    let u2 = lcg_uniform(ctx);
    let mag = (-2.0 * u1.ln()).sqrt();
    let angle = 2.0 * std::f64::consts::PI * u2;
    let z0 = mag * angle.cos();
    let z1 = mag * angle.sin();
    ctx.rng_spare = Some(z1);
    z0
}

/// RAND(distribution [, parm1 [, parm2 [, parm3]]])
/// Supported distributions: UNIFORM, NORMAL, EXPONENTIAL, POISSON, BINOMIAL.
pub(super) fn fn_rand(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let dist = match args.first() {
        None => return Value::missing(),
        Some(Value::Char(s)) => s.trim().to_uppercase(),
        Some(v) if v.is_missing() => return Value::missing(),
        Some(_) => return Value::missing(),
    };

    match dist.as_str() {
        "UNIFORM" => {
            let lo = args.get(1).and_then(|v| coerce_num(v, ctx)).unwrap_or(0.0);
            let hi = args.get(2).and_then(|v| coerce_num(v, ctx)).unwrap_or(1.0);
            if hi <= lo {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            let u = lcg_uniform(ctx);
            Value::Num(lo + u * (hi - lo))
        }
        "NORMAL" => {
            let mu = args.get(1).and_then(|v| coerce_num(v, ctx)).unwrap_or(0.0);
            let sigma = args.get(2).and_then(|v| coerce_num(v, ctx)).unwrap_or(1.0);
            if sigma <= 0.0 {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            Value::Num(mu + sigma * rng_normal(ctx))
        }
        "EXPONENTIAL" => {
            let lambda = args.get(1).and_then(|v| coerce_num(v, ctx)).unwrap_or(1.0);
            if lambda <= 0.0 {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            let u = lcg_uniform(ctx);
            Value::Num(-u.ln() / lambda)
        }
        "POISSON" => {
            let lambda = args.get(1).and_then(|v| coerce_num(v, ctx)).unwrap_or(1.0);
            if lambda < 0.0 {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            // Knuth's direct algorithm: count until product of uniforms < e^{-λ}.
            let threshold = (-lambda).exp();
            let mut k = 0u64;
            let mut p = 1.0f64;
            loop {
                p *= lcg_uniform(ctx);
                if p < threshold {
                    break;
                }
                k += 1;
            }
            Value::Num(k as f64)
        }
        "BINOMIAL" => {
            let prob = match args.get(1).and_then(|v| coerce_num(v, ctx)) {
                None => return Value::missing(),
                Some(p) => p,
            };
            let n = match args.get(2).and_then(|v| coerce_num(v, ctx)) {
                None => return Value::missing(),
                Some(v) => v.round() as u64,
            };
            if !(0.0..=1.0).contains(&prob) {
                ctx.error_flag = true;
                ctx.invalid_data += 1;
                return Value::missing();
            }
            let mut successes = 0u64;
            for _ in 0..n {
                if lcg_uniform(ctx) < prob {
                    successes += 1;
                }
            }
            Value::Num(successes as f64)
        }
        _ => {
            ctx.error_flag = true;
            ctx.invalid_data += 1;
            Value::missing()
        }
    }
}

/// RANUNI([seed]) — Uniform(0,1) random variate.
/// If seed is provided and non-missing, reinitialise the RNG.
pub(super) fn fn_ranuni(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if let Some(v) = args.first() {
        if let Some(seed) = coerce_num(v, ctx) {
            ctx.rng_state = seed_to_state(seed as i64);
        }
    }
    Value::Num(lcg_uniform(ctx))
}

/// RANNOR([seed]) — Standard Normal(0,1) random variate.
pub(super) fn fn_rannor(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if let Some(v) = args.first() {
        if let Some(seed) = coerce_num(v, ctx) {
            ctx.rng_state = seed_to_state(seed as i64);
            ctx.rng_spare = None; // invalidate cached spare after re-seed
        }
    }
    Value::Num(rng_normal(ctx))
}

/// RANEXP([seed]) — Exponential(1) random variate.
pub(super) fn fn_ranexp(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if let Some(v) = args.first() {
        if let Some(seed) = coerce_num(v, ctx) {
            ctx.rng_state = seed_to_state(seed as i64);
        }
    }
    let u = lcg_uniform(ctx);
    Value::Num(-u.ln())
}

/// RANBIN(p, n [, seed]) — Binomial(n, p) random variate.
pub(super) fn fn_ranbin(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let prob = match args.first() {
        None => return Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(p) => p,
        },
    };
    let n = match args.get(1) {
        None => return Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(nf) => nf.round() as u64,
        },
    };
    // Optional seed in 3rd position.
    if let Some(sv) = args.get(2) {
        if let Some(seed) = coerce_num(sv, ctx) {
            ctx.rng_state = seed_to_state(seed as i64);
        }
    }
    if !(0.0..=1.0).contains(&prob) {
        ctx.error_flag = true;
        ctx.invalid_data += 1;
        return Value::missing();
    }
    let mut successes = 0u64;
    for _ in 0..n {
        if lcg_uniform(ctx) < prob {
            successes += 1;
        }
    }
    Value::Num(successes as f64)
}

/// Convert an integer seed into an LCG state. Seed 0 is mapped to the default
/// state so we always have a non-trivial starting point.
#[inline]
pub(super) fn seed_to_state(seed: i64) -> u64 {
    if seed == 0 {
        0x0000_0007_A120_1960_u64
    } else {
        seed as u64
    }
}

/// Public helper used by `exec.rs` to wire CALL STREAMINIT.
/// Converts a seed integer to an LCG state value for `EvalCtx::rng_state`.
pub fn streaminit_seed(seed: i64) -> u64 {
    seed_to_state(seed)
}
