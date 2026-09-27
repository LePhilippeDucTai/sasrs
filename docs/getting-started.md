# Premiers pas avec sasrs

Ce guide part d'un environnement vierge et vous amène jusqu'à la lecture du
résultat d'un programme SAS exécuté par `sasrs` : installation, premier
programme, récupération d'une table, lecture d'un diagnostic et codes retour.

`sasrs` est un interpréteur batch : il lit un programme SAS 9.4 classique
(étapes DATA, PROC, macro-langage), l'exécute, écrit une **log** (défaut :
stderr) et un **listing** (défaut : stdout), et stocke les tables en Parquet.

---

## 1. Installation

Trois voies réelles, selon ce dont vous disposez :

### 1.1 Binaire de release (aucune dépendance)

Les [GitHub Releases](https://github.com/LePhilippeDucTai/sasrs/releases)
publient des binaires précompilés par tag `v<version>` :

| Asset | Plateforme |
|---|---|
| `sasrs-linux-x86_64.tar.gz` | Linux x86_64 |
| `sasrs-windows-x86_64.exe` | Windows x86_64 (binaire nu) |
| `sasrs-macos-arm64.tar.gz` | macOS Apple Silicon |

L'asset `SHA256SUMS` de la même release contient les empreintes ; vérifiez
avant d'exécuter :

```sh
sha256sum -c SHA256SUMS          # après avoir téléchargé le binaire voulu
tar xzf sasrs-linux-x86_64.tar.gz
./sasrs --version                # ex. : sasrs 0.1.0 (commit <sha>, features: …)
```

Les assets d'une release sont immuables (jamais remplacés) ; une version
défectueuse est corrigée par un nouveau tag — voir
[`docs/release.md`](release.md).

### 1.2 Depuis les sources : `cargo install`

Nécessite une toolchain Rust stable (le fichier `rust-toolchain.toml` du dépôt
épingle la version utilisée en développement) :

```sh
git clone https://github.com/LePhilippeDucTai/sasrs.git
cd sasrs
cargo install --path .           # installe le binaire `sasrs` dans ~/.cargo/bin
```

Deux features optionnelles existent (OFF par défaut, le build standard n'en
dépend pas) :

```sh
cargo install --path . --features graphics   # images ODS GRAPHICS (PNG/SVG)
cargo install --path . --features s3         # libname x 's3://bucket/prefix';
```

### 1.3 Via Python (sans Rust, Windows x86_64)

Si la machine cible n'a pas Rust mais a Python et un accès réseau à
`github.com`, le sous-dossier [`python/`](../python/) du dépôt fournit un
wrapper (bibliothèque standard uniquement) qui télécharge au premier
lancement le binaire Windows `sasrs-windows-x86_64.exe` depuis la release,
vérifie son SHA-256, le met en cache puis l'exécute :

```sh
uvx --from "git+https://github.com/LePhilippeDucTai/sasrs#subdirectory=python" sasrs program.sas
# ou, dans un projet :
uv add "git+https://github.com/LePhilippeDucTai/sasrs#subdirectory=python"
```

Toute erreur (réseau absent, empreinte invalide, plateforme non supportée)
produit un message clair sur stderr et un code retour 1 — jamais de
traceback. Seul Windows x86_64 a un binaire publié ; les autres plateformes
doivent passer par §1.1 ou §1.2.

---

## 2. Premier programme

Créez `hello.sas` :

```sas
data work.hello;
  length name $16;
  input name $ age;
  datalines;
Alice 32
Bob 45
Carol 28
;
run;

proc print data=work.hello;
run;
```

Exécutez-le :

```sh
sasrs hello.sas
```

La log (numérotation du source + messages `NOTE`/`WARNING`/`ERROR`) va sur
stderr, le listing sur stdout, comme une exécution batch SAS. Pour les
écrire dans des fichiers :

```sh
sasrs hello.sas --log hello.log --print hello.lst
```

Options principales (voir `sasrs --help` pour la liste complète) :

| Option | Rôle |
|---|---|
| `--log <FILE>` | log dans un fichier au lieu de stderr |
| `--print <FILE>` | listing dans un fichier au lieu de stdout |
| `--work <DIR>` | répertoire de la bibliothèque WORK (défaut : répertoire temporaire **jeté en fin de session**) |
| `--deterministic` | sortie déterministe (temps figés) — utilisé par les tests |
| `--vectorize` | fast-path vectorisé optionnel des étapes DATA simples |

Un exemple complet d'analyse (import CSV, agrégation SQL, export, print) est
fourni dans [`examples/cli/analysis.sas`](../examples/cli/analysis.sas) avec
son jeu de données [`examples/data/patients.csv`](../examples/data/patients.csv) :

```sh
mkdir -p examples/out     # l'exemple écrit ../out/summary.csv sous examples/cli/
sasrs examples/cli/analysis.sas
```

---

## 3. Récupérer une table

### 3.1 CLI : `--work` + Parquet

Sans `--work`, WORK vit dans un répertoire temporaire détruit à la fin de
l'exécution : donnez un répertoire pour conserver les tables :

```sh
sasrs hello.sas --work ./workdir
ls workdir
# hello.parquet
# hello.parquet.sasmeta.json
```

Chaque table est un fichier Parquet (`<table>.parquet`) accompagné d'un
sidecar de métadonnés SAS (`<table>.parquet.sasmeta.json` : format, label,
longueur caractère déclarée). Le Parquet se relit avec n'importe quel lecteur
(Polars, DuckDB, pandas/pyarrow…) ; le sidecar est géré par `sasrs` (il est
ignoré avec un `WARNING` si son empreinte ne correspond plus au fichier de
données — voir le protocole d'écriture atomique dans le
[README](../README.md#storage-and-recovery) et l'[ADR 0001](adr/0001-stockage-parquet-sidecar.md)).

### 3.2 Rust : la façade `sasrs::api`

Le crate est aussi une bibliothèque. La façade [`sasrs::api`](../src/api.rs)
fournit une session qui exécute du code et relit les tables produites :

```rust
use sasrs::api::{Options, Session};

let mut session = Session::new(Options {
    deterministic: true,
    ..Options::default()
})
.expect("la session doit s'initialiser");

let submission = session.submit("\
    data work.hello; input name $ age; datalines; Alice 32; run;");
assert_eq!(submission.exit_code, 0);

// Relecture de la table : données (Polars DataFrame) + métadonnées SAS.
let (df, vars) = session.dataset("work", "hello").unwrap();
println!("{} obs, variables : {:?}", df.height(),
         vars.iter().map(|v| &v.name).collect::<Vec<_>>());

let report = session.close();    // WORK temporaire jeté
assert_eq!(report.exit_code, 0);
```

Un exemple complet, exécutable et testé, est
[`examples/quickstart.rs`](../examples/quickstart.rs) :

```sh
cargo run --locked --example quickstart
```

### 3.3 Python

Le wrapper Python du §1.3 est un lanceur du binaire : il ne réimplémente pas
l'interpréteur et n'expose pas d'API de relecture. Récupérez la table par la
voie Parquet (§3.1 — avec `--work` et `proc export`/`proc copy` vers un
répertoire connu, puis lecture du `.parquet` avec pyarrow/polars côté
Python), ou par la façade Rust (§3.2).

---

## 4. Lire un diagnostic

Les messages de la log suivent la sévérité SAS :

- `NOTE:` — information (table créée, nombre d'observations, option reconnue
  mais sans effet…) ;
- `WARNING:` — limitation display-only ou situation à examiner (option
  reconnue mais différée, non-convergence…) ; le résultat n'est pas modifié
  silencieusement ;
- `ERROR:` — quelque chose a pu changer le résultat (instruction/option non
  supportée qui pourrait modifier les résultats, fichier illisible, échec
  d'écriture) ; l'étape est arrêtée et le traitement continue à l'étape
  suivante, comme SAS.

Exemple — une instruction inconnue :

```log
ERROR: Statement BOGUS is not yet implemented.
NOTE: The SAS System stopped processing this step because of errors.
```

Exemple — routage `PROC PRINTTO` non supporté (display-only) :

```log
WARNING: PROCEDURE PRINTTO: LOG= routing not supported until J07-P5; the log destination is unchanged ('/tmp/x.log').
```

En bibliothèque, chaque message est aussi disponible sous forme structurée :
`Submission::diagnostics` expose un `Vec<Diagnostic>` (sévérité `Note` /
`Warning` / `Error` + message), dans l'ordre d'émission — voir
[`examples/quickstart.rs`](../examples/quickstart.rs).

La règle de sévérité (ERROR pour tout ce qui peut changer un résultat,
WARNING pour le display-only, jamais de repli silencieux) est le contrat de
support décrit dans [`docs/support-contract.md`](support-contract.md).

---

## 5. Codes retour

Le code retour du processus (et `Submission::exit_code` en bibliothèque)
suit le nombre d'erreurs/avertissements de la log :

| Code | Signification |
| :--: | --- |
| `0` | exécution terminée sans `ERROR` ni `WARNING` dans la log. |
| `1` | exécution terminée, mais au moins un `WARNING` (option display-only différée, `PROC PRINTTO`, non-convergence…) ; un `%ABORT RETURN n` demandé est ramené à 1 si des erreurs ont été comptées. |
| `2` | au moins un `ERROR` : programme illisible, étape rejetée, échec d'écriture de sortie (`--log`/`--print`/stdout/stderr). |

Vérifiez toujours la log : le code retour résume la sévérité maximale, la log
porte le détail.

---

## Pour aller plus loin

- [`README.md`](../README.md) — couverture détaillée (PROC, DATA step, SQL,
  macro, ODS), stockage et récupération, licence.
- [`examples/`](../examples/) — exemples autonomes testés (CLI + Rust).
- [`docs/encoding.md`](encoding.md) — contrat d'encodage (D-001).
- [`docs/support-contract.md`](support-contract.md) — contrat de sévérité
  des diagnostics.
- [`CONTRIBUTING.md`](../CONTRIBUTING.md) — contribuer (reproducer + test de
  non-régression).
