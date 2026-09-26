//! J03-P6 — tests de non-régression des divergences corrigées des fonctions
//! caractères. CHAQUE valeur attendue cite sa source dans la doc SAS
//! (SAS 9.4 Functions Reference, lefunctionsref — mirror PDF japonais
//! consulté : https://www.sas.com/offices/asiapacific/japan/service/help/pdf/lefunctionsref.pdf).
use super::*;

// ── FIND : position de départ INCLUSE (J03-P6) ───────────────────────────

#[test]
fn datastep_divergence_find_doc_examples() {
    // Doc FIND, « サンプル » :
    //   find('She sells seashells? Yes, she does.','she ') = 27
    //   find(variable1,variable2,'i') = 1  (modifiers 'i', cible 'she ')
    //   find(xyz,'she',22) = 27 (startpos 22, recherche vers la droite)
    // NB : la crate prend les modifiers en 4ᵉ position (l'ordre des
    // arguments de la doc où 'i' est en 3ᵉ position n'est pas supporté —
    // divergence de signature hors périmètre J03-P6).
    assert_eq!(
        invoke(
            "FIND",
            &[chr("She sells seashells? Yes, she does."), chr("she ")]
        ),
        num(27.0)
    );
    assert_eq!(
        invoke(
            "FIND",
            &[
                chr("She sells seashells? Yes, she does."),
                chr("she "),
                num(1.0),
                chr("i")
            ]
        ),
        num(1.0)
    );
    assert_eq!(
        invoke(
            "FIND",
            &[
                chr("She sells seashells? Yes, she does."),
                chr("she"),
                num(22.0)
            ]
        ),
        num(27.0)
    );
}

#[test]
fn datastep_divergence_find_start_pos_is_inclusive() {
    // Doc FIND : « startpos の位置から検索を開始し、右方向に検索します » —
    // la recherche COMMENCE à startpos (incluse). Périmètre J03-P6 :
    // `FIND('abc','a')` = 1. La position de départ n'est JAMAIS sautée,
    // même quand elle porte elle-même l'occurrence.
    assert_eq!(invoke("FIND", &[chr("abc"), chr("a")]), num(1.0));
    assert_eq!(invoke("FIND", &[chr("abc"), chr("a"), num(1.0)]), num(1.0));
    // 'o' est EN position 5 de 'hello world'.
    assert_eq!(
        invoke("FIND", &[chr("hello world"), chr("o"), num(5.0)]),
        num(5.0)
    );
    assert_eq!(
        invoke("FIND", &[chr("hello"), chr("o"), num(4.0)]),
        num(5.0)
    );
    // startpos au-delà de la longueur → 0 (doc : « startpos が string の長さ
    // よりも大きい場合、FIND は値 0 を返します »).
    assert_eq!(invoke("FIND", &[chr("abc"), chr("a"), num(4.0)]), num(0.0));
}

// ── COMPBL : 2+ espaces → 1 espace ; les tabulations ne sont pas des
// blancs (I18N niveau 0) ──────────────────────────────────────────────────

#[test]
fn datastep_divergence_compbl_doc_semantics() {
    // Doc COMPBL : « 文字列中に 2 つ以上の空白が連続して出現するたびに、
    // COMPBL 関数はそれらを 1 つの空白に変換して、複数の空白を削除します »
    // — chaque RUN de 2+ espaces devient UN espace ; un blanc isolé est
    // inchangé (« COMPBL 関数は複数の空白を 1 つの空白に圧縮しますが、1
    // つの空白には影響しません »).
    assert_eq!(
        invoke("COMPBL", &[chr("hello    world")]),
        chr("hello world")
    );
    assert_eq!(
        invoke("COMPBL", &[chr("125 E  Main St")]),
        chr("125 E Main St")
    );
    // Les blancs de bord (runs de 2+) deviennent UN blanc : ils ne sont pas
    // supprimés entièrement.
    assert_eq!(
        invoke("COMPBL", &[chr("  hello world  ")]),
        chr(" hello world ")
    );
    // Un seul blanc de chaque côté : inchangé.
    assert_eq!(
        invoke("COMPBL", &[chr(" hello world ")]),
        chr(" hello world ")
    );
}

#[test]
fn datastep_divergence_compbl_tabs_are_not_blanks() {
    // Fonction I18N niveau 0 (SBCS) : « blank » = espace 0x20. Les
    // tabulations ne sont ni supprimées ni fusionnées ; chaque run
    // d'espaces autour de la tabulation devient un espace.
    assert_eq!(
        invoke("COMPBL", &[chr("hello  \t  world")]),
        chr("hello \t world")
    );
    assert_eq!(invoke("COMPBL", &[chr("a\tb")]), chr("a\tb"));
}

// ── STRIP/CATS/CATX : espaces (0x20) uniquement, pas les tabulations ─────

#[test]
fn datastep_divergence_strip_spaces_only() {
    // Doc STRIP : « STRIP 関数は引数の先頭と末尾から空白をすべて削除して
    // 返します » — niveau I18N 0 : espaces 0x20, pas les tabulations.
    assert_eq!(invoke("STRIP", &[chr("   hello   ")]), chr("hello"));
    assert_eq!(invoke("STRIP", &[chr("\thello\t")]), chr("\thello\t"));
    assert_eq!(invoke("STRIP", &[chr(" \thello ")]), chr("\thello"));
}

#[test]
fn datastep_divergence_cats_spaces_only() {
    // Doc CATS : « 先頭と末尾の空白を削除して、連結文字列を返します »
    // (niveau I18N 0 : espaces 0x20).
    assert_eq!(invoke("CATS", &[chr(" a "), chr(" b ")]), chr("ab"));
    // Les tabulations ne sont PAS retirées.
    assert_eq!(invoke("CATS", &[chr(" a "), chr("\tb\t")]), chr("a\tb\t"));
}

#[test]
fn datastep_divergence_catx_spaces_only() {
    // Doc CATX : chaque item est « stripé » (espaces 0x20), un item sans
    // aucun caractère non-blanc est sauté, pas de délimiteur doublé ni en
    // tête/queue.
    assert_eq!(
        invoke("CATX", &[chr("-"), chr(" a "), chr(" b ")]),
        chr("a-b")
    );
    // ' b ' entièrement blanc → sauté ; '\tb\t' contient un caractère
    // non-blanc ('b') → conservé AVEC ses tabulations.
    assert_eq!(
        invoke("CATX", &[chr("-"), chr(" a "), chr("   "), chr("\tb\t")]),
        chr("a-\tb\t")
    );
}

// ── REPEAT : plafond en CARACTÈRES (J03-P6) ──────────────────────────────

#[test]
fn datastep_divergence_repeat_cap_in_chars() {
    // Doc REPEAT (I18N niveau 2, SBCS/DBCS/MBCS UTF-8) : le résultat est
    // compté en caractères. La garde MQ9.1 (anti-OOM) plafonne désormais à
    // 32 767 CARACTÈRES — cohérent avec les largeurs en caractères de
    // J03-P3/P4/P5 — et non plus 32 767 octets : « é » (2 octets) donne
    // 32 767 caractères (65 534 octets), plus 16 383 comme avant.
    let out = invoke("REPEAT", &[chr("é"), num(1e9)]);
    match out {
        Value::Char(s) => {
            assert_eq!(s.chars().count(), 32_767, "plafond en caractères");
            assert_eq!(s.len(), 65_534, "32 767 'é' = 65 534 octets");
        }
        other => panic!("attendu Char, obtenu {other:?}"),
    }
}

#[test]
fn datastep_divergence_repeat_normal_multibyte() {
    // Doc REPEAT, « サンプル » : repeat('ONE',2) = 'ONEONEONE' (sémantique
    // n+1). La crate conserve la sémantique « n copies » historique
    // (divergence documentée, hors périmètre J03-P6) : on vérifie juste le
    // passage multioctet INTACT (pas de troncature d'octet).
    assert_eq!(invoke("REPEAT", &[chr("é"), num(3.0)]), chr("ééé"));
    assert_eq!(invoke("REPEAT", &[chr("éü"), num(2.0)]), chr("éüéü"));
}
