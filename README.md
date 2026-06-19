# sasrs

A SAS 9.4 language interpreter built in Rust on top of [Polars](https://pola.rs/).

`sasrs` reads a classic SAS program (DATA steps, PROC steps, the macro language)
and executes it in batch, writing a SAS-style **log** and **listing**. Datasets
are backed by Parquet tables via Polars.

> Status: early development. The interpreter covers a large subset of SAS 9.4
> (DATA step, PROC SQL, the macro processor, many base/stat procedures, and ODS
> HTML/RTF/PDF/Excel output). Statistical modelling procedures are still in
> progress — see `PROGRESS.md` and `PLAN.md` for the milestone roadmap.

## Installation

```sh
cargo install --path .
```

This installs the `sasrs` binary.

## Usage

```sh
sasrs program.sas
```

By default the log is written to stderr and the listing to stdout, mirroring a
SAS batch run.

| Option            | Description                                                        |
| ----------------- | ------------------------------------------------------------------ |
| `--log <FILE>`    | Write the log to a file instead of stderr.                         |
| `--print <FILE>`  | Write the listing to a file instead of stdout.                     |
| `--work <DIR>`    | WORK library directory (default: a temporary dir, dropped on exit).|
| `--deterministic` | Deterministic output (frozen timestamps) — used by snapshot tests. |
| `--vectorize`     | Enable the optional vectorized fast path for simple DATA steps.    |

Example:

```sh
sasrs analysis.sas --log analysis.log --print analysis.lst
```

## Library API

`sasrs` is also usable as a library:

```rust
use sasrs::{run, RunOptions};

let outcome = run(source, RunOptions::default());
```

## Optional features

- `s3` — enables an S3 storage backend for libraries
  (`libname x 's3://bucket/prefix';`), pulling in the Polars `cloud` + `aws`
  features. Off by default; the default build is unaffected.

```sh
cargo build --features s3
```

## License

Licensed under either of

- MIT license ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option.
