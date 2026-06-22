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
| `REG` | 🟡 | `DATA=`; multiple `MODEL dep = x1 x2 … / NOINT NOPRINT SELECTION=` per run (labelled MODEL1, MODEL2…); `NOINT` (uncorrected SS, `Uncorrected Total`, no Intercept row); `SELECTION=FORWARD\|BACKWARD\|STEPWISE` with `SLENTRY=`/`SLE=`, `SLSTAY=`/`SLS=` (partial-F entry/removal + selection-summary table), `SELECTION=RSQUARE\|ADJRSQ\|CP\|MAXR\|MINR\|NONE` (all-subsets / R²-improvement: per-size or ranked model tables with R²/Adj R²/Mallows C(p)) with `BEST=`/`INCLUDE=`/`START=`/`STOP=`/`DETAILS`/`STB`; `OUTPUT OUT= PREDICTED= RESIDUAL=` (per MODEL); OLS via QR, ANOVA table, R²/Adj R²/F/t-tests, parameter estimates with SE, listwise missing deletion. `TEST` statement (linear hypotheses on β: comma-separated equations → "Test … Results" table, F num/den df, Pr>F); `RESTRICT` statement (linear equality constraints → constrained LS re-estimation, restricted ANOVA/estimates + RESTRICT Lagrange-multiplier row, DF=-1); MODEL `CLB` (parameter confidence limits), `ALPHA=`, `CLM`/`CLI` (Output Statistics table: predicted, Std Error Mean Predict, CL Mean/Predict, residual) + OUTPUT keywords `STDP STDI STDR LCL UCL LCLM UCLM` (leverage-based); MODEL `R` (residual analysis: Std Error Residual, Student Residual + gauge, Cook's D, Sum/PRESS block) and `INFLUENCE` (RStudent, Hat Diag, Cov Ratio, DFFITS, per-parameter DFBETAS) + OUTPUT keywords `STUDENT RSTUDENT COOKD H PRESS DFFITS COVRATIO DFBETAS`; collinearity/specification MODEL options `VIF`/`TOL` (parameter table), `COLLIN`/`COLLINOINT` (eigenvalue condition indices + variance proportions), `SPEC` (White test χ²), `DW`/`DWPROB` (Durbin-Watson D + 1st-order autocorrelation + normal-approx p-values), `ACOV`/`HCC` (White HC0 covariance + heteroscedasticity-consistent SE table); partial SS/correlations MODEL options `SS1`/`SS2` (Type I/II SS), `STB` (standardized estimates), `PCORR1`/`PCORR2` (squared partial corr), `SCORR1`/`SCORR2` (squared semi-partial corr), `SEQB` (sequential estimates), `PRESS` (model PRESS statistic); Diagnostics wired to ODS GRAPHICS: automatic residuals-vs-predicted scatter after each `MODEL` → PNG/SVG under `--features graphics` (`reg_{N}`), else NOTE; `PLOTS` statement parsed → NOTE-deferred | `BY`, `SELECTION=LASSO`, full `PLOTS=` diagnostic panel |
| `ANOVA` | ✅ | `DATA=`; `CLASS` (one or more variables, distinct levels via `sas_cmp`); `MODEL y1 y2 = effects / NOPRINT` (multiple dependents; main effects, interactions `a*b`, multiple CLASS); `MEANS effect`; per-effect ANOVA table (Model/Error/Corrected Total, F/Pr>F), fit statistics (R², C.V., Root MSE, dep Mean), **Type I SS** (sequential, reference-cell) + **Type III SS** (partial, sum-to-zero effect coding → matches SAS for unbalanced designs with interactions), cell means with Std Dev | `MEANS` comparison tests (Tukey/Duncan/Scheffé), Type II/IV SS, nested/continuous-covariate effects, `BY` groups |
| `GLM` | ✅ | `DATA=`; `CLASS` (one or more); `MODEL y1 y2 = effects / SOLUTION NOPRINT` (multiple dependents; main effects, interactions `a*b`, multiple CLASS); `LSMEANS effect / SE` (uniform marginal LS means, multi-way); `ESTIMATE`/`CONTRAST 'label' effect c…` (main effects); `MEANS effect`; ANOVA table + **Type I** (sequential) / **Type III** (sum-to-zero effect coding, SAS-matching for unbalanced+interaction) SS, fit statistics, reference-cell parameter estimates (last level = 0 / "B", interaction cross-labels), LS means with SE, Contrasts (F/Pr>F), Estimates (t/Pr>\|t\|) | `LSMEANS` comparison/adjust (Tukey/Dunnett), `ESTIMATE`/`CONTRAST` on interaction terms, Type II/IV SS, continuous covariates, `BY` groups |
| `LOGISTIC` | 🟡 | `DATA=`; `CLASS` (reference coding, ref=last); `MODEL y(DESCENDING EVENT='val') = effects / LINK=LOGIT\|CLOGLOG\|PROBIT NOPRINT`; `FREQ var`; binary logistic + **ordinal proportional-odds (cumulative logit)** for >2 ordered levels (shared slope, ordered intercepts); Newton-Raphson MLE; Class Level Information; Model Fit Statistics (AIC/SC/-2LogL), Global Null tests (LR/Score/Wald), Analysis of ML Estimates (β/SE/Wald χ²/p), Odds Ratio Estimates (logit links); `OUTPUT OUT= PREDICTED=/P=/XBETA=` | EFFECT coding (PARAM=REF only), nominal/generalized-logit multinomial, Score Test for Proportional Odds (deferred), `BY`, `SCORE`, `UNITS`, `ROC` |
| `GENMOD` | 🟡 | `DATA=`; `FREQ var`; `CLASS` (reference coding, ref=last); `MODEL y(DESCENDING EVENT='val') = effects / DIST= LINK= SCALE= NOSCALE NOPRINT`; DIST=POISSON (LOG), BINOMIAL (LOGIT), NORMAL (IDENTITY), **GAMMA** (canonical reciprocal or LINK=LOG, V(μ)=μ²); NR/IRLS MLE (GCONV=1e-8, μ-domain step-halving); Class Level Information; Criteria For Assessing Goodness Of Fit (Deviance/Scaled Deviance/Pearson/LL/AIC/AICC/BIC); Analysis Of Maximum Likelihood Parameter Estimates (β/SE/Wald 95% CI/Wald χ²/p, reference level DF 0); Scale parameter (fixed=1 Poisson/Binomial, √MSE Normal, Gamma Pearson-dispersion 1/φ̂ form); `SCALE=`/`NOSCALE` fix the dispersion; Response Profile (Binomial only) | exact ML (digamma) Gamma scale (Pearson approximation used), multinomial, GEE/`REPEATED`, `OFFSET=`, `ESTIMATE`/`CONTRAST`, `BY`, `OUTPUT OUT=` |
| `PRINCOMP` | 🟡 | `DATA=`; `VAR var1 var2 …`; `N=k` (truncate display); `COV` (covariance instead of correlation); `OUT=` **component scores** (input cols + `Prin1..Prink`, score variance = eigenvalue, standardized/centered per COV); Simple Statistics; Correlation/Covariance Matrix; Eigenvalues table; Eigenvectors; deterministic sign convention | `PARTIAL`, `WEIGHT`, `TYPE=CORR` input datasets, `OUTSTAT=`, `BY` groups |
| `FACTOR` | 🟡 | `DATA=`; `VAR var1 var2 …`; `NFACTORS=k` (or Kaiser MINEIGEN λ>1 default); `METHOD=PRINCIPAL` (default); `ROTATE=VARIMAX`/`QUARTIMAX`/`NONE` (orthogonal) or **`ROTATE=PROMAX`** (oblique: Procrustes power target → Rotated Factor Pattern + Inter-Factor Correlations); `COV`; `OUT=` **factor scores** (regression method, input cols + `Factor1..Factorm`); Prior Communality ONE; Eigenvalues; Factor Pattern; Variance Explained; Final Communality Estimates; Rotated pattern | `METHOD=ML/ITER`, `ROTATE=OBLIMIN` (deferred NOTE), `HEYWOOD`, `ALPHA`, `SCORE`, `BY` groups |
| `DISTANCE` | 🟡 | `DATA=`; `VAR var1 var2 …`; `OUT=ds` (distance matrix dataset); `METHOD=EUCLID/L2` (default), `CITYBLOCK/L1`, `LINF/CHEBYCHEV`, `COSINE`, `CORR`; Distance Matrix listing (Row/Col labeled); output `_TYPE_=DISTANCE` dataset | `SHAPE=`, `FREQ`, normalization options, `ID` variable for row labels |
| `CLUSTER` | 🟡 | `DATA=`; `VAR var1 var2 …`; `METHOD=WARD` (default), `AVERAGE`, `SINGLE`, `COMPLETE`; `ID var`; `OUTTREE=` **dendrogram dataset** (`_NAME_/_PARENT_/_NCL_/_FREQ_/_HEIGHT_` + leaf VAR coords; `_HEIGHT_`=1−RSQ monotone); Cluster History (NClusters, Clusters Joined, Freq, SPRSQ, RSQ); Lance-Williams update | `PSEUDO=`, `NOEIGEN`, `CCC`, graphical dendrogram |
| `FASTCLUS` | 🟡 | `DATA=`; `VAR var1 var2 …`; `MAXCLUSTERS=k` (required); `OUT=ds` (with `_CLUSTER_` variable); `MAXITER=`; `CONVERGE=`; farthest-first seed selection; Cluster Summary (Freq/RMS Std/Max Distance/Nearest Cluster); Statistics for Variables (R-Square); `ID var` | `SEED=` (specific seed obs), `RADIUS=`, `DISTANCE`, fuzzy clustering, `MEAN` |
| `DISCRIM` | 🟡 | `DATA=`; `CLASS var`; `VAR var1 var2 …`; `ID var`; `OUT=ds` (`_FROM_`, `_INTO_`, `_<k>` posteriors); `PRIORS EQUAL` (default) / `PROPORTIONAL`; `POOL=YES` (LDA); Class Level Information; Within-Class Covariance Matrices; Pooled Covariance; Pairwise D² (Mahalanobis²); Linear Discriminant Function Coefficients; Classification Results (obs-by-obs + posteriors); Error Count Estimates | `POOL=NO` (QDA), `CROSSVALIDATE`, `OUTSTAT=`, `METHOD=NPAR/KERNEL`, `THRESHOLD=`, `BY` groups |
| `MIXED` | 🟡 | `DATA=`; `METHOD=REML` (default) / `ML`; `CLASS var1 …`; `MODEL effects = / NOINT SOLUTION` (general fixed-effects design: intercept, continuous, CLASS reference coding); `RANDOM INTERCEPT / SUBJECT=var TYPE=VC\|CS`; `REPEATED effect / SUBJECT=var TYPE=VC\|CS\|AR(1)\|UN`; estimation via closed-form (legacy VC single random intercept) or **general (RE)ML optimisation** (Nelder-Mead + restarts + coordinate polish) over the V(θ)=ZGZ'+R covariance; Covariance Parameter Estimates (`UN(i,j)`, `AR(1)`, `Residual`); Fixed-effects solution (β̂/SE/t/df/p) with Contain df; 8 listing sections | `RANDOM` slopes / multiple random effects; `TYPE=` other than VC/CS/AR(1)/UN; `LSMEANS`; `ESTIMATE`; `CONTRAST`; `COVTEST`; Kenward-Roger/Satterthwaite df; `BY` groups |
| `GLIMMIX` | 🟡 | `DATA=`; `METHOD=RSPL` (default) / **`LAPLACE`** (single random intercept, true ML); `CLASS var1 …`; `MODEL effects[(event='val')] = … / NOINT DIST=NORMAL\|POISSON\|BINARY LINK=IDENTITY\|LOG\|LOGIT\|PROBIT\|CLOGLOG SOLUTION` (general fixed-effects design: intercept/continuous/CLASS reference coding); `RANDOM INTERCEPT / SUBJECT=var TYPE=VC`; `REPEATED … / SUBJECT=var TYPE=VC\|CS\|AR(1)\|UN` (R-side, RSPL); `FREQ var`; RSPL/PQL (Breslow-Clayton); NORMAL/IDENTITY+random ≡ REML/ML (exact); Poisson/Binary no-random ≡ GENMOD/LOGISTIC; PROBIT/CLOGLOG no-random ≡ LOGISTIC links; LAPLACE Normal+random ≡ MIXED ML (cross-validated); Generalized Chi-Square; Type III Tests; Solutions for Fixed Effects | `METHOD=QUAD` (deferral NOTE); `DIST=GAMMA`; `LAPLACE` with AR(1)/UN/multiple-random (NOTE); `RANDOM` slopes; `LSMEANS`; `ESTIMATE`; `CONTRAST`; `WEIGHT`; `BY` groups |
| `IML` | 🟡 | Sub-language (own lexer/parser/evaluator). Matrix literals `{1 2, 3 4}`, `{"x" "y"}`; indexing `A[i,j]`/`A[i,*]`; operators `'` (transpose), `*` (matmul), `#` (Hadamard), `@` (Kronecker), `+ - /`, comparisons; `NROW`/`NCOL`/`DIM`/`T`; stats `SUM`/`MEAN`/`STD`/`MIN`/`MAX`/`ABS`/`SQRT`/`EXP`/`LOG`; control flow `IF/THEN/ELSE`, `DO i=a TO b [BY c]`, `DO WHILE/UNTIL`; `PRINT`; linear algebra `INV`, `SOLVE`, `EIGVAL` (symmetric), `CHOL` (upper U), `CALL QR(Q,R,A)`, `CALL SVDCD(U,D,V,A)`; I/O `CREATE ds FROM mat[COLNAME=]`, `APPEND FROM`, `CLOSE`, `USE`, `READ ALL VAR {..} INTO mat`; `SHAPE(x,nr[,nc])` (row-major reshape + recycling); range subscripts `A[1:2,1:3]`/`A[2:3,*]`; `DET`; `EIGVEC` + `CALL EIGEN(val,vec,A)` (symmetric, descending) | `READ NEXT`, `WHERE`, `LOAD`/`STORE`/`SHOW`, modules (`START`/`FINISH`) |
| `GPLOT` | 🟡 | `DATA=`; `PLOT y*x` / `PLOT y*x=group` / `PLOT (y1 y2)*x`. Without `--features graphics`: NOTE "image deferred". With `--features graphics`: PNG/SVG via `gplot_{N}` with **multi-series overlay** (one series per Y var / per group level); `SYMBOL`n (INTERPOL=JOIN→line, VALUE=→marker, COLOR=) and `AXIS`n (ORDER=, LABEL=) honored | SYMBOL HEIGHT/WIDTH/LINE/REPEAT; AXIS log/discrete & tick formatting; PLOT2 second axis; `=group` combined with multiple Y; `VPLOT`; `BY` |
| `GCHART` | 🟡 | `DATA=`; `VBAR`/`HBAR cat / SUMVAR= TYPE=FREQ\|SUM\|MEAN`; `PIE cat / SUMVAR= TYPE=`. Without `--features graphics`: NOTE "image deferred". With `--features graphics`: VBar and **PIE** charts via `gchart_{N}` (slices proportional to FREQ/SUM/MEAN) | `SUBGROUP=`; HBAR rendering; `BY` |
| `PLOT` | 🟡 | `DATA=`; `PLOT y*x` / `PLOT y*x='sym'` / `PLOT (y1 y2)*x` / `PLOT y*x=group`. When ODS GRAPHICS OFF: ASCII scatter in listing (20×60 grid, A/B/C overlaps, labelled axes). When ODS GRAPHICS ON: delegates to image (`plot_{N}`) | `HREF=`/`VREF=`; `HAXIS=`/`VAXIS=`; multiple plots in one grid; `BY` |
| `SGPLOT` | 🟡 | `DATA=`; statements `SCATTER x= y= / GROUP= MARKERATTRS=()`, `SERIES x= y=`, `VBAR`/`HBAR cat / RESPONSE= STAT=FREQ\|SUM\|MEAN`, `HISTOGRAM var / BINWIDTH= SCALE=`, `DENSITY`, `VBOX resp / CATEGORY=`, `REG x= y= / DEGREE=`, `LOESS x= y= / SMOOTH=`, `XAXIS`/`YAXIS LABEL= VALUES=(min to max by step) TYPE=LINEAR\|LOG\|DISCRETE`, `BY`. Without `--features graphics`: NOTE "image deferred", byte-identical default build. With `--features graphics`: PNG/SVG via `plotters`, sequential naming `{IMAGENAME\|sgplot}_{N}.{ext}`; **LOESS** (tricube local-linear smoother, SMOOTH=) and **DENSITY** (NORMAL/KERNEL) rendered as overlays; histograms as real binned bars; XAXIS/YAXIS VALUES= → forced ranges | `HBAR`/`VBOX`/`REG` rendering (parse-only under graphics → NOTE); `MARKERATTRS=`/`LINEATTRS=` ignored; multi-plot overlays beyond primary+LOESS/DENSITY; legends; `BY`-group images |

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
| Control flow | ✅ | `%IF/%THEN/%ELSE`, `%DO/%END`, `%DO i=a %TO b %BY c`, `%DO %WHILE`, `%DO %UNTIL`, `%RETURN`, `%GOTO`/`%label:`, `%ABORT` (`CANCEL`/`ABEND`/`RETURN [n]`) |
| Evaluation | ✅ | `%EVAL`, `%SYSEVALF`, `%SYSFUNC`/`%QSYSFUNC` (full DATA step function library — no whitelist — with optional trailing `format.`) |
| Quoting | 🟡 | `%STR`, `%NRSTR`, `%UNQUOTE`, `%CMPRES`, `%QCMPRES` (no `%SUPERQ`/`%BQUOTE`/`%NRBQUOTE`) |
| Utilities | ✅ | `%PUT`, `%INCLUDE` (quoted path, fileref via `FILENAME`, non-quoted path) + autocall (`SASAUTOS`), `%SYMEXIST`, `%SYSMEXIST`, `%SYSGET` |
| Automatic vars | ✅ | `&SYSDATE(9)`, `&SYSTIME`, `&SYSDAY`, `&SYSDAYNUM`, `&SYSMONTH`, `&SYSYEAR`, `&SYSVER`, `&SYSSCP(L)`; status codes `&SYSCC`/`&SYSERR`/`&SYSRC`/`&SQLOBS`/`&SQLRC`; `&SYSLAST` (live, last dataset); env info `&SYSPROCESSNAME`, `&SYSENV`, `&SYSUSERID`, `&SYSHOSTNAME`, … |
| Tracing | ✅ | `MPRINT`, `MLOGIC`, `SYMBOLGEN` |
| Unsupported (clean NOTE) | 🔴 | `%SYSEXEC` (OS command), `%WINDOW`/`%DISPLAY` (interactive), `%SYSCALL`, `%SYSMACDELETE`, `%SYSMSTORECLEAR`, `%SYSLPUT`/`%SYSRPUT` — consumed with a "not supported in this build" NOTE; unknown `%keyword` left verbatim (SAS behaviour) |

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
| `%INCLUDE` | 🟡 | quoted paths, **filerefs** (via `FILENAME`), **non-quoted paths**, autocall (`SASAUTOS`); `*`/stdin deferred (NOTE) |
| `FILENAME` | 🟡 | `FILENAME ref 'path';` / `ref path;` → fileref registry for `%INCLUDE` (resolved like LIBNAME/SASAUTOS); device/pipe/URL forms noted & ignored |
| `X` | 🔴 | not supported |

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
