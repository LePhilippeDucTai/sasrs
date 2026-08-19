use super::*;

// ----------------------------------------------------------------------------
// Traducteur SqlExpr → polars::prelude::Expr
// ----------------------------------------------------------------------------

pub(super) fn sql_expr_to_polars(e: &SqlExpr, ctx: &Ctx) -> Result<Expr> {
    match e {
        SqlExpr::Base(b) => base_expr_to_polars(b, ctx),
        SqlExpr::Star => Err(SasError::runtime(
            "PROC SQL: '*' is only valid in a select-list.",
        )),
        SqlExpr::QualifiedStar(_) => Err(SasError::runtime(
            "PROC SQL: 'table.*' is only valid in a select-list.",
        )),
        SqlExpr::Qualified { column, .. } => Ok(col(column.clone())),
        SqlExpr::Calculated(name) => {
            let key = name.to_ascii_lowercase();
            let target = ctx
                .aliases
                .iter()
                .find(|(a, _)| *a == key)
                .map(|(_, ex)| ex.clone())
                .ok_or_else(|| {
                    SasError::runtime(format!(
                        "PROC SQL: CALCULATED {} refers to an unknown column.",
                        name.to_uppercase()
                    ))
                })?;
            sql_expr_to_polars(&target, ctx)
        }
        SqlExpr::Aggregate {
            func,
            distinct,
            arg,
            star,
        } => aggregate_to_polars(func, *distinct, arg.as_deref(), *star, ctx),
        SqlExpr::Between {
            expr,
            low,
            high,
            negated,
        } => {
            let a = sql_expr_to_polars(expr, ctx)?;
            let lo = sql_expr_to_polars(low, ctx)?;
            let hi = sql_expr_to_polars(high, ctx)?;
            let between = a.clone().gt_eq(lo).and(a.lt_eq(hi));
            Ok(if *negated { between.not() } else { between })
        }
        SqlExpr::IsNull { expr, negated } => {
            let a = sql_expr_to_polars(expr, ctx)?;
            Ok(if *negated {
                a.is_not_null()
            } else {
                a.is_null()
            })
        }
        SqlExpr::Like {
            expr,
            pattern,
            negated,
        } => {
            let a = sql_expr_to_polars(expr, ctx)?;
            let m = like_to_match(a, pattern)?;
            Ok(if *negated { m.not() } else { m })
        }
        SqlExpr::Contains {
            expr,
            pattern,
            negated,
        } => {
            let a = sql_expr_to_polars(expr, ctx)?;
            let m = contains_to_match(a, pattern)?;
            Ok(if *negated { m.not() } else { m })
        }
        SqlExpr::SoundsLike {
            expr,
            text,
            negated,
        } => {
            let a = sql_expr_to_polars(expr, ctx)?;
            let m = sounds_like_to_match(a, text)?;
            Ok(if *negated { m.not() } else { m })
        }
        SqlExpr::Binary { op, left, right } => binary_to_polars(*op, left, right, ctx),
        SqlExpr::Unary { op, expr } => {
            let a = sql_expr_to_polars(expr, ctx)?;
            Ok(match op {
                UnaryOp::Minus => lit(0.0) - a,
                UnaryOp::Plus => a,
                UnaryOp::Not => a.not(),
            })
        }
        // Les sous-requêtes sont résolues en littéraux par `resolve_subqueries`
        // AVANT l'abaissement. Si l'une survit ici, c'est un chemin non couvert
        // (ex. `translate_predicate` du DELETE, qui n'effectue pas la passe).
        SqlExpr::Subquery(_) | SqlExpr::InSubquery { .. } | SqlExpr::Exists { .. } => Err(
            SasError::runtime("PROC SQL: subqueries are not supported in this context."),
        ),
    }
}

/// Traduit un `Expr` (feuille « base ») en Polars.
pub(super) fn base_expr_to_polars(e: &SasExpr, ctx: &Ctx) -> Result<Expr> {
    match e {
        SasExpr::Num(n) => Ok(lit(*n)),
        SasExpr::Str(s) => Ok(lit(s.clone())),
        SasExpr::Missing(MissingKind::Dot) => Ok(lit(NULL)),
        // Tout missing en tant que littéral → null (les spéciaux sont
        // déjà normalisés en null sur les colonnes).
        SasExpr::Missing(_) => Ok(lit(NULL)),
        SasExpr::Var(name) => Ok(col(name.clone())),
        SasExpr::Binary { op, left, right } => base_binary_to_polars(*op, left, right, ctx),
        SasExpr::Unary { op, expr } => {
            let a = base_expr_to_polars(expr, ctx)?;
            Ok(match op {
                UnaryOp::Minus => lit(0.0) - a,
                UnaryOp::Plus => a,
                UnaryOp::Not => a.not(),
            })
        }
        SasExpr::In { expr, list } => {
            let a = base_expr_to_polars(expr, ctx)?;
            let items: Vec<Expr> = list
                .iter()
                .map(|x| base_expr_to_polars(x, ctx))
                .collect::<Result<_>>()?;
            Ok(a.is_in(concat_list(items)?))
        }
        SasExpr::Call { name, .. } => Err(SasError::runtime(format!(
            "PROC SQL: function {}() is not supported yet.",
            name.to_uppercase()
        ))),
        SasExpr::Index { name, .. } => Err(SasError::runtime(format!(
            "PROC SQL: array reference {} is not supported in SQL.",
            name.to_uppercase()
        ))),
        SasExpr::HashMethod(call) => Err(SasError::runtime(format!(
            "PROC SQL: hash method call on {} is not supported in SQL.",
            call.object.to_uppercase()
        ))),
    }
}

/// Comparaison `a = .` / `a <> .` → is_null / is_not_null.
pub(super) fn is_missing_literal(e: &SasExpr) -> bool {
    matches!(e, SasExpr::Missing(_))
}

pub(super) fn base_binary_to_polars(
    op: BinaryOp,
    left: &SasExpr,
    right: &SasExpr,
    ctx: &Ctx,
) -> Result<Expr> {
    // Egalité/inégalité contre un littéral missing.
    if matches!(op, BinaryOp::Eq) {
        if is_missing_literal(right) {
            return Ok(base_expr_to_polars(left, ctx)?.is_null());
        }
        if is_missing_literal(left) {
            return Ok(base_expr_to_polars(right, ctx)?.is_null());
        }
    }
    if matches!(op, BinaryOp::Ne) {
        if is_missing_literal(right) {
            return Ok(base_expr_to_polars(left, ctx)?.is_not_null());
        }
        if is_missing_literal(left) {
            return Ok(base_expr_to_polars(right, ctx)?.is_not_null());
        }
    }
    let l = base_expr_to_polars(left, ctx)?;
    let r = base_expr_to_polars(right, ctx)?;
    Ok(apply_binop(op, l, r))
}

pub(super) fn binary_to_polars(
    op: BinaryOp,
    left: &SqlExpr,
    right: &SqlExpr,
    ctx: &Ctx,
) -> Result<Expr> {
    // Missing literal en SqlExpr::Base.
    if matches!(op, BinaryOp::Eq | BinaryOp::Ne) {
        let l_missing = matches!(left, SqlExpr::Base(b) if is_missing_literal(b));
        let r_missing = matches!(right, SqlExpr::Base(b) if is_missing_literal(b));
        if r_missing {
            let a = sql_expr_to_polars(left, ctx)?;
            return Ok(if op == BinaryOp::Eq {
                a.is_null()
            } else {
                a.is_not_null()
            });
        }
        if l_missing {
            let a = sql_expr_to_polars(right, ctx)?;
            return Ok(if op == BinaryOp::Eq {
                a.is_null()
            } else {
                a.is_not_null()
            });
        }
    }
    let l = sql_expr_to_polars(left, ctx)?;
    let r = sql_expr_to_polars(right, ctx)?;
    Ok(apply_binop(op, l, r))
}

pub(super) fn apply_binop(op: BinaryOp, l: Expr, r: Expr) -> Expr {
    match op {
        BinaryOp::Add => l + r,
        BinaryOp::Sub => l - r,
        BinaryOp::Mul => l * r,
        BinaryOp::Div => l / r,
        BinaryOp::Power => l.pow(r),
        BinaryOp::Concat => l.cast(DataType::String) + r.cast(DataType::String),
        BinaryOp::Lt => l.lt(r),
        BinaryOp::Le => l.lt_eq(r),
        BinaryOp::Gt => l.gt(r),
        BinaryOp::Ge => l.gt_eq(r),
        BinaryOp::Eq => l.eq(r),
        BinaryOp::Ne => l.neq(r),
        BinaryOp::And => l.and(r),
        BinaryOp::Or => l.or(r),
    }
}

pub(super) fn aggregate_to_polars(
    func: &str,
    distinct: bool,
    arg: Option<&SqlExpr>,
    star: bool,
    ctx: &Ctx,
) -> Result<Expr> {
    let f = func.to_ascii_lowercase();
    match f.as_str() {
        "count" => match arg {
            // `COUNT(*)` (ou `COUNT()`) : cardinalité, pas de colonne.
            None => Ok(len()),
            Some(_) if star => Ok(len()),
            Some(inner) => {
                let a = sql_expr_to_polars(inner, ctx)?;
                if distinct {
                    Ok(a.n_unique())
                } else {
                    Ok(a.count())
                }
            }
        },
        "sum" | "avg" | "mean" | "min" | "max" => {
            let arg = arg.ok_or_else(|| {
                SasError::runtime(format!(
                    "PROC SQL: aggregate {}() requires an argument.",
                    func.to_uppercase()
                ))
            })?;
            let a = sql_expr_to_polars(arg, ctx)?;
            let a = if distinct { a.unique() } else { a };
            Ok(match f.as_str() {
                "sum" => a.sum(),
                "avg" | "mean" => a.mean(),
                "min" => a.min(),
                "max" => a.max(),
                _ => unreachable!(),
            })
        }
        other => Err(SasError::runtime(format!(
            "PROC SQL: aggregate function {}() is not supported.",
            other.to_uppercase()
        ))),
    }
}

/// Traduit un prédicat SQL `expr LIKE pattern` en expression Polars.
///
/// Sémantique SAS du LIKE (cf. SAS SQL) :
///   - `%`  : correspond à zéro caractère ou plus,
///   - `_`  : correspond à exactement un caractère,
///   - tout autre caractère se compare littéralement,
///   - la comparaison est **sensible à la casse** (contrairement à `=` SAS
///     qui l'est aussi sur les char ; SAS ne fait PAS de upcase ici),
///   - une valeur missing (null) ne matche jamais → résultat null/false.
///
/// On n'utilise PAS la feature `regex` de Polars (non activée). Pour couvrir
/// l'intégralité des motifs (y compris `_`, les `%` internes et la forme
/// substring `%abc%`), on optimise les cas courants en primitives Polars
/// (`eq` / `starts_with` / `ends_with` / `contains_literal`) et on retombe sur
/// un matcher SAS maison appliqué via `Expr::map` pour les cas généraux.
pub(super) fn like_to_match(a: Expr, pattern: &str) -> Result<Expr> {
    // Cas spéciaux purement composés de jokers `%` → tout non-missing matche.
    // (`%`, `%%`, ... = "zéro ou plus" répété = "n'importe quoi".)
    if !pattern.is_empty() && pattern.chars().all(|c| c == '%') {
        return Ok(a.clone().is_not_null());
    }

    // Optimisations : motifs sans `_` et sans plusieurs `%` internes.
    // On les traduit en primitives Polars natives (plus rapides, vectorisées).
    // Pour la forme `%abc%`, on retombe sur le matcher maison pour éviter
    // les dépendances regex.
    if !pattern.contains('_') {
        let leading = pattern.starts_with('%');
        let trailing = pattern.ends_with('%');
        let core = pattern.trim_matches('%');
        if !core.contains('%') && (leading, trailing) != (true, true) {
            let core = core.to_string();
            return Ok(match (leading, trailing) {
                // Pas de joker du tout → égalité exacte.
                (false, false) => a.eq(lit(core)),
                // `abc%` → commence par "abc".
                (false, true) => a.str().starts_with(lit(core)),
                // `%abc` → finit par "abc".
                (true, false) => a.str().ends_with(lit(core)),
                // `%abc%` → gérée par le matcher maison ci-dessous.
                (true, true) => unreachable!(),
            });
        }
    }

    // Cas général (joker `_`, ou plusieurs `%` internes) : matcher SAS maison
    // appliqué élément par élément via une UDF Polars renvoyant un booléen.
    let pat = pattern.to_string();
    Ok(a.map(
        move |col: Column| {
            let s = col.str()?;
            let out: BooleanChunked = s
                .iter()
                .map(|opt| opt.map(|v| sas_like_match(v, &pat)))
                .collect();
            Ok(Some(out.into_column()))
        },
        GetOutput::from_type(DataType::Boolean),
    ))
}

/// Matcher SAS `LIKE` pour une seule valeur (sensible à la casse) :
/// `%` = 0+ caractères, `_` = exactement 1 caractère, le reste littéral.
/// Implémentation par backtracking glob classique (sur les `char`, pour gérer
/// l'UTF-8 correctement).
pub(super) fn sas_like_match(text: &str, pattern: &str) -> bool {
    let t: Vec<char> = text.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    // i : index dans le texte, j : index dans le motif.
    let (mut i, mut j) = (0usize, 0usize);
    // Dernier `%` rencontré et position du texte au moment de ce `%` : permet
    // le backtracking (avancer d'un caractère dans le texte si la suite échoue).
    let mut star_j: Option<usize> = None;
    let mut star_i = 0usize;
    while i < t.len() {
        if j < p.len() && (p[j] == t[i] || p[j] == '_') {
            i += 1;
            j += 1;
        } else if j < p.len() && p[j] == '%' {
            star_j = Some(j);
            star_i = i;
            j += 1;
        } else if let Some(sj) = star_j {
            // Échec : le dernier `%` absorbe un caractère de plus.
            j = sj + 1;
            star_i += 1;
            i = star_i;
        } else {
            return false;
        }
    }
    // Texte épuisé : le reste du motif doit être uniquement des `%`.
    while j < p.len() && p[j] == '%' {
        j += 1;
    }
    j == p.len()
}

/// Traduit un prédicat SQL `expr CONTAINS 'sous-chaîne'` (M42.2) en expression
/// Polars.
///
/// Sémantique Oracle/SAS : `CONTAINS` ≡ `INDEX(expr, 'sous-chaîne') > 0`
/// (cf. `datastep::functions::char::search::fn_index`) — recherche de
/// sous-chaîne littérale, **sensible à la casse**, position 1-based/0 si
/// absente. On n'appelle PAS `fn_index` directement : cette fonction attend
/// un `EvalCtx` complet (compteurs de notes, RNG, hash objects...) propre au
/// moteur DATA step, hors de propos pour un prédicat évalué colonne par
/// colonne ici. Son cœur (`str::find`) est trivial à réappliquer sans
/// dupliquer de LOGIQUE (seule la recherche de sous-chaîne compte ; SAS ne
/// distingue pas les positions au-delà de "trouvé/pas trouvé" pour CONTAINS).
///
/// Comme pour `like_to_match`, la feature `regex` de Polars n'étant pas
/// activée, on ne peut pas utiliser `Expr::str().contains_literal` : on
/// retombe sur le même mécanisme d'UDF `Expr::map`.
///
/// `INDEX(s, '')` renvoie toujours 0 (chaîne vide jamais "trouvée") : donc
/// `CONTAINS ''` est toujours faux, y compris pour une valeur non manquante.
/// Une valeur missing (null) ne "contient" jamais rien → résultat null/false
/// (comme LIKE).
pub(super) fn contains_to_match(a: Expr, pattern: &str) -> Result<Expr> {
    if pattern.is_empty() {
        return Ok(lit(false));
    }
    let pat = pattern.to_string();
    Ok(a.map(
        move |col: Column| {
            let s = col.str()?;
            let out: BooleanChunked = s
                .iter()
                .map(|opt| opt.map(|v| v.contains(pat.as_str())))
                .collect();
            Ok(Some(out.into_column()))
        },
        GetOutput::from_type(DataType::Boolean),
    ))
}

/// Traduit un prédicat SQL `expr SOUNDS LIKE 'texte'` (M42.2) en expression
/// Polars.
///
/// Sémantique Oracle/SAS : `SOUNDS LIKE` ≡ `SOUNDEX(expr) = SOUNDEX('texte')`.
/// Le code Soundex de `'texte'` est constant (littéral du prédicat) : on le
/// calcule UNE fois hors de la fermeture, puis on compare le Soundex de
/// chaque valeur de colonne à cette constante via une UDF `Expr::map`, même
/// mécanisme que `like_to_match`/`contains_to_match` (pas de fonction Soundex
/// native dans l'API Polars disponible ici).
pub(super) fn sounds_like_to_match(a: Expr, text: &str) -> Result<Expr> {
    let target = soundex(text);
    Ok(a.map(
        move |col: Column| {
            let s = col.str()?;
            let out: BooleanChunked = s
                .iter()
                .map(|opt| opt.map(|v| soundex(v) == target))
                .collect();
            Ok(Some(out.into_column()))
        },
        GetOutput::from_type(DataType::Boolean),
    ))
}

/// Code Soundex standard (algorithme américain classique, celui qu'implémente
/// `SOUNDEX()` en SAS) : 1 lettre + 3 chiffres, complété par des zéros ou
/// tronqué.
///
/// Table de codage (insensible à la casse ; les caractères non alphabétiques
/// sont ignorés, comme s'ils n'existaient pas) :
///   B,F,P,V → 1 · C,G,J,K,Q,S,X,Z → 2 · D,T → 3 · L → 4 · M,N → 5 · R → 6
///   A,E,I,O,U,H,W,Y → pas de chiffre.
///
/// Règle d'adjacence (celle qui distingue les implémentations « naïves » de
/// la vraie table Soundex) : deux lettres de MÊME code ne comptent qu'une
/// fois si elles sont adjacentes OU séparées uniquement par H/W — H et W
/// sont transparents, ils NE rompent PAS la fusion (exemple canonique
/// "Ashcraft" → A261 : le S et le C, tous deux code 2 et séparés par le H,
/// fusionnent comme s'ils étaient adjacents). Une voyelle (A/E/I/O/U ou Y)
/// entre deux lettres de même code, elle, ROMPT la fusion : les deux
/// chiffres sont alors conservés. Attention : cette règle de fusion ne
/// s'applique PAS entre la lettre initiale et la lettre suivante même si
/// elles partagent le même code — exemple "Pfister" → P123 (le F n'est PAS
/// fusionné avec le P initial, contrairement à l'intuition « P et F sont
/// dans le même groupe » ; seules les lettres codées APRÈS la première
/// participent entre elles à la règle de fusion).
///
/// Chaîne vide ou sans aucune lettre → `"0000"` (pas de lettre initiale à
/// conserver ; comportement observé de SAS pour une entrée vide — choix
/// documenté ici faute de pouvoir le vérifier directement contre un SAS réel
/// dans cet environnement).
pub(super) fn soundex(s: &str) -> String {
    fn digit(c: char) -> Option<u8> {
        match c.to_ascii_uppercase() {
            'B' | 'F' | 'P' | 'V' => Some(1),
            'C' | 'G' | 'J' | 'K' | 'Q' | 'S' | 'X' | 'Z' => Some(2),
            'D' | 'T' => Some(3),
            'L' => Some(4),
            'M' | 'N' => Some(5),
            'R' => Some(6),
            _ => None,
        }
    }

    let letters: Vec<char> = s.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    let Some(&first) = letters.first() else {
        return "0000".to_string();
    };

    let mut code = String::with_capacity(4);
    code.push(first.to_ascii_uppercase());
    // Dernier chiffre RETENU (pour la règle de fusion) parmi les lettres
    // APRÈS la première — volontairement `None` au départ (cf. doc-comment
    // ci-dessus : "Pfister" → P123, le F n'est pas fusionné avec le P).
    let mut last: Option<u8> = None;
    for &c in &letters[1..] {
        if code.len() == 4 {
            break;
        }
        let uc = c.to_ascii_uppercase();
        if uc == 'H' || uc == 'W' {
            // H/W : transparents, ne rompent PAS la fusion en cours.
            continue;
        }
        match digit(uc) {
            Some(d) => {
                if Some(d) != last {
                    code.push((b'0' + d) as char);
                }
                last = Some(d);
            }
            // Voyelle (ou Y) : rompt la fusion pour la suite.
            None => last = None,
        }
    }

    while code.len() < 4 {
        code.push('0');
    }
    code
}
