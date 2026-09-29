<div align="center">

# sasrs

**A SAS 9.4-style language interpreter written in Rust, powered by Polars.**

Run classic SAS batch programs with DATA steps, PROC steps, macros, SQL and ODS — while storing datasets as open Parquet files and exposing the engine through a Rust API.

[![CI](https://github.com/LePhilippeDucTai/sasrs/actions/workflows/ci.yml/badge.svg)](https://github.com/LePhilippeDucTai/sasrs/actions/workflows/ci.yml)
![Rust 2024](https://img.shields.io/badge/Rust-2024-000000?logo=rust)
![Polars](https://img.shields.io/badge/engine-Polars-0075FF)
![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)

[Getting started](docs/getting-started.md) · [Examples](examples/) · [Conformance status](conformance/STATUS.md) · [Support contract](docs/support-contract.md) · [Contributing](CONTRIBUTING.md)

</div>

> [!IMPORTANT]
> **sasrs is a work in progress, not yet a drop-in replacement for SAS 9.4.**
> The project intentionally distinguishes implemented behavior, externally validated behavior, documented approximations and unsupported features. If an unsupported behavior could change a result, sasrs is designed to report it explicitly instead of silently falling back.

## Why sasrs?

sasrs is aimed at teams and developers who want to execute familiar SAS-style programs while keeping the runtime, storage and integration layer open.

| | What you get |
| --- | --- |
| 🦀 **Native Rust engine** | A standalone interpreter with no proprietary SAS runtime dependency. |
| ⚡ **Polars execution** | Columnar execution and Parquet-backed datasets. |
| 🧾 **Familiar SAS workflow** | DATA steps, PROC steps, macro language, PROC SQL, formats, ODS and SAS-style logs/listings. |
| 📦 **Open storage** | WORK and assigned libraries use Parquet plus a small SAS metadata sidecar. |
| 🔎 **Explicit diagnostics** | NOTE / WARNING / ERROR diagnostics and meaningful process exit codes. |
| 🧩 **Embeddable API** | Use the same execution engine directly from Rust through `sasrs::api`. |
| 🧪 **Reproducible execution** | Deterministic mode, snapshot tests, differential tests and a conformance corpus. |

---

## 5-minute quick start

### 1. Install

#### From a release binary

Tagged releases publish precompiled binaries for:

- Linux x86_64
- Windows x86_64
- macOS Apple Silicon

See [GitHub Releases](https://github.com/LePhilippeDucTai/sasrs/releases) and verify the matching `SHA256SUMS` asset before execution.

#### From source

```sh
git clone https://github.com/LePhilippeDucTai/sasrs.git
cd sasrs
cargo install --path .
```

This installs the `sasrs` executable into `~/.cargo/bin`.

### 2. Write a first SAS program

Create `hello.sas`:

```sas
data work.people;
  length name $16;
  input name $ age;
  datalines;
Alice 32
Bob 45
Carol 28
;
run;

proc print data=work.people;
run;
```

### 3. Run it

```sh
sasrs hello.sas
```

By default:

- the **SAS-style log** is written to stderr;
- the **listing** is written to stdout;
- WORK lives in a temporary directory and is removed when the process exits.

The listing contains the three observations:

```text
Alice    32
Bob      45
Carol    28
```

A clean run exits with code `0`.

To persist the log, listing and WORK library:

```sh
sasrs hello.sas \
  --log hello.log \
  --print hello.lst \
  --work ./work
```

The persisted WORK directory then contains files such as:

```text
people.parquet
people.parquet.sasmeta.json
```

---

## Common workflows

### 1. Import CSV → aggregate with PROC SQL → export

The repository contains a tested end-to-end example:

```sh
mkdir -p examples/out
sasrs examples/cli/analysis.sas
```

The program imports `examples/data/patients.csv`, groups patients by sex with PROC SQL, exports the result and prints it.

The input is:

```csv
Name,Sex,Age,Weight
Alice,F,32,55.5
Bob,M,45,80.0
Carol,F,28,61.2
Dan,M,51,92.4
Edith,F,39,66.7
```

Core SAS code:

```sas
proc import datafile='../data/patients.csv'
  out=work.patients
  dbms=csv
  replace;
  getnames=yes;
run;

proc sql;
  create table work.summary as
  select Sex,
         count(*) as n_patients,
         mean(Age) as mean_age
  from work.patients
  group by Sex;
quit;

proc export data=work.summary
  outfile='../out/summary.csv'
  dbms=csv
  replace;
run;
```

Expected `summary.csv`:

```csv
Sex,N_PATIENTS,MEAN_AGE
F,3,33
M,2,48
```

This exact logical result is exercised by `tests/examples.rs`.

### 2. Use classic DATA-step transformations

```sas
data work.loans;
  input id balance rate;
  annual_interest = balance * rate;
  if balance >= 100000 then segment = 2;
  else segment = 1;
  datalines;
1 80000  0.035
2 125000 0.041
3 50000  0.029
;
run;

proc print data=work.loans;
  var id balance rate annual_interest segment;
run;
```

Expected calculated values:

```text
id   balance   rate    annual_interest   segment
1     80000    0.035        2800             1
2    125000    0.041        5125             2
3     50000    0.029        1450             1
```

### 3. Embed sasrs in a Rust application

The public `sasrs::api` facade keeps a live session across submissions and lets the host read produced datasets as Polars `DataFrame` values.

```rust
use sasrs::api::{Options, Session};

fn main() {
    let mut session =
        Session::new(Options::default()).expect("session init");

    let submission = session.submit(
        "data work.answer; value = 42; run;"
    );

    assert_eq!(submission.exit_code, 0);

    let (df, vars) = session
        .dataset("work", "answer")
        .expect("WORK.ANSWER");

    println!("{} row(s), {} column(s)", df.height(), vars.len());

    let report = session.close();
    println!("exit={}", report.exit_code);
}
```

Expected output:

```text
1 row(s), 1 column(s)
exit=0
```

For a complete executable walkthrough:

```sh
cargo run --locked --example quickstart
```

The example imports the patients CSV, creates `WORK.SUMMARY`, reads it back through the API, inspects structured diagnostics and finishes with:

```text
OK
```

See [`examples/quickstart.rs`](examples/quickstart.rs).

---

## What is supported?

sasrs already covers a broad subset of classic SAS batch workloads.

| Area | Highlights |
| --- | --- |
| **DATA step** | Dataset creation, `SET`, `MERGE`, `BY`, `WHERE`, `IF/THEN/ELSE`, iterative `DO`, arrays, hash objects, `RETAIN`, formats/informats, external input/output and common DATA-step functions. |
| **PROC SQL** | `SELECT`, joins, grouping, aggregates, subqueries, set operators, `CREATE TABLE AS`, views and common DDL/DML. |
| **Base procedures** | `PRINT`, `SORT`, `CONTENTS`, `IMPORT`, `EXPORT`, `TRANSPOSE`, `APPEND`, `COMPARE`, `RANK`, `DATASETS`, `REPORT`, `TABULATE` and others. |
| **Statistical procedures** | `MEANS/SUMMARY`, `FREQ`, `UNIVARIATE`, `TTEST`, `NPAR1WAY`, `CORR`, `REG` and additional implemented procedures. |
| **Macro language** | `%MACRO`, parameters, `%LET`, macro variables, `%IF`, `%DO`, quoting functions, `%SYSFUNC`, `%INCLUDE` and tracing. |
| **ODS / output** | LISTING, HTML, RTF, PDF, Excel, selected `ODS OUTPUT` objects and optional graphics rendering. |
| **Formats** | Built-in numeric/date/character formats, informats and user-defined `PROC FORMAT` catalogs. |
| **Libraries** | Local Parquet-backed libraries and optional S3 library support. |

The detailed matrix belongs in the generated and test-backed documentation rather than on the landing page:

- [Conformance status](conformance/STATUS.md)
- [Getting started](docs/getting-started.md)
- [Support contract](docs/support-contract.md)
- [Contributing / coverage-state definitions](CONTRIBUTING.md)

Selected compatibility claims are backed by validated cases `compat/transpose/*`, see [`conformance/STATUS.md`](conformance/STATUS.md). The conformance report is generated from the corpus and records both validated behavior and known divergences.

---

## CLI reference

```text
sasrs <PROGRAM.sas> [OPTIONS]
```

| Option | Description |
| --- | --- |
| `--log <FILE>` | Write the SAS log to a file instead of stderr. |
| `--print <FILE>` | Write the listing to a file instead of stdout. |
| `--work <DIR>` | Persist WORK in the given directory. |
| `--deterministic` | Freeze non-deterministic output for reproducible tests. |
| `--vectorize` | Enable the optional vectorized fast path for simple DATA steps. |

### Exit codes

| Code | Meaning |
| :---: | --- |
| `0` | Completed without WARNING or ERROR. |
| `1` | Completed with at least one WARNING. |
| `2` | At least one ERROR occurred, or an output stream/file could not be written. |

The full diagnostic policy is documented in [`docs/support-contract.md`](docs/support-contract.md).

---

## Rust library API

The CLI and library facade use the same execution engine.

A `Session` can:

- submit multiple SAS programs while preserving session state;
- keep WORK, librefs, macro state, formats and SQL views alive between submissions;
- return the log and listing for each submission;
- expose structured NOTE / WARNING / ERROR diagnostics;
- register a host Polars `DataFrame` as a SAS dataset;
- read a SAS dataset back as `DataFrame + Vec<VarMeta>`;
- report ODS files produced by the session;
- clean up temporary WORK on close.

Primary types live under:

```rust
sasrs::api::{
    ApiError,
    CloseReport,
    Diagnostic,
    Options,
    ProducedFile,
    ProducedFileKind,
    Session,
    Severity,
    Submission,
    VarMeta,
    VarType,
}
```

See [`src/api.rs`](src/api.rs) and [`examples/quickstart.rs`](examples/quickstart.rs).

---

## Python launcher

The `python/` subdirectory contains a thin Python launcher for environments where installing Rust is undesirable.

On Windows x86_64 it can download the matching precompiled sasrs release, verify its SHA-256 and execute it:

```sh
uvx --from "git+https://github.com/LePhilippeDucTai/sasrs#subdirectory=python" sasrs program.sas
```

The Python package is a launcher around the native executable; it is not a separate Python implementation of the interpreter.

See [`python/README.md`](python/README.md).

---

## Optional features

### ODS graphics

Enable PNG/SVG rendering for supported graphical procedures:

```sh
cargo build --features graphics
```

### S3 libraries

Enable the S3 storage backend:

```sh
cargo build --features s3
```

Then SAS programs can assign an S3-backed library:

```sas
libname lake 's3://bucket/prefix';
```

Both features are off by default.

---

## Storage and recovery

sasrs datasets are stored as two files:

```text
<table>.parquet
<table>.parquet.sasmeta.json
```

The Parquet file contains the data. The sidecar contains SAS-specific metadata such as formats, labels and declared character lengths.

Writes use an atomic publication protocol: data is published first through a temporary file and atomic rename, then metadata is published the same way. A stale or inconsistent sidecar is ignored with a diagnostic rather than silently applied to different data.

This makes the underlying data directly accessible from the wider Arrow/Parquet ecosystem while keeping SAS metadata available to sasrs.

Design details:

- [ADR 0001 — Parquet + sidecar storage](docs/adr/0001-stockage-parquet-sidecar.md)
- [Encoding contract](docs/encoding.md)

---

## Reliability model

The project treats compatibility as an evidence problem, not as a checkbox.

The repository contains:

- ordinary Rust unit/integration tests;
- snapshot tests;
- property tests;
- differential tests;
- example-level end-to-end tests;
- storage fault-injection tests;
- a dedicated SAS compatibility/conformance corpus.

The guiding rule is simple:

> **Anything that could change a result should not become a silent fallback.**

Unsupported result-affecting behavior is expected to surface as an ERROR. Display-only limitations may surface as a WARNING or NOTE according to the [support contract](docs/support-contract.md).

---

## Documentation map

| Document | Purpose |
| --- | --- |
| [`docs/getting-started.md`](docs/getting-started.md) | Installation and first execution from a blank environment. |
| [`examples/`](examples/) | Tested CLI and Rust API examples. |
| [`conformance/STATUS.md`](conformance/STATUS.md) | Generated inventory of validated behavior and known divergences. |
| [`docs/support-contract.md`](docs/support-contract.md) | Diagnostic and unsupported-feature policy. |
| [`docs/encoding.md`](docs/encoding.md) | Character and file encoding contract. |
| [`docs/release.md`](docs/release.md) | Release artifacts and release process. |
| [`docs/adr/`](docs/adr/) | Architecture decisions. |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | Development, validation and contribution rules. |

---

## Project status

sasrs is under active development.

The priority is not to claim every corner of SAS 9.4 syntax as supported. The priority is to make supported workloads **predictable, inspectable and testable**, and to record known divergences explicitly.

For production evaluation, start with the [live conformance report](conformance/STATUS.md) and the [support contract](docs/support-contract.md), then test your own representative SAS workload.

---

## Contributing

Contributions are welcome, particularly when they include:

1. a minimal SAS program demonstrating the behavior;
2. a reference or independent oracle for the expected result;
3. a regression/conformance test;
4. the implementation and documentation update.

See [`CONTRIBUTING.md`](CONTRIBUTING.md).

---

## License

Licensed under either of:

- [MIT](LICENSE-MIT)
- [Apache License 2.0](LICENSE-APACHE)

at your option.

---

<div align="center">

**SAS syntax in. Open data out. Rust underneath.**

</div>
