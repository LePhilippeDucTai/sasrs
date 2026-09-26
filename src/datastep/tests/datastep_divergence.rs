//! J03-P6 — tests de non-régression des divergences de l'étape DATA, pilotes
//! par les fixtures `tests/fixtures/j03/datastep_*.sas` (mêmes programmes
//! que les snapshots `tests/snapshots/snapshot__fixtures@j03__*.snap`, mais
//! avec assertions EXPLICITES des valeurs attendues).
//!
//! CHAQUE valeur attendue cite sa source dans la documentation SAS
//! (SAS Language Reference by Example, ch. 21 « Examples: Update Data » —
//! https://go.documentation.sas.com/api/collections/pgmsascdc/9.4_3.5/docsets/lepg/content/lepg.pdf ;
//! SAS Functions Reference — lefunctionsref).

/// Exécute une fixture j03 en mode déterministe et renvoie
/// (log, listing). Les lignes du listing sont NORMALISÉES (espaces
/// multiples → un) pour rendre les assertions robustes à la mise en page.
fn run_j03_fixture(name: &str) -> (String, Vec<String>) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/j03")
        .join(name);
    let source = std::fs::read_to_string(&path).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let outcome = crate::run(
        &source,
        crate::RunOptions {
            work_dir: None,
            base_dir: Some(tmp.path().to_path_buf()),
            deterministic: true,
            vectorize: false,
        },
    );
    assert_eq!(outcome.exit_code, 0, "la fixture {name} doit sortir 0");
    let listing = outcome
        .listing
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>();
    (outcome.log, listing)
}

/// Vrai si le listing contient une ligne (normalisée) commençant par `row`.
fn has_row(listing: &[String], row: &str) -> bool {
    let row = row.split_whitespace().collect::<Vec<_>>().join(" ");
    listing.contains(&row)
}

// ── UPDATE : exemple documenté LEPG 21.31 (défaut MISSINGCHECK) ──────────

#[test]
fn datastep_divergence_update_doc_2131() {
    // Exemple « Update a Data Set with Missing and Different Values for the
    // BY Variables » (Output 21.31 + notes) :
    // - mineral (nouvelle variable de transaction) ajoutée, manquante
    //   pour les obs sans valeur ;
    // - les obs b et g de la transaction (sans maître) sont AJOUTÉES :
    //   « Values in observations 2 and 6 in the transaction data set … are
    //   added to the master data set as new observations. » (animal
    //   manquant) ;
    // - « The value for plant in observation 4 [master e] is not changed to
    //   missing even though it is missing in the transaction data set. » ;
    // - « Three observations … have updated values for the variable plant »
    //   (a, c, f).
    let (_log, listing) = run_j03_fixture("datastep_update.sas");
    // Ordre interclassé par clé (a..g) comme l'exemple trié par BY.
    assert!(has_row(&listing, "1 a Ant Apricot Amethyst"), "{listing:?}");
    assert!(has_row(&listing, "2 b Barley Beryl"), "{listing:?}");
    assert!(has_row(&listing, "3 c Cat Cactus"), "{listing:?}");
    assert!(has_row(&listing, "4 d Dog Dewberry"), "{listing:?}");
    assert!(has_row(&listing, "5 e Eagle Eggplant"), "{listing:?}");
    assert!(has_row(&listing, "6 f Frog Fennel"), "{listing:?}");
    assert!(has_row(&listing, "7 g Grape Garnet"), "{listing:?}");
}

// ── UPDATE UPDATEMODE=NOMISSINGCHECK : LEPG 21.32 ────────────────────────

#[test]
fn datastep_divergence_update_nomissingcheck_doc_2132() {
    // « the value of plant in observation 5 is set to missing because it is
    // missing in the transaction data set and the UPDATEMODE=NOMISSINGCHECK
    // option is in effect. » — seule différence attendue vs 21.31 : l'obs e
    // (5ᵉ) a plant manquant. Forme PARENTHÉSÉE de l'option.
    let (_log, listing) = run_j03_fixture("datastep_update_nomissingcheck.sas");
    assert!(has_row(&listing, "1 a Ant Apricot Amethyst"), "{listing:?}");
    assert!(has_row(&listing, "2 b Barley Beryl"), "{listing:?}");
    assert!(has_row(&listing, "5 e Eagle"), "{listing:?}");
    assert!(has_row(&listing, "7 g Grape Garnet"), "{listing:?}");
}

// ── UPDATE : doublons de clé, LEPG 21.30 ─────────────────────────────────

#[test]
fn datastep_divergence_update_duplicate_keys_doc_2130() {
    // « The value Dewberry in the master data set is replaced by Dill,
    // which is the last value for plant in the transaction data set. » :
    // TOUTES les transactions d'une même clé sont appliquées dans l'ordre
    // (d → Date puis Dill). Forme NUE de UPDATEMODE= (valeur par défaut
    // explicite MISSINGCHECK) pour couvrir le parsing en fin de statement.
    let (_log, listing) = run_j03_fixture("datastep_update_dupes.sas");
    assert!(has_row(&listing, "1 a Ant Apricot"), "{listing:?}");
    assert!(has_row(&listing, "4 d Dog Dill"), "{listing:?}");
    assert!(has_row(&listing, "5 e Eagle Escarole"), "{listing:?}");
    // Pas d'obs ajoutée : toutes les clés de transaction existent au maître.
    assert_eq!(listing.iter().filter(|l| l.starts_with('7')).count(), 0);
}

// ── FORMAT/ATTRIB créent la variable inconnue (comme en SAS) ─────────────

#[test]
fn datastep_divergence_format_attrib_create_var() {
    // Doc SAS (FORMAT statement) : une variable inconnue référencée par
    // FORMAT est créée (numérique par défaut) et apparaît dans le dataset
    // de sortie ; ATTRIB (label=/format=) fait de même.
    let (log, listing) = run_j03_fixture("datastep_format_newvar.sas");
    // Les deux variables créées existent et portent leurs valeurs.
    assert!(has_row(&listing, "1 x 1.50 2"), "{listing:?}");
    // Le libellé ATTRIB est bien attaché (proc print label).
    assert!(
        listing.iter().any(|l| l.contains("Etiquette J03")),
        "label ATTRIB attendu : {listing:?}"
    );
    // Le dataset contient bien 3 variables (base, newnum, newlbl).
    assert!(
        log.contains("WORK.FMTNEW has 1 observations and 3 variables"),
        "{log}"
    );
}

// ── Fonctions caractères (doc lefunctionsref) ────────────────────────────

#[test]
fn datastep_divergence_char_fns_fixture() {
    let (_log, listing) = run_j03_fixture("datastep_char_fns.sas");
    // FIND — doc : find('abc','a') = 1 ; find(xyz,'she',22) = 27 ;
    // 'o' en position 5 de 'hello world' (position de départ INCLUSE).
    // COMPBL — '125 E  Main St' → '125 E Main St'.
    // REPEAT — plafond 32 767 CARACTÈRES pour 'é' (65 534 octets) ;
    // l'ancien plafond octet donnait 16 383.
    // La ligne unique du listing porte les valeurs normalisées (les blancs
    // de bord de COMPBL et les tabulations ne sont pas distinguables en
    // listing : ils sont vérifiés EXACTEMENT dans les tests unitaires
    // `functions::tests::datastep_divergence`).
    let row = listing
        .iter()
        .find(|l| l.starts_with("1 She sells"))
        .expect("ligne de valeurs");
    let cols: Vec<&str> = row.split_whitespace().collect();
    // Valeurs numériques (tokens fiables après normalisation) :
    // find('abc','a')=1, find(s,'she',22)=27, find('hello world','o',5)=5,
    // length(repeat('é',1e9))=32767 (plafond en caractères).
    for expected in ["1", "27", "5", "32767"] {
        assert!(
            cols.contains(&expected),
            "{expected} attendu dans {cols:?}"
        );
    }
}
