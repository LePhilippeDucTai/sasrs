use super::*;

/// Une matrice est « vraie » si elle est 1×1 et non nulle (sémantique SAS IML
/// des conditions IF/WHILE : la condition doit être un scalaire).
pub(super) fn matrix_truthy(m: &Matrix) -> bool {
    if m.len() == 1 && m[0].len() == 1 {
        m[0][0] != 0.0 && !m[0][0].is_nan()
    } else {
        // Toute la matrice doit être non nulle (sémantique IML : ALL).
        !m.is_empty() && m.iter().all(|r| r.iter().all(|v| *v != 0.0))
    }
}

/// Formate une valeur numérique pour le listing IML (logique BEST. : entiers
/// sans décimale, flottants tronqués à 4 décimales, trailing zeros enlevés).
pub(super) fn fmt_val(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let s = format!("{v:.4}");
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(super) fn render_matrix(name: &str, m: &Matrix, session: &mut Session) {
    let (nr, nc) = dims(m);
    session.listing.write_line(name);
    session.listing.blank();

    // Cellules formatées.
    let cells: Vec<Vec<String>> = m
        .iter()
        .map(|r| r.iter().map(|v| fmt_val(*v)).collect())
        .collect();

    let show_col_hdr = nc >= 2;
    let show_row_hdr = nr >= 2;

    // Largeur de chaque colonne : max(4, valeurs formatées, en-tête COLk).
    let mut widths = vec![4usize; nc];
    for (j, w) in widths.iter_mut().enumerate() {
        if show_col_hdr {
            *w = (*w).max(format!("COL{}", j + 1).len());
        }
        for row in &cells {
            *w = (*w).max(row[j].len());
        }
    }

    // Largeur de l'étiquette de ligne.
    let row_label_w = if show_row_hdr {
        (0..nr)
            .map(|i| format!("ROW{}", i + 1).len())
            .max()
            .unwrap_or(0)
    } else {
        0
    };

    let gap = 2usize;

    // En-tête de colonnes.
    if show_col_hdr {
        let mut line = String::new();
        if show_row_hdr {
            line.push_str(&" ".repeat(row_label_w));
        }
        for (j, w) in widths.iter().enumerate() {
            line.push_str(&" ".repeat(gap));
            let hdr = format!("COL{}", j + 1);
            line.push_str(&format!("{hdr:>w$}", w = *w));
        }
        session.listing.write_line(&line);
        session.listing.blank();
    }

    // Lignes.
    for (i, row) in cells.iter().enumerate() {
        let mut line = String::new();
        if show_row_hdr {
            let lbl = format!("ROW{}", i + 1);
            line.push_str(&format!("{lbl:<w$}", w = row_label_w));
        }
        for (j, w) in widths.iter().enumerate() {
            line.push_str(&" ".repeat(gap));
            line.push_str(&format!("{:>w$}", row[j], w = *w));
        }
        session.listing.write_line(&line);
    }
    session.listing.blank();
}
