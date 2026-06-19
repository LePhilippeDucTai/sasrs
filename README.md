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

## Feature coverage

The tables below summarise what the interpreter supports today, down to the
individual options of each procedure and DATA step statement. Legend:

- ✅ — covered
- 🟡 — partial (a documented subset of options, or simplifications vs SAS 9.4)
- 🔴 — recognised by the dispatcher but **not implemented** (skeleton / `todo!()`)

### Procedures (PROC)

| PROC | State | Covered statements & options | Not covered / deferred |
| --- | :---: | --- | --- |
| `PRINT` | ✅ | `DATA=`, `NOOBS`, `LABEL`; `VAR` | `WHERE`, `BY`, `ID`, `SUM`, style options |
| `SORT` | ✅ | `DATA=`, `OUT=`, `NODUPKEY`, `NODUPRECS`/`NODUP`; `BY [DESCENDING]` | `TAGSORT`, `SORTSEQ=`, `KEY=` |
| `CONTENTS` | ✅ | `DATA=`, `VARNUM`, `DATA=lib._ALL_` | `DETAILS`, `OUT=`, ODS output object |
| `MEANS` / `SUMMARY` | ✅ | `DATA=`, `NOPRINT`, stat keywords (`N NMISS MEAN STD MIN MAX SUM RANGE STDERR CV MEDIAN CLM LCLM UCLM`); `CLASS`, `VAR`, `BY`, `WEIGHT`, `OUTPUT OUT= stat(var)=name` | `WAYS`, `TYPES`, `PRINTALLTYPES`, percentile keywords |
| `TRANSPOSE` | ✅ | `DATA=`, `OUT=`, `PREFIX=`, `NAME=`; `BY`, `ID`, `VAR` | `IDLABEL`, `COPY`, `LET`, `SUFFIX=` |
| `APPEND` | ✅ | `BASE=`, `DATA=`, `FORCE` | `APPENDVER=` |
| `RANK` | ✅ | `DATA=`, `OUT=`, `DESCENDING`, `TIES=(MEAN\|LOW\|HIGH\|DENSE)`, `GROUPS=`, methods `FRACTION`/`NPLUS1`/`PERCENT`/`NORMAL=(BLOM\|TUKEY\|VW)`/`SAVAGE`; `VAR`, `RANKS`, `BY` | — |
| `CORR` | ✅ | `DATA=`, `NOSIMPLE`, `NOPROB`, `NOCORR`, `PEARSON`, `SPEARMAN`, `KENDALL`, `OUT=`/`OUTP=`/`OUTS=`/`OUTK=`; `VAR`, `WITH`, `WEIGHT` (Pearson only) | `HOEFFDING`, partial correlation, weighted Spearman/Kendall |
| `COMPARE` | ✅ | `BASE=`, `COMPARE=`, `OUT=`, `NOVALUES`, `BRIEFSUMMARY` | `CRITERION=`, `ID`, `VAR`/`WITH` |
| `IMPORT` | ✅ | `DATAFILE=`/`FILENAME=`, `OUT=`, `DBMS=(CSV\|TAB\|DLM)`, `REPLACE`; `GETNAMES=`, `DELIMITER=`/`DLM=` | `DBMS=XLSX`/`EXCEL`, `GUESSINGROWS=` (parsed, ignored) |
| `EXPORT` | ✅ | `DATA=`, `OUTFILE=`, `DBMS=(CSV\|TAB\|DLM)`, `REPLACE`; `DELIMITER=`/`DLM=` | `DBMS=XLSX`/`EXCEL` |
| `SQL` | ✅ | see [PROC SQL](#proc-sql) below | dictionary tables, `ODS OUTPUT` capture |
| `FORMAT` | 🟡 | `VALUE` (`$`/numeric, ranges `a-b`, `low-<b`, `a<-high`, value lists, `OTHER`), `INVALUE` (user informats), `PICTURE` (`PREFIX=`/`MULT=`/`FILL=`) | `CNTLIN=`/`CNTLOUT=`, `FMTLIB`, persistent format catalogs |
| `FREQ` | 🟡 | `DATA=`; `TABLES` one-way & two-way (`v1*v2`) with `/MISSING /OUT= /NOPERCENT /NOROW /NOCOL /NOFREQ /NOCUM /CHISQ /FISHER` (2×2) `/MEASURES /AGREE /TREND` | tables ≥3-way, `BY`, `WEIGHT`, `/LIST`, Fisher on tables >2×2 |
| `UNIVARIATE` | 🟡 | `DATA=`, `NOPRINT`, `NORMAL`/`NORMALTEST`; `VAR` (`/ normal`), `WEIGHT`, `BY` (parsed), `OUTPUT` (parsed). Report: Moments, Basic Measures, Quantiles, Extreme Obs, Tests for Normality | plots (`HISTOGRAM`/`QQPLOT`/`PROBPLOT`/`CDFPLOT` → NOTE only), weighted quantiles/extremes |
| `TABULATE` | 🟡 | `DATA=`; `CLASS`, `VAR`, `TABLE` (1/2/3 dims), stats `N NMISS SUM MEAN MIN MAX STD PCTN PCTSUM`, `ALL`, `*` crossings | `OUT=` dataset, `FORMAT=`/labels in headers, group denominators `PCTN<...>`, 4th dimension |
| `REPORT` | 🟡 | `DATA=`, `NOWD`/`NOWINDOW`, `NOHEADER`, `HEADLINE`, `HEADSKIP`, `OUT=`; `COLUMN`, `DEFINE` (`DISPLAY`/`ORDER`/`GROUP`/`ANALYSIS`, `ORDER=`, label), `WHERE`, `BREAK AFTER /SUMMARIZE`, `RBREAK`, `COMPUTE` (simple assignment) + `COMPUTE AFTER`/`LINE` | `DEFINE` `FORMAT=`/`WIDTH=`/`FLOW`/`SPACING`, complex `COMPUTE` (`_Cn_`, functions, formats) |
| `DATASETS` | 🟡 | `LIB=`/`LIBRARY=`, `NOLIST`; `DELETE`, `CHANGE old=new`, run-group `RUN`/`QUIT` | `COPY`, `APPEND`, `MODIFY`, `REPAIR`, `EXCHANGE`, `SAVE`, `CONTENTS` |
| `CATALOG` | 🟡 | `CATALOG=libref.cat`; `CONTENTS` (in-memory formats), `DELETE`/`COPY` (no-op + NOTE) | real `.sas7bcat` catalogs, entry-type selection |
| `PRINTTO` | 🟡 | `LOG=`, `PRINT=`, `NEW`, reset (options stored) | actual file routing (NOTE only; deferred) |
| `OPTIONS` | 🟡 | `PROC OPTIONS` listing of system options | per-option detail |
| `TTEST` | 🟡 | 1-sample (H0=, ALPHA=, SIDES=), 2-sample CLASS (Pooled + Satterthwaite + F equality test), PAIRED; VAR/CLASS/PAIRED statements; ODS OUTPUT TTest | BY groups, one-sided p wiring, CI columns |
| `NPAR1WAY` | 🟡 | CLASS (required), VAR (default all numeric), WILCOXON/KRUSKAL flags; Wilcoxon rank-sum (Z + 2-sided p, midranks, tie correction); Kruskal-Wallis (H/tie_factor, χ², df=k-1) | BY groups, OUT= dataset, exact Wilcoxon, score methods (Median/Savage/Normal) |
| `REG` | 🟡 | `DATA=`; `MODEL dep = x1 x2 … / NOPRINT`; `OUTPUT OUT= PREDICTED= RESIDUAL=`; OLS via QR (intercept only), ANOVA table, R²/Adj R²/F/t-tests, parameter estimates with SE, listwise missing deletion | `NOINT`, `TEST` statement, CLM/CLI prediction intervals, `BY`, multiple MODEL statements per run |
| `ANOVA` | 🟡 | `DATA=`; `CLASS` (one variable, distinct levels via `sas_cmp`); `MODEL y1 y2 = effect / NOPRINT` (multiple dependents, one CLASS effect); `MEANS effect`; one-way ANOVA table (Model/Error/Corrected Total, F/Pr>F), fit statistics (R², C.V., Root MSE, dep Mean), Type I SS + Type III SS (identical for one-way), cell means table with Std Dev | Interaction effects (`a*b`), multi-way ANOVA, multiple CLASS variables in MODEL, `MEANS` comparison tests (Tukey/Duncan/Scheffé), `BY` groups |
| `GLM` | 🟡 | `DATA=`; `CLASS` (one variable); `MODEL y1 y2 = effect / SOLUTION NOPRINT` (multiple dependents); `LSMEANS effect / SE`; `ESTIMATE 'label' effect c1 c2 …`; `CONTRAST 'label' effect c1 c2 …`; `MEANS effect`; ANOVA table + Type I/III SS (identical for one-way), fit statistics, parameter estimates (reference-cell coding, last level = 0 / "B"), LS means with SE, Contrasts (F/Pr>F df=1), Estimates (t/Pr>\|t\|) | Multi-way TYPE III ≠ TYPE I, interaction effects (`a*b`), multiple CLASS variables, `LSMEANS` comparison tests (Tukey/Dunnett), `BY` groups |
| `LOGISTIC` | 🟡 | `DATA=`; `MODEL y(DESCENDING EVENT='val') = x1 x2 / NOPRINT`; `FREQ var`; binary logistic regression via Newton-Raphson MLE; Model Fit Statistics (AIC/SC/-2LogL), Global Null Hypothesis tests (LR/Score/Wald), Analysis of ML Estimates (β/SE/Wald χ²/p), Odds Ratio Estimates (OR + 95% Wald CI) | `CLASS` variables (error: not yet implemented), multinomial/ordinal logistic, `LINK=`, `BY` groups, `OUTPUT OUT=`, `SCORE`, `UNITS`, `ROC` |
| `GENMOD` | 🟡 | `DATA=`; `FREQ var`; `MODEL y(DESCENDING EVENT='val') = x1 x2 / DIST= LINK= NOPRINT`; DIST=POISSON (link=LOG), BINOMIAL (link=LOGIT), NORMAL (link=IDENTITY); NR/IRLS MLE (GCONV=1e-8); Criteria For Assessing Goodness Of Fit (Deviance/Scaled Deviance/Pearson/LL/AIC/AICC/BIC); Analysis Of Maximum Likelihood Parameter Estimates (β/SE/Wald 95% CI/Wald χ²/p); Scale parameter (fixed=1 for Poisson/Binomial, estimated=√MSE for Normal); Response Profile (Binomial only) | `DIST=GAMMA` (not yet implemented), `CLASS`, multinomial, GEE/`REPEATED`, `OFFSET=`, `ESTIMATE`/`CONTRAST`, `BY`, `OUTPUT OUT=` |
| `PRINCOMP` | 🟡 | `DATA=`; `VAR var1 var2 …`; `N=k` (truncate display); `COV` (covariance instead of correlation); Simple Statistics (Mean/StdDev); Correlation Matrix (or Covariance Matrix); Eigenvalues table (Eigenvalue/Difference/Proportion/Cumulative); Eigenvectors; deterministic sign convention (largest-magnitude element positive) | `OUT=` component scores (parse accepted, not computed), `PARTIAL`, `WEIGHT`, `TYPE=CORR` input datasets, `BY` groups |
| `FACTOR` | 🟡 | `DATA=`; `VAR var1 var2 …`; `NFACTORS=k` (or Kaiser MINEIGEN λ>1 default); `METHOD=PRINCIPAL` (default); `ROTATE=VARIMAX` (Kaiser 1958 normalized Varimax) or `ROTATE=NONE`; `COV`; Prior Communality ONE; Eigenvalues; Factor Pattern (loadings); Variance Explained; Final Communality Estimates; Rotated Factor Pattern + Variance post-VARIMAX | `METHOD=ML/ITER`, oblique rotations, `OUT=` scores (parse accepted, not computed), `HEYWOOD`, `ALPHA`, `SCORE`, `BY` groups |
| `DISTANCE` | 🟡 | `DATA=`; `VAR var1 var2 …`; `OUT=ds` (distance matrix dataset); `METHOD=EUCLID/L2` (default), `CITYBLOCK/L1`, `LINF/CHEBYCHEV`, `COSINE`, `CORR`; Distance Matrix listing (Row/Col labeled); output `_TYPE_=DISTANCE` dataset | `SHAPE=`, `FREQ`, normalization options, `ID` variable for row labels |
| `CLUSTER` | 🟡 | `DATA=`; `VAR var1 var2 …`; `METHOD=WARD` (default), `AVERAGE`, `SINGLE`, `COMPLETE`; `ID var`; Cluster History (NClusters, Clusters Joined, Freq, SPRSQ, RSQ); Lance-Williams update formula | `OUTTREE=` (parse accepted, not computed), `PSEUDO=`, `NOEIGEN`, graphical dendrogram |
| `FASTCLUS` | 🟡 | `DATA=`; `VAR var1 var2 …`; `MAXCLUSTERS=k` (required); `OUT=ds` (with `_CLUSTER_` variable); `MAXITER=`; `CONVERGE=`; farthest-first seed selection; Cluster Summary (Freq/RMS Std/Max Distance/Nearest Cluster); Statistics for Variables (R-Square); `ID var` | `SEED=` (specific seed obs), `RADIUS=`, `DISTANCE`, fuzzy clustering, `MEAN` |

### DATA step

| Area | State | Detail |
| --- | :---: | --- |
| Data sources | ✅ | `SET` (incl. `END=`/`NOBS=`/`POINT=`, multi-dataset concat), `MERGE` + `IN=`, `BY` interleave, `UPDATE`, `MODIFY` |
| Dataset options | ✅ | `KEEP=`, `DROP=`, `RENAME=(a=b)`, `WHERE=()` (`FIRSTOBS=`/`OBS=` only on `INFILE`) |
| External input | ✅ | `INFILE` (`DELIMITER=`/`DLM=`, `DSD`, `FIRSTOBS=`, `OBS=`, `MISSOVER`, `TRUNCOVER`, `STOPOVER`, `LRECL=`), `INPUT` (list / column / formatted), `DATALINES`/`CARDS` |
| Text output | ✅ | `FILE` (`LOG`/`PRINT`/external path), `PUT` (named / formatted / literal / `_ALL_`, `@n`/`+n`/`/`, `@`/`@@` hold) |
| Control flow | ✅ | `IF/THEN/ELSE`, subsetting `IF`, `DO`/`END`, iterative `DO ... TO ... BY [WHILE/UNTIL]`, `DO WHILE`, `DO UNTIL`, `DO` value list, `DO OVER`, `SELECT/WHEN/OTHERWISE`, labels + `GOTO`/`LINK`/`RETURN`, `OUTPUT`, `DELETE`, `STOP` |
| Variables & attributes | ✅ | `RETAIN`, sum statement (`var + expr`), `LENGTH`, `FORMAT`, `LABEL`, `ATTRIB`, `KEEP`, `DROP`, `ARRAY` (multi-dim, `_NUMERIC_`/`_CHARACTER_`/`_ALL_`, temporary, `DO OVER`) |
| Automatic variables | ✅ | `_N_`, `_ERROR_`, `FIRST.`/`LAST.`, `END=`, `NOBS=`, `POINT=`, `IN=` |
| Hash objects | ✅ | `DECLARE HASH`/`HITER`, methods `find/check/add/replace/remove/clear/output/num_items/find_next/find_prev`, `ordered:`/`duplicate:`/`multidata:`/`dataset:` |
| `CALL` routines | 🟡 | `CALL SYMPUT`, `CALL EXECUTE` (others parsed → runtime error) |
| Not supported | 🔴 | standalone `WHERE` statement, bare `SET;`/`MERGE;`, multiple `SET` statements, `INFORMAT` statement |

### DATA step / macro functions

~115 functions are implemented across these categories:

| Category | Functions |
| --- | --- |
| Descriptive | `SUM MEAN MIN MAX N NMISS RANGE LARGEST SMALLEST ORDINAL COALESCE MISSING` |
| Math | `ABS SQRT EXP LOG LOG2 LOG10 INT ROUND ROUNDZ MOD CEIL FLOOR SIGN FACT COMB PERM GAMMA LGAMMA DIGAMMA BETA` |
| Trigonometry | `SIN COS TAN ARSIN ARCOS ATAN ATAN2 SINH COSH TANH` |
| Strings | `UPCASE LOWCASE PROPCASE TRIM STRIP LEFT LENGTH SUBSTR SUBSTRN INDEX FIND FINDC COUNT COUNTC VERIFY SCAN CAT CATS CATX CATQ COMPRESS COMPBL TRANWRD TRANSLATE REVERSE REPEAT CHAR BYTE RANK WHICHC` |
| Dates & times | `TODAY DATE MDY YEAR MONTH DAY WEEKDAY INTCK INTNX DATEPART TIMEPART DATETIME DHMS HMS HOUR MINUTE SECOND DATDIF YRDIF JULDATE DATEJUL NLDATE` |
| Conversion | `PUT INPUT` |
| Distributions | `CDF PDF SDF LOGCDF QUANTILE PROBNORM PROBT PROBF PROBCHI PROBBETA PROBGAM PROBBNML POISSON` |
| Random variates | `RAND RANUNI RANNOR RANEXP RANBIN` |
| Macro bridge | `SYMGET` |

### Macro language

| Feature | State | Detail |
| --- | :---: | --- |
| Definition / call | ✅ | `%MACRO`/`%MEND`, positional & keyword params with defaults, `%name(args)` |
| Variables | ✅ | `%LET`, `&var`/`&var.`, `%LOCAL`, `%GLOBAL`, nested indirection `&&&x` |
| Control flow | ✅ | `%IF/%THEN/%ELSE`, `%DO/%END`, `%DO i=a %TO b %BY c`, `%DO %WHILE`, `%DO %UNTIL` |
| Evaluation | ✅ | `%EVAL`, `%SYSEVALF`, `%SYSFUNC`/`%QSYSFUNC` (whitelisted DATA step functions) |
| Quoting | 🟡 | `%STR`, `%NRSTR`, `%UNQUOTE`, `%CMPRES`, `%QCMPRES` (no `%SUPERQ`/`%BQUOTE`/`%NRBQUOTE`) |
| Utilities | ✅ | `%PUT`, `%INCLUDE` (quoted path) + autocall (`SASAUTOS`), `%SYMEXIST`, `%SYSMEXIST`, `%SYSGET` |
| Automatic vars | ✅ | `&SYSDATE(9)`, `&SYSTIME`, `&SYSDAY`, `&SYSDAYNUM`, `&SYSMONTH`, `&SYSYEAR` |
| Tracing | ✅ | `MPRINT`, `MLOGIC`, `SYMBOLGEN` |

### PROC SQL

| Feature | State | Detail |
| --- | :---: | --- |
| Queries | ✅ | `SELECT [DISTINCT]`, `WHERE`, `GROUP BY` (incl. positional), `HAVING`, `ORDER BY [ASC\|DESC]`, `CALCULATED`, column/table aliases |
| DDL/DML | ✅ | `CREATE TABLE AS`, `CREATE VIEW`, `DROP TABLE`/`VIEW`, `INSERT ... VALUES`/`SELECT`, `UPDATE ... SET`, `DELETE`, `DESCRIBE TABLE` |
| Joins | ✅ | `INNER`, `LEFT`, `RIGHT`, `FULL`, `CROSS` |
| Predicates | ✅ | `BETWEEN`, `IS [NOT] NULL`/`MISSING`, `LIKE` (`%`/`_`), `IN`/`NOT IN`, scalar/`IN`/`EXISTS` subqueries |
| Set operators | ✅ | `UNION [ALL]`, `EXCEPT`, `INTERSECT` |
| Aggregates | ✅ | `COUNT(*)`, `COUNT([DISTINCT] col)`, `SUM`, `AVG`/`MEAN`, `MIN`, `MAX` |
| Not supported | 🔴 | dictionary tables, `ODS OUTPUT` capture, `CONTAINS`/`SOUNDS LIKE` |

### Output (ODS) & formats

| Area | State | Detail |
| --- | :---: | --- |
| ODS destinations | ✅ | `LISTING` (text), `HTML`, `RTF`, `PDF`, `EXCEL` (xlsx); `ODS <dest> CLOSE`, `ODS _ALL_ CLOSE`, `FILE=`, `STYLE=` (partial) |
| ODS — not supported | 🔴 | `ODS SELECT`/`EXCLUDE`, `ODS OUTPUT` capture, graphics, embedded images |
| Built-in formats | ✅ | numeric (`w.d`, `BEST`, `COMMA`, `DOLLAR`, `Z`, `PERCENT`, `E`, `EURO`, `COMMAX`…), dates/times (`DATEw`, `DDMMYY`, `MMDDYY`, `YYMMDD`, `DATETIME`, `TIME`, `MONYY`, `WEEKDATE`, `DOWNAME`, ISO 8601 `B8601`/`E8601`…), character (`$w`, `$CHAR`, `$UPCASE`, `$HEX`, `$QUOTE`), specials (`HEX`, `BINARY`, `OCTAL`, `ROMAN`, `WORDS`, `FRACT`, `NEGPAREN`) |
| Built-in informats | ✅ | `w.d`, `COMMA`, `DOLLAR`, `DATEw`, `MMDDYY`/`DDMMYY`/`YYMMDD`, `TIME`, `$CHAR`/`$w` |
| User formats | 🟡 | `PROC FORMAT` `VALUE`/`INVALUE`/`PICTURE` (in-memory; no `CNTLIN=`/`CNTLOUT=` or persistent catalogs) |

### Global statements

| Statement | State | Detail |
| --- | :---: | --- |
| `LIBNAME` | ✅ | assign / `CLEAR` (path resolved against the program dir; `s3://` with the `s3` feature) |
| `OPTIONS` | 🟡 | `LINESIZE`/`LS=` applied; other options parsed with a "not yet supported" warning |
| `TITLE` | 🟡 | `TITLE1`–`TITLE9` parsed (only `TITLE1` rendered in the listing) |
| `ODS` | ✅ | see Output section |
| `%INCLUDE` | 🟡 | quoted paths + autocall (no fileref / stdin form) |
| `FILENAME`, `X` | 🔴 | not supported |

> The coverage above reflects the current state of `main`; statistical modelling
> procedures (`TTEST`, `NPAR1WAY`, and beyond) are the active milestones. See
> `PROGRESS.md` and `PLAN.md` for the full roadmap.

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
