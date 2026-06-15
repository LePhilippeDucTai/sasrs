//! Génération programmatique des parquets d'entrée des fixtures
//! (aucun binaire en git). Chaque test crée un répertoire temporaire :
//!   <tmp>/data/class.parquet   ← clone de sashelp.class
//! puis exécute la fixture avec base_dir = <tmp> ; les scripts utilisent
//! `libname d 'data';` (chemin relatif, donc snapshots stables).

use polars::prelude::*;
use std::fs::File;
use std::path::Path;

/// Clone de sashelp.class (name, sex, age, height, weight) — le dataset
/// jouet le plus reconnaissable de l'écosystème SAS.
pub fn write_class_parquet(dir: &Path) {
    let mut df = df!(
        "name" => ["Alfred", "Alice", "Barbara", "Carol", "Henry", "James",
                    "Jane", "Janet", "Jeffrey", "John", "Joyce", "Judy",
                    "Louise", "Mary", "Philip", "Robert", "Ronald", "Thomas",
                    "William"],
        "sex" => ["M", "F", "F", "F", "M", "M", "F", "F", "M", "M", "F",
                   "F", "F", "F", "M", "M", "M", "M", "M"],
        "age" => [14i64, 13, 13, 14, 14, 12, 12, 15, 13, 12, 11, 14, 12,
                   15, 16, 12, 15, 11, 15],
        "height" => [69.0f64, 56.5, 65.3, 62.8, 63.5, 57.3, 59.8, 62.5,
                      62.5, 59.0, 51.3, 64.3, 56.3, 66.5, 72.0, 64.8, 67.0,
                      57.5, 66.5],
        "weight" => [112.5f64, 84.0, 98.0, 102.5, 102.5, 83.0, 84.5, 112.5,
                      84.0, 99.5, 50.5, 90.0, 77.0, 112.0, 150.0, 128.0,
                      133.0, 85.0, 112.0],
    )
    .unwrap();
    std::fs::create_dir_all(dir).unwrap();
    let mut file = File::create(dir.join("class.parquet")).unwrap();
    ParquetWriter::new(&mut file).finish(&mut df).unwrap();
}

/// Écrit `<dir>/pets.csv` à côté de `class.parquet` ; ignoré par les
/// fixtures qui ne le référencent pas (chemin relatif → snapshots stables).
pub fn write_pets_csv(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("pets.csv"),
        "name,species,age,weight\n\
         Rex,Dog,5,30.5\n\
         Felix,Cat,3,4.2\n\
         Tweety,Bird,1,0.1\n",
    )
    .unwrap();
}
