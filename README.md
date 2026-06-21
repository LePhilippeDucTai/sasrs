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
| `PRINT` | ✅ | `DATA=`, `NOOBS`, `LABEL`, `DOUBLE`, `N`; `VAR`, `BY` (per-group sections, sorted input), `ID` (replaces `Obs`), `SUM` (per-`BY`-group subtotals + grand total) | `WHERE`, `SUMBY`, `PAGEBY`, style options |
| `SORT` | ✅ | `DATA=`, `OUT=`, `NODUPKEY`, `NODUPRECS`/`NODUP`, `TAGSORT` (no-op hint), `SORTSEQ=ASCII\|LINGUISTIC` (LINGUISTIC falls back to `sas_cmp` binary order); `BY [DESCENDING]`, `KEY=var [/ DESCENDING]` | — |
| `CONTENTS` | ✅ | `DATA=`, `VARNUM`, `DATA=lib._ALL_`, `OUT=` (one row/variable: `NAME`/`TYPE` 1=num 2=char/`LENGTH`/`VARNUM`/`LABEL`/`FORMAT`), `SHORT` (flat name list), `DETAILS`/`NODETAILS` (obs/var header lines) | physical file-size/page details, ODS output object |
| `MEANS` / `SUMMARY` | ✅ | `DATA=`, `NOPRINT`, `PRINTALLTYPES`, stat keywords (`N NMISS MEAN STD MIN MAX SUM RANGE STDERR CV MEDIAN CLM LCLM UCLM` + percentiles `P1 P5 P10 P20 P25 P30 P40 P50 P60 P70 P75 P80 P90 P95 P99 Q1 Q3 QRANGE`, Definition 5); `CLASS`, `VAR`, `BY`, `WEIGHT`, `WAYS`, `TYPES`, `OUTPUT OUT= stat(var)=name` | `MAXDEC=`, `NWAY`, `MISSING`, `ORDER=`, `ID`, multi-label formats |
| `TRANSPOSE` | ✅ | `DATA=`, `OUT=`, `PREFIX=`, `NAME=`; `BY`, `ID`, `VAR` | `IDLABEL`, `COPY`, `LET`, `SUFFIX=` |
| `APPEND` | ✅ | `BASE=`, `DATA=`, `FORCE`, `NOWARN` (suppresses FORCE structural-diff warnings), `APPENDVER=Vn` (no-op hint) | — |
| `RANK` | ✅ | `DATA=`, `OUT=`, `DESCENDING`, `TIES=(MEAN\|LOW\|HIGH\|DENSE)`, `GROUPS=`, methods `FRACTION`/`NPLUS1`/`PERCENT`/`NORMAL=(BLOM\|TUKEY\|VW)`/`SAVAGE`; `VAR`, `RANKS`, `BY` | — |
| `CORR` | ✅ | `DATA=`, `NOSIMPLE`, `NOPROB`, `NOCORR`, `PEARSON`, `SPEARMAN`, `KENDALL`, `HOEFFDING` (D exact ≡ SAS ; `Prob > D` = approximation asymptotique Blum-Kiefer-Rosenblatt/Imhof, n≥5), `OUT=`/`OUTP=`/`OUTS=`/`OUTK=`; `VAR`, `WITH`, `WEIGHT` (Pearson, Spearman & Kendall — rangs moyens pondérés ; paires Kendall pondérées par wᵢ·wⱼ ; ≡ méthode ordinaire sur données répliquées), `PARTIAL` (Pearson partial correlation, df = n−k−2, residual/least-squares method) | partial Spearman/Kendall ; `Prob > D` tabulée exacte pour petit n |
| `COMPARE` | ✅ | `BASE=`, `COMPARE=`, `OUT=`, `NOVALUES`, `BRIEFSUMMARY` | `CRITERION=`, `ID`, `VAR`/`WITH` |
| `IMPORT` | ✅ | `DATAFILE=`/`FILENAME=`, `OUT=`, `DBMS=(CSV\|TAB\|DLM)`, `REPLACE`; `GETNAMES=`, `DELIMITER=`/`DLM=` | `DBMS=XLSX`/`EXCEL`, `GUESSINGROWS=` (parsed, ignored) |
| `EXPORT` | ✅ | `DATA=`, `OUTFILE=`, `DBMS=(CSV\|TAB\|DLM)`, `REPLACE`; `DELIMITER=`/`DLM=` | `DBMS=XLSX`/`EXCEL` |
| `SQL` | ✅ | see [PROC SQL](#proc-sql) below | dictionary tables, `ODS OUTPUT` capture |
| `FORMAT` | 🟡 | `VALUE` (`$`/numeric, ranges `a-b`, `low-<b`, `a<-high`, value lists, `OTHER`), `INVALUE` (user informats), `PICTURE` (`PREFIX=`/`MULT=`/`FILL=`) | `CNTLIN=`/`CNTLOUT=`, `FMTLIB`, persistent format catalogs |
| `FREQ` | 🟡 | `DATA=`; `TABLES` one-way, two-way (`v1*v2`) & n-way (`v1*v2*…`, stratified two-way of the last two vars) with `/MISSING /OUT= /NOPERCENT /NOROW /NOCOL /NOFREQ /NOCUM /LIST /CHISQ /FISHER` (2×2) `/MEASURES /AGREE /TREND`; `WEIGHT` (sum of weights into frequencies & CHISQ), `BY` | Fisher on tables >2×2 |
| `UNIVARIATE` | 🟡 | `DATA=`, `NOPRINT`, `NORMAL`/`NORMALTEST`; `VAR` (`/ normal`), `WEIGHT`, `BY` (parsed), `OUTPUT` (parsed). Report: Moments, Basic Measures, Quantiles, Extreme Obs, Tests for Normality. With `WEIGHT`: weighted Quantiles (weighted Definition 5 — cumulative-weight position) and weighted `Median`/`Q1`/`Q3`/`Range`/`IQR`, plus Extreme Obs (raw extreme values). Plots wired to ODS GRAPHICS: `HISTOGRAM`/`QQPLOT`/`PROBPLOT`/`CDFPLOT`/`PPPLOT` → PNG/SVG under `--features graphics` (`univar_{N}`; histogram, normal-QQ/normal-probability scatter, empirical CDF, P-P scatter), else the shared "image deferred" NOTE; nothing emitted when ODS GRAPHICS off | weighted skewness/kurtosis (computed unweighted); plot-statement options (`/ NORMAL`, annotation) |
| `TABULATE` | 🟡 | `DATA=`; `CLASS`, `VAR`, `TABLE` (1/2/3 dims), stats `N NMISS SUM MEAN MIN MAX STD PCTN PCTSUM`, `ALL`, `*` crossings, `OUT=` cell dataset, `FORMAT=`/`*f=` cell formats, `='label'` + stored LABEL in headers | group denominators `PCTN<...>` |
| `REPORT` | 🟡 | `DATA=`, `NOWD`/`NOWINDOW`, `NOHEADER`, `HEADLINE`, `HEADSKIP`, `OUT=`; `COLUMN`, `DEFINE` (`DISPLAY`/`ORDER`/`GROUP`/`ANALYSIS`, `ORDER=`, label, `FORMAT=`, `WIDTH=`, `SPACING=`), `WHERE`, `BREAK AFTER /SUMMARIZE`, `RBREAK`, `COMPUTE` (assignment + `_Cn_`/named column refs) + `COMPUTE AFTER`/`LINE` (with `@col` pointer and trailing format) | `DEFINE` `FLOW`; richer `COMPUTE` (assignment back into computed columns with the full function library) |
| `DATASETS` | 🟡 | `LIB=`/`LIBRARY=`, `NOLIST`; `DELETE`, `CHANGE old=new`, `COPY OUT= [IN=] [;SELECT ...]`, `EXCHANGE a=b`, `SAVE m1 m2`, `MODIFY m; RENAME old=new; LABEL v='..'`, run-group `RUN`/`QUIT` | `APPEND`, `REPAIR`, `CONTENTS` (inside DATASETS), `MODIFY` dataset-level attrs |
| `CATALOG` | 🟡 | `CATALOG=libref.cat`; `CONTENTS` (in-memory formats), `DELETE`/`COPY` (no-op + NOTE) | real `.sas7bcat` catalogs, entry-type selection |
| `PRINTTO` | 🟡 | `LOG=`, `PRINT=`, `NEW`, reset (options stored) | actual file routing (NOTE only; deferred) |
| `OPTIONS` | 🟡 | `PROC OPTIONS` listing of system options | per-option detail |
| `TTEST` | ✅ | 1-sample (H0=, ALPHA=, SIDES= with one-sided p), 2-sample CLASS (Pooled + Satterthwaite + F equality test), PAIRED; VAR/CLASS/PAIRED/BY statements; CI= confidence-limit columns (mean + std); ODS OUTPUT TTest | — |
| `NPAR1WAY` | ✅ | CLASS (required), VAR (default all numeric), BY groups; WILCOXON/KRUSKAL flags; Wilcoxon rank-sum (Z + 2-sided p, midranks, tie correction) + exact permutation test (EXACT, n≤30); Kruskal-Wallis (H/tie_factor, χ², df=k-1); MEDIAN/SAVAGE/NORMAL(=VW) score tests (2-sample Z + one-way χ²); OUTPUT OUT= dataset (`_WIL_/Z_WIL/P2_WIL/P1_WIL`, `XP1_WIL/XP2_WIL`, `_KW_/DF_KW/P_KW`, `_MED_/_SAV_/_VW_` families) | Exact test for Median/Savage/Normal scores (Wilcoxon-rank only); BY-key OUT= columns stored as formatted strings |
| `REG` | 🟡 | `DATA=`; multiple `MODEL dep = x1 x2 … / NOINT NOPRINT SELECTION=` per run (labelled MODEL1, MODEL2…); `NOINT` (uncorrected SS, `Uncorrected Total`, no Intercept row); `SELECTION=FORWARD\|BACKWARD\|STEPWISE` with `SLENTRY=`/`SLE=`, `SLSTAY=`/`SLS=` (partial-F entry/removal + selection-summary table); `OUTPUT OUT= PREDICTED= RESIDUAL=` (per MODEL); OLS via QR, ANOVA table, R²/Adj R²/F/t-tests, parameter estimates with SE, listwise missing deletion. Diagnostics wired to ODS GRAPHICS: automatic residuals-vs-predicted scatter after each `MODEL` → PNG/SVG under `--features graphics` (`reg_{N}`), else NOTE; `PLOTS` statement parsed → NOTE-deferred | `TEST` statement, CLM/CLI prediction intervals, `BY`, `SELECTION=RSQUARE/ADJRSQ/CP/LASSO`, full `PLOTS=` diagnostic panel |
| `ANOVA` | ✅ | `DATA=`; `CLASS` (one or more variables, distinct levels via `sas_cmp`); `MODEL y1 y2 = effects / NOPRINT` (multiple dependents; main effects, interactions `a*b`, multiple CLASS); `MEANS effect`; per-effect ANOVA table (Model/Error/Corrected Total, F/Pr>F), fit statistics (R², C.V., Root MSE, dep Mean), **Type I SS** (sequential, reference-cell) + **Type III SS** (partial, sum-to-zero effect coding → matches SAS for unbalanced designs with interactions), cell means with Std Dev | `MEANS` comparison tests (Tukey/Duncan/Scheffé), Type II/IV SS, nested/continuous-covariate effects, `BY` groups |
| `GLM` | ✅ | `DATA=`; `CLASS` (one or more); `MODEL y1 y2 = effects / SOLUTION NOPRINT` (multiple dependents; main effects, interactions `a*b`, multiple CLASS); `LSMEANS effect / SE` (uniform marginal LS means, multi-way); `ESTIMATE`/`CONTRAST 'label' effect c…` (main effects); `MEANS effect`; ANOVA table + **Type I** (sequential) / **Type III** (sum-to-zero effect coding, SAS-matching for unbalanced+interaction) SS, fit statistics, reference-cell parameter estimates (last level = 0 / "B", interaction cross-labels), LS means with SE, Contrasts (F/Pr>F), Estimates (t/Pr>\|t\|) | `LSMEANS` comparison/adjust (Tukey/Dunnett), `ESTIMATE`/`CONTRAST` on interaction terms, Type II/IV SS, continuous covariates, `BY` groups |
| `LOGISTIC` | 🟡 | `DATA=`; `MODEL y(DESCENDING EVENT='val') = x1 x2 / NOPRINT`; `FREQ var`; binary logistic regression via Newton-Raphson MLE; Model Fit Statistics (AIC/SC/-2LogL), Global Null Hypothesis tests (LR/Score/Wald), Analysis of ML Estimates (β/SE/Wald χ²/p), Odds Ratio Estimates (OR + 95% Wald CI) | `CLASS` variables (error: not yet implemented), multinomial/ordinal logistic, `LINK=`, `BY` groups, `OUTPUT OUT=`, `SCORE`, `UNITS`, `ROC` |
| `GENMOD` | 🟡 | `DATA=`; `FREQ var`; `MODEL y(DESCENDING EVENT='val') = x1 x2 / DIST= LINK= NOPRINT`; DIST=POISSON (link=LOG), BINOMIAL (link=LOGIT), NORMAL (link=IDENTITY); NR/IRLS MLE (GCONV=1e-8); Criteria For Assessing Goodness Of Fit (Deviance/Scaled Deviance/Pearson/LL/AIC/AICC/BIC); Analysis Of Maximum Likelihood Parameter Estimates (β/SE/Wald 95% CI/Wald χ²/p); Scale parameter (fixed=1 for Poisson/Binomial, estimated=√MSE for Normal); Response Profile (Binomial only) | `DIST=GAMMA` (not yet implemented), `CLASS`, multinomial, GEE/`REPEATED`, `OFFSET=`, `ESTIMATE`/`CONTRAST`, `BY`, `OUTPUT OUT=` |
| `PRINCOMP` | 🟡 | `DATA=`; `VAR var1 var2 …`; `N=k` (truncate display); `COV` (covariance instead of correlation); Simple Statistics (Mean/StdDev); Correlation Matrix (or Covariance Matrix); Eigenvalues table (Eigenvalue/Difference/Proportion/Cumulative); Eigenvectors; deterministic sign convention (largest-magnitude element positive) | `OUT=` component scores (parse accepted, not computed), `PARTIAL`, `WEIGHT`, `TYPE=CORR` input datasets, `BY` groups |
| `FACTOR` | 🟡 | `DATA=`; `VAR var1 var2 …`; `NFACTORS=k` (or Kaiser MINEIGEN λ>1 default); `METHOD=PRINCIPAL` (default); `ROTATE=VARIMAX` (Kaiser 1958 normalized Varimax) or `ROTATE=NONE`; `COV`; Prior Communality ONE; Eigenvalues; Factor Pattern (loadings); Variance Explained; Final Communality Estimates; Rotated Factor Pattern + Variance post-VARIMAX | `METHOD=ML/ITER`, oblique rotations, `OUT=` scores (parse accepted, not computed), `HEYWOOD`, `ALPHA`, `SCORE`, `BY` groups |
| `DISTANCE` | 🟡 | `DATA=`; `VAR var1 var2 …`; `OUT=ds` (distance matrix dataset); `METHOD=EUCLID/L2` (default), `CITYBLOCK/L1`, `LINF/CHEBYCHEV`, `COSINE`, `CORR`; Distance Matrix listing (Row/Col labeled); output `_TYPE_=DISTANCE` dataset | `SHAPE=`, `FREQ`, normalization options, `ID` variable for row labels |
| `CLUSTER` | 🟡 | `DATA=`; `VAR var1 var2 …`; `METHOD=WARD` (default), `AVERAGE`, `SINGLE`, `COMPLETE`; `ID var`; Cluster History (NClusters, Clusters Joined, Freq, SPRSQ, RSQ); Lance-Williams update formula | `OUTTREE=` (parse accepted, not computed), `PSEUDO=`, `NOEIGEN`, graphical dendrogram |
| `FASTCLUS` | 🟡 | `DATA=`; `VAR var1 var2 …`; `MAXCLUSTERS=k` (required); `OUT=ds` (with `_CLUSTER_` variable); `MAXITER=`; `CONVERGE=`; farthest-first seed selection; Cluster Summary (Freq/RMS Std/Max Distance/Nearest Cluster); Statistics for Variables (R-Square); `ID var` | `SEED=` (specific seed obs), `RADIUS=`, `DISTANCE`, fuzzy clustering, `MEAN` |
| `DISCRIM` | 🟡 | `DATA=`; `CLASS var`; `VAR var1 var2 …`; `ID var`; `OUT=ds` (`_FROM_`, `_INTO_`, `_<k>` posteriors); `PRIORS EQUAL` (default) / `PROPORTIONAL`; `POOL=YES` (LDA); Class Level Information; Within-Class Covariance Matrices; Pooled Covariance; Pairwise D² (Mahalanobis²); Linear Discriminant Function Coefficients; Classification Results (obs-by-obs + posteriors); Error Count Estimates | `POOL=NO` (QDA), `CROSSVALIDATE`, `OUTSTAT=`, `METHOD=NPAR/KERNEL`, `THRESHOLD=`, `BY` groups |
| `MIXED` | 🟡 | `DATA=`; `METHOD=REML` (default) / `ML`; `CLASS var1 …`; `MODEL dep = / SOLUTION`; `RANDOM INTERCEPT / SUBJECT=var TYPE=VC` (Variance Components) or `TYPE=CS` (Compound Symmetry); REML/ML via closed-form (balanced) or golden-section profile search (unbalanced); Fixed-effects solution (β̂/SE/t/df/p) with Contain df; 8 listing sections (Model Information, Class Level Info, Dimensions, Nobs, Iteration History, Covariance Parameter Estimates, Fit Statistics, Solution for Fixed Effects) | `TYPE=AR(1)`/`UN` (error: not yet implemented); `RANDOM` slopes; `REPEATED`; `LSMEANS`; `ESTIMATE`; `CONTRAST`; `COVTEST`; multiple random effects; `BY` groups |
| `GLIMMIX` | 🟡 | `DATA=`; `METHOD=RSPL` (default, Residual Pseudo-Likelihood); `CLASS var1 …`; `MODEL dep[(event='val')] = x1 x2 / DIST=NORMAL\|POISSON\|BINARY LINK=IDENTITY\|LOG\|LOGIT SOLUTION`; `RANDOM INTERCEPT / SUBJECT=var TYPE=VC`; `FREQ var`; RSPL/PQL (Breslow-Clayton 1993): IRLS linearisation → weighted MME at each step; NORMAL/IDENTITY+random ≡ REML (exact); Poisson/Binary without random ≡ GENMOD/LOGISTIC (cross-validated); 3-way RSPL dispatch; Generalized Chi-Square; Type III Tests of Fixed Effects (F/t with Contain df); Solutions for Fixed Effects | `METHOD=LAPLACE`/`QUAD` (error: not yet implemented); `DIST=GAMMA` (error); `TYPE=AR(1)`/`UN` (error); `RANDOM` slopes; `REPEATED`; `LSMEANS`; `ESTIMATE`; `CONTRAST`; `WEIGHT`; `BY` groups |
| `IML` | 🟡 | Sub-language (own lexer/parser/evaluator). Matrix literals `{1 2, 3 4}`, `{"x" "y"}`; indexing `A[i,j]`/`A[i,*]`; operators `'` (transpose), `*` (matmul), `#` (Hadamard), `@` (Kronecker), `+ - /`, comparisons; `NROW`/`NCOL`/`DIM`/`T`; stats `SUM`/`MEAN`/`STD`/`MIN`/`MAX`/`ABS`/`SQRT`/`EXP`/`LOG`; control flow `IF/THEN/ELSE`, `DO i=a TO b [BY c]`, `DO WHILE/UNTIL`; `PRINT`; linear algebra `INV`, `SOLVE`, `EIGVAL` (symmetric), `CHOL` (upper U), `CALL QR(Q,R,A)`, `CALL SVDCD(U,D,V,A)`; I/O `CREATE ds FROM mat[COLNAME=]`, `APPEND FROM`, `CLOSE`, `USE`, `READ ALL VAR {..} INTO mat` | `SHAPE`, range subscripts `a[1:2,1:2]`, `EIGVEC`/`DET`/`CALL EIGEN`, `READ NEXT`, `WHERE`, `LOAD`/`STORE`/`SHOW`, modules (`START`/`FINISH`) |
| `GPLOT` | 🟡 | `DATA=`; `PLOT y*x` / `PLOT y*x=group` / `PLOT (y1 y2)*x`. Without `--features graphics`: NOTE "image deferred". With `--features graphics`: PNG/SVG scatter via `gplot_{N}`. `SYMBOL`/`AXIS` statements parsed + NOTE | Actual SYMBOL/AXIS rendering; multi-overlay; `VPLOT`; `BY` |
| `GCHART` | 🟡 | `DATA=`; `VBAR`/`HBAR cat / SUMVAR= TYPE=FREQ\|SUM\|MEAN`. Without `--features graphics`: NOTE "image deferred". With `--features graphics`: VBar chart via `gchart_{N}` | `PIE` (deferred NOTE); actual SUM/MEAN aggregation under graphics; `SUBGROUP=`; `BY` |
| `PLOT` | 🟡 | `DATA=`; `PLOT y*x` / `PLOT y*x='sym'` / `PLOT (y1 y2)*x` / `PLOT y*x=group`. When ODS GRAPHICS OFF: ASCII scatter in listing (20×60 grid, A/B/C overlaps, labelled axes). When ODS GRAPHICS ON: delegates to image (`plot_{N}`) | `HREF=`/`VREF=`; `HAXIS=`/`VAXIS=`; multiple plots in one grid; `BY` |
| `SGPLOT` | 🟡 | `DATA=`; statements `SCATTER x= y= / GROUP= MARKERATTRS=()`, `SERIES x= y=`, `VBAR`/`HBAR cat / RESPONSE= STAT=FREQ\|SUM\|MEAN`, `HISTOGRAM var / BINWIDTH= SCALE=`, `DENSITY`, `VBOX resp / CATEGORY=`, `REG x= y= / DEGREE=`, `LOESS x= y= / SMOOTH=`, `XAXIS`/`YAXIS LABEL= VALUES=(min to max by step) TYPE=LINEAR\|LOG\|DISCRETE`, `BY`. Without `--features graphics`: NOTE "image deferred", byte-identical default build. With `--features graphics`: PNG/SVG written via `plotters` (`DrawingSpec`/`draw_to_file`), sequential naming `{IMAGENAME\|sgplot}_{N}.{ext}`. `LOESS`/`DENSITY`/`BY` parsed but deferred (NOTE) | v1 renders only the first plot statement per PROC; `HBAR`/`VBOX`/`REG` rendering (parse-only under graphics → NOTE); `MARKERATTRS=`/`LINEATTRS=` parsed but ignored; multi-plot overlays; legends; `BY`-group images; `LOESS`/`DENSITY` curves |

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
| `ODS GRAPHICS` | 🟡 | `ON`/`OFF`, `WIDTH=`/`HEIGHT=`/`IMAGEFMT=(PNG\|SVG)`/`IMAGENAME=`/`RESET=` (per-statement); image rendering via the optional `graphics` feature (`plotters`). Drives PROC SGPLOT plots, PROC UNIVARIATE `HISTOGRAM`/`QQPLOT`, and PROC REG residual diagnostics. Default build is byte-identical (NOTE only, no image) |
| ODS — not supported | 🔴 | `ODS SELECT`/`EXCLUDE`, `ODS OUTPUT` capture, embedded images |
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
