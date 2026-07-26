use super::*;

// ────────────── squelette du statement MODEL (MQ4.6 — model procs) ──────────────
//
// Le squelette « réponse → '=' → effets jusqu'à `/` ou `;` » du statement
// MODEL était recopié dans mixed/glimmix/genmod/logistic (mono-réponse) et
// glm/anova (multi-réponses, termes d'interaction `a*b`). Les briques
// ci-dessous sont extraites verbatim de ces procs. Les messages d'erreur
// diffèrent entre familles (« expected response variable in MODEL » pour
// MIXED/GLIMMIX vs « expected response variable » pour GENMOD/LOGISTIC…) :
// ils restent fournis par l'appelant afin de préserver l'identité
// octet-à-octet des logs. Les options après `/` divergent proc par proc et
// restent locales.

/// Lit le nom de la variable réponse (partie gauche mono-réponse du MODEL) et
/// le consomme. Erreur `err_msg` (au span du token courant) si le token n'est
/// pas un identifiant. Extrait verbatim de `mixed::parse_model`.
pub(crate) fn parse_model_response(ts: &mut StatementStream, err_msg: &str) -> Result<String> {
    let response = ts
        .peek()
        .ident()
        .map(str::to_string)
        .ok_or_else(|| SasError::parse(err_msg, ts.peek().span))?;
    ts.next();
    Ok(response)
}

/// Exige puis consomme le `=` du statement MODEL ; sinon erreur `err_msg` au
/// span du token courant. Extrait verbatim de `mixed::parse_model`.
pub(crate) fn expect_model_eq(ts: &mut StatementStream, err_msg: &str) -> Result<()> {
    if ts.peek().kind != TokenKind::Eq {
        return Err(SasError::parse(err_msg, ts.peek().span));
    }
    ts.next();
    Ok(())
}

/// Options de réponse optionnelles `(event='val' descending …)` entre la
/// réponse et le `=` (GLIMMIX/GENMOD/LOGISTIC). Sans parenthèse ouvrante, ne
/// consomme rien. Les tokens inconnus dans la parenthèse sont ignorés.
/// Renvoie `(event, descending)`. Extrait verbatim de `glimmix::parse_model`.
pub(crate) fn parse_response_options(ts: &mut StatementStream) -> (Option<String>, bool) {
    let mut event: Option<String> = None;
    let mut descending = false;
    if ts.peek().kind == TokenKind::LParen {
        ts.next();
        loop {
            if ts.peek().kind == TokenKind::RParen
                || ts.peek().kind == TokenKind::Semi
                || ts.peek().kind == TokenKind::Eof
            {
                break;
            }
            if ts.peek().is_kw("event") {
                ts.next();
                if ts.peek().kind == TokenKind::Eq {
                    ts.next();
                    if let TokenKind::Str { value, .. } = &ts.peek().kind.clone() {
                        event = Some(value.clone());
                        ts.next();
                    }
                }
            } else if ts.peek().is_kw("descending") {
                descending = true;
                ts.next();
            } else {
                ts.next();
            }
        }
        if ts.peek().kind == TokenKind::RParen {
            ts.next();
        }
    }
    (event, descending)
}

/// Liste plate d'effets : identifiants jusqu'à `/`, `;` ou Eof (le
/// terminateur n'est PAS consommé) ; tout autre token est ignoré. Sert aux
/// effets fixes du MODEL (MIXED/GLIMMIX), aux prédicteurs (GENMOD/LOGISTIC)
/// et aux effets du RANDOM (MIXED/GLIMMIX). Extrait verbatim de
/// `mixed::parse_model`.
pub(crate) fn parse_effect_list(ts: &mut StatementStream) -> Vec<String> {
    let mut effects: Vec<String> = Vec::new();
    while ts.peek().kind != TokenKind::Semi
        && ts.peek().kind != TokenKind::Slash
        && ts.peek().kind != TokenKind::Eof
    {
        if let Some(name) = ts.peek().ident().map(str::to_string) {
            effects.push(name);
        }
        ts.next();
    }
    effects
}

/// Partie gauche multi-réponses de GLM/ANOVA : identifiants jusqu'à `=`, `;`
/// ou Eof, puis consomme le `=` s'il est présent (pas d'erreur sinon —
/// fidèle aux parsers d'origine). Extrait verbatim de `glm::parse`.
pub(crate) fn parse_model_lhs(ts: &mut StatementStream) -> Vec<String> {
    let mut dependents: Vec<String> = Vec::new();
    loop {
        if ts.peek().kind == TokenKind::Semi
            || ts.peek().kind == TokenKind::Eof
            || ts.peek().kind == TokenKind::Eq
        {
            break;
        }
        if let Some(name) = ts.peek().ident().map(str::to_string) {
            dependents.push(name);
            ts.next();
        } else {
            ts.next();
        }
    }
    if ts.peek().kind == TokenKind::Eq {
        ts.next();
    }
    dependents
}

/// Effets avec chaînes d'interaction `a*b*c` (GLM/ANOVA) jusqu'à `/`, `;` ou
/// Eof (le terminateur n'est PAS consommé). Renvoie la représentation plate
/// (parties jointes par `*`) ET les termes structurés (une liste de noms par
/// terme) pour le moteur multiway. Extrait verbatim de `glm::parse`.
pub(crate) fn parse_effect_terms(ts: &mut StatementStream) -> (Vec<String>, Vec<Vec<String>>) {
    let mut effects: Vec<String> = Vec::new();
    let mut terms: Vec<Vec<String>> = Vec::new();
    loop {
        if ts.peek().kind == TokenKind::Semi
            || ts.peek().kind == TokenKind::Eof
            || ts.peek().kind == TokenKind::Slash
        {
            break;
        }
        if let Some(name) = ts.peek().ident().map(str::to_string) {
            ts.next();
            // Build the structured term: name, then any `* name` continuations.
            let mut parts: Vec<String> = vec![name];
            while ts.peek().kind == TokenKind::Star {
                ts.next();
                if let Some(next_name) = ts.peek().ident().map(str::to_string) {
                    parts.push(next_name);
                    ts.next();
                } else {
                    break;
                }
            }
            // Legacy flat representation: join interaction parts with `*`.
            effects.push(parts.join("*"));
            terms.push(parts);
        } else {
            ts.next();
        }
    }
    (effects, terms)
}

/// Reconstruit le bloc de covariance UN(t) (t×t) à partir des paramètres
/// empaquetés en triangulaire inférieure, dans l'ordre SAS : UN(1,1), UN(2,1),
/// UN(2,2), UN(3,1), … Partagé par PROC MIXED et PROC GLIMMIX (MQ8.3).
// Indices explicites : `r`/`c` parcourent le triangle inférieur d'une matrice
// symétrique, la forme itérateur n'y dit rien de plus (cf. MQ7.2c).
#[allow(clippy::needless_range_loop)]
pub(crate) fn un_block(theta: &[f64], t: usize) -> Vec<Vec<f64>> {
    let mut m = vec![vec![0.0; t]; t];
    let mut k = 0;
    for r in 0..t {
        for c in 0..=r {
            let val = theta[k];
            m[r][c] = val;
            m[c][r] = val;
            k += 1;
        }
    }
    m
}

/// Lignes COMPLÈTES (aucune valeur manquante) des colonnes décodées, dans
/// l'ordre des observations : `decoded` est en colonne-major, le résultat en
/// ligne-major. Une ligne portant un missing (donc un `f64` non fini) est
/// écartée entièrement — c'est la règle « complete case » de PROC PRINCOMP et
/// PROC FACTOR (MQ8.9 — les deux en avaient une copie identique).
pub(crate) fn complete_case_rows(decoded: &[Vec<f64>], n_read: usize) -> Vec<Vec<f64>> {
    let mut data_rows: Vec<Vec<f64>> = Vec::new();
    for r in 0..n_read {
        let row: Vec<f64> = decoded.iter().map(|col| col[r]).collect();
        if row.iter().all(|v| v.is_finite()) {
            data_rows.push(row);
        }
    }
    data_rows
}

/// Convention de signe SAS sur une matrice de vecteurs propres (colonnes) :
/// chaque colonne est orientée pour que son élément de plus GRANDE valeur
/// absolue soit positif. Sans cela le signe des composantes/facteurs dépend de
/// l'algorithme et le listing n'est pas reproductible.
///
/// MQ8.9 — copie identique dans `factor/analysis.rs` et `princomp/analysis.rs`.
// Indices explicites : `row`/`col` parcourent une matrice carrée (cf. MQ7.2c).
#[allow(clippy::needless_range_loop)]
pub(crate) fn apply_sign_convention(v: &mut [Vec<f64>], p: usize) {
    for col in 0..p {
        let mut max_abs = 0.0_f64;
        let mut max_val = 0.0_f64;
        for row in 0..p {
            let a = v[row][col].abs();
            if a > max_abs {
                max_abs = a;
                max_val = v[row][col];
            }
        }
        if max_val < 0.0 {
            for row in 0..p {
                v[row][col] = -v[row][col];
            }
        }
    }
}
