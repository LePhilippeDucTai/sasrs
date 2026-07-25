
mod calendar;
mod interval;
mod diff;

pub(crate) use calendar::*;
pub(crate) use interval::*;
pub(crate) use diff::*;

// ──────────────────────────────────────────────────────────────────────────────
// Date functions
// ──────────────────────────────────────────────────────────────────────────────

use super::*;

pub(crate) fn fn_today(_args: &[Value], _ctx: &mut EvalCtx) -> Value {
    Value::Num(today_sas())
}

pub(crate) fn fn_mdy(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 3 {
        ctx.invalid_data += 1;
        return Value::missing();
    }
    let m = match coerce_num(&args[0], ctx) {
        None => return Value::missing(),
        Some(f) => f as i64,
    };
    let d = match coerce_num(&args[1], ctx) {
        None => return Value::missing(),
        Some(f) => f as i64,
    };
    let y = match coerce_num(&args[2], ctx) {
        None => return Value::missing(),
        Some(f) => f as i64,
    };
    if !is_valid_date(y, m, d) {
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    }
    Value::Num(ymd_to_sas_date(y, m, d))
}

pub(crate) fn fn_year(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| sas_date_to_ymd(f as i64).0 as f64)
}

pub(crate) fn fn_month(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| sas_date_to_ymd(f as i64).1 as f64)
}

pub(crate) fn fn_day(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| sas_date_to_ymd(f as i64).2 as f64)
}

pub(crate) fn fn_weekday(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |f| sas_weekday(f as i64) as f64)
}

pub(crate) fn fn_datepart(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |dt| split_datetime(dt).0)
}

pub(crate) fn fn_timepart(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |dt| split_datetime(dt).1.trunc())
}

pub(crate) fn fn_datetime_combine(args: &[Value], ctx: &mut EvalCtx) -> Value {
    // DATETIME(date, time) — combine une date SAS et une heure-du-jour.
    let date = match args.first() {
        None => return Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    let time = match args.get(1) {
        None => 0.0,
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    Value::Num(date * SECONDS_PER_DAY + time)
}

pub(crate) fn fn_hms(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let h = match args.first() {
        None => 0.0,
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    let m = match args.get(1) {
        None => 0.0,
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    let s = match args.get(2) {
        None => 0.0,
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    // h ≥ 0 ; m,s dans 0–59.
    if h < 0.0 || !(0.0..=59.0).contains(&m) || !(0.0..=59.0).contains(&s) {
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    }
    Value::Num(h.trunc() * 3600.0 + m.trunc() * 60.0 + s.trunc())
}

pub(crate) fn fn_dhms(args: &[Value], ctx: &mut EvalCtx) -> Value {
    // DHMS(date, hour, minute, second) → datetime.
    let d = match args.first() {
        None => return Value::missing(),
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    let h = match args.get(1) {
        None => 0.0,
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    let m = match args.get(2) {
        None => 0.0,
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    let s = match args.get(3) {
        None => 0.0,
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::missing(),
            Some(f) => f,
        },
    };
    if h < 0.0 || !(0.0..=59.0).contains(&m) || !(0.0..=59.0).contains(&s) {
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    }
    let time = h.trunc() * 3600.0 + m.trunc() * 60.0 + s.trunc();
    Value::Num(d * SECONDS_PER_DAY + time)
}

pub(crate) fn fn_hour(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |dt| (split_datetime(dt).1 / 3600.0).floor())
}

pub(crate) fn fn_minute(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |dt| ((split_datetime(dt).1 % 3600.0) / 60.0).floor())
}

pub(crate) fn fn_second(args: &[Value], ctx: &mut EvalCtx) -> Value {
    unary_num(args, ctx, |dt| (split_datetime(dt).1 % 60.0).trunc())
}

pub(crate) fn fn_nldate(args: &[Value], ctx: &mut EvalCtx) -> Value {
    let date = match args.first() {
        None => return Value::Char(String::new()),
        Some(v) => match coerce_num(v, ctx) {
            None => return Value::Char(String::new()),
            Some(f) => f.trunc() as i64,
        },
    };
    // La langue (EN/FR/...) ne change rien dans cette implémentation simplifiée.
    let _lang = match args.get(1) {
        Some(Value::Char(s)) => s.trim().to_uppercase(),
        _ => "EN".to_string(),
    };
    Value::Char(format_date9(date))
}
