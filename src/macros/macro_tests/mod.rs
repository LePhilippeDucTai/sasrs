use super::*;

fn run(input: &str) -> String {
    MacroStage::default().process(input)
}

/// Comme `run` mais passe par `expand_open_code` (applique la passe finale
/// d'unmask des sentinelles `%str`/`%nrstr`). Engine déterministe pour les
/// variables automatiques figées.
fn expand(input: &str) -> String {
    MacroEngine::new(true).expand_open_code(input)
}

fn segments(src: &str) -> Vec<String> {
    let mut seg = RawSegmenter::new(src);
    let mut out = Vec::new();
    while let Some((s, e)) = seg.next_segment() {
        out.push(src[s..e].to_string());
    }
    out
}

// --- M11.4 : %eval (évaluateur d'expression entière) ---

fn eval(expr: &str) -> Result<i64, MacroError> {
    MacroStage::default().macro_eval(expr)
}

// --- M19.2 : %include + bibliothèques autocall (SASAUTOS) ---

use std::io::Write;

/// Crée un engine déterministe dont la base d'inclusion est `dir`.
fn engine_in(dir: &std::path::Path) -> MacroEngine {
    let mut e = MacroEngine::new(true);
    e.set_include_base_dir(dir.to_path_buf());
    e
}

/// Écrit `content` dans `dir/name` et rend le chemin.
fn write_file(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    path
}

mod segmenter;
mod sysfunc;
mod include;
mod window;
