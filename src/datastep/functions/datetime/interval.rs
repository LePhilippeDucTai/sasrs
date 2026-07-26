use super::*;

// ──────────────────────────────────────────────────────────────────────────────
// Interval date functions : INTCK / INTNX
// ──────────────────────────────────────────────────────────────────────────────

/// Parsed interval keyword (premier argument caractère de INTCK/INTNX).
pub(crate) enum Interval {
    Day,
    Week,
    Month,
    Qtr,
    Year,
}

/// Parse l'intervalle (insensible à la casse, blancs de bord supprimés).
/// Renvoie None pour un intervalle inconnu.
pub(crate) fn parse_interval(v: &Value) -> Option<Interval> {
    let s = match v {
        Value::Char(s) => s.trim().to_uppercase(),
        _ => return None,
    };
    match s.as_str() {
        "DAY" => Some(Interval::Day),
        "WEEK" => Some(Interval::Week),
        "MONTH" => Some(Interval::Month),
        "QTR" | "QUARTER" => Some(Interval::Qtr),
        "YEAR" => Some(Interval::Year),
        _ => None,
    }
}

/// Index de semaine SAS (les semaines commencent le DIMANCHE). Le jour SAS 0
/// (1960-01-01) est un VENDREDI ; le dimanche le plus récent à cette date est
/// le jour -5 (1959-12-27), et le dimanche suivant est le jour 2 (1960-01-03).
/// `floor((d - 2) / 7)` place donc chaque dimanche (… -5, 2, 9 …) sur une
/// frontière. On utilise une division euclidienne pour gérer correctement les
/// jours négatifs.
pub(crate) fn week_index(sas_day: i64) -> i64 {
    (sas_day - 2).div_euclid(7)
}

/// INTCK('interval', from, to) → nombre discret de frontières d'intervalle
/// franchies (méthode "DISCRETE" par défaut de SAS). Intervalle inconnu ou
/// date manquante → missing.
pub(crate) fn fn_intck(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 3 {
        ctx.invalid_data += 1;
        return Value::missing();
    }
    let Some(interval) = parse_interval(&args[0]) else {
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    };
    let from = match coerce_num(&args[1], ctx) {
        None => return Value::missing(),
        Some(f) => f.floor() as i64,
    };
    let to = match coerce_num(&args[2], ctx) {
        None => return Value::missing(),
        Some(f) => f.floor() as i64,
    };
    let (y1, m1, _d1) = sas_date_to_ymd(from);
    let (y2, m2, _d2) = sas_date_to_ymd(to);
    let count = match interval {
        Interval::Day => (to - from) as f64,
        Interval::Week => (week_index(to) - week_index(from)) as f64,
        Interval::Month => ((y2 * 12 + m2) - (y1 * 12 + m1)) as f64,
        Interval::Qtr => {
            let q1 = (m1 - 1) / 3; // 0-based quarter index
            let q2 = (m2 - 1) / 3;
            ((y2 * 4 + q2) - (y1 * 4 + q1)) as f64
        }
        Interval::Year => (y2 - y1) as f64,
    };
    Value::Num(count)
}

/// Alignement de INTNX (4e argument optionnel, défaut BEGINNING).
pub(crate) enum Align {
    Beginning,
    End,
    Same,
    Middle,
}

pub(crate) fn parse_align(v: Option<&Value>) -> Align {
    let s = match v {
        Some(Value::Char(s)) => s.trim().to_uppercase(),
        _ => return Align::Beginning,
    };
    // On matche sur le premier caractère significatif (B/E/S/M).
    match s.chars().next() {
        Some('E') => Align::End,
        Some('S') => Align::Same,
        Some('M') => Align::Middle,
        _ => Align::Beginning, // 'B'/BEG/BEGINNING et tout le reste
    }
}

/// INTNX('interval', start, increment [, 'alignment']) → date SAS.
/// Date manquante / intervalle inconnu → missing.
pub(crate) fn fn_intnx(args: &[Value], ctx: &mut EvalCtx) -> Value {
    if args.len() < 3 {
        ctx.invalid_data += 1;
        return Value::missing();
    }
    let Some(interval) = parse_interval(&args[0]) else {
        ctx.invalid_data += 1;
        ctx.error_flag = true;
        return Value::missing();
    };
    let start = match coerce_num(&args[1], ctx) {
        None => return Value::missing(),
        Some(f) => f.floor() as i64,
    };
    let inc = match coerce_num(&args[2], ctx) {
        None => return Value::missing(),
        Some(f) => f.trunc() as i64,
    };
    let align = parse_align(args.get(3));
    let (sy, sm, sd) = sas_date_to_ymd(start);

    let (y, m, d) = match interval {
        Interval::Day => {
            // Période = 1 jour ; alignement sans objet (B=E=S=start+inc).
            return Value::Num((start + inc) as f64);
        }
        Interval::Week => {
            // Période = 7 jours débutant un dimanche.
            // Le dimanche d'index k est le jour 7*k + 2 (cf. week_index :
            // … -5, 2, 9 …). Dimanche de la semaine de `start` :
            let start_sunday = week_index(start) * 7 + 2;
            let target_sunday = start_sunday + inc * 7;
            let day = match align {
                Align::Beginning => target_sunday,
                Align::End => target_sunday + 6, // samedi
                Align::Same => target_sunday + (start - start_sunday), // même jour de semaine
                Align::Middle => target_sunday + 3, // milieu : mercredi
            };
            return Value::Num(day as f64);
        }
        Interval::Month => {
            // Période = mois civil. Début de période = (sy, sm, 1).
            let (ny, nm) = normalize_ym(sy, (sm - 1) + inc);
            let last = last_day_of_month(ny, nm);
            let d = match align {
                Align::Beginning => 1,
                Align::End => last,
                Align::Same => sd.min(last),
                Align::Middle => 15,
            };
            (ny, nm, d)
        }
        Interval::Qtr => {
            // Période = trimestre (mois de début 1, 4, 7, 10).
            let q0 = (sm - 1) / 3; // 0-based quarter of start
            let total_q = sy * 4 + q0 + inc;
            let ny = total_q.div_euclid(4);
            let nq = total_q.rem_euclid(4); // 0..3
            let first_month = nq * 3 + 1;

            match align {
                Align::Beginning => (ny, first_month, 1),
                Align::End => {
                    let last_month = first_month + 2;
                    (ny, last_month, last_day_of_month(ny, last_month))
                }
                Align::Same => {
                    // Même offset (mois dans le trimestre + jour) que start.
                    let month_in_q = (sm - 1) % 3; // 0..2
                    let tm = first_month + month_in_q;
                    let last = last_day_of_month(ny, tm);
                    (ny, tm, sd.min(last))
                }
                Align::Middle => {
                    // Milieu du trimestre ≈ 15 du mois central.
                    (ny, first_month + 1, 15)
                }
            }
        }
        Interval::Year => {
            let ny = sy + inc;
            match align {
                Align::Beginning => (ny, 1, 1),
                Align::End => (ny, 12, 31),
                Align::Same => {
                    let last = last_day_of_month(ny, sm);
                    (ny, sm, sd.min(last))
                }
                Align::Middle => (ny, 7, 1),
            }
        }
    };

    Value::Num(days_since_1960(y, m, d) as f64)
}
