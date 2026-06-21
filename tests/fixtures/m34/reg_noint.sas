/* M34.4 : PROC REG — NOINT (no-intercept) regression on sashelp.class.
   Through-the-origin OLS of weight on height: beta = Σ(xy)/Σ(x²).

   Oracle cross-checks (NOINT uses UNCORRECTED sums of squares):
     - ANOVA third row is "Uncorrected Total" with DF = n = 19 and
       SS = Σ weight²  (NOT the centered Σ(weight-mean)²).
     - R-Square = SSM/SST = Σŷ²/Σy² (close to 1: weight ≈ k·height), and is
       LARGER than the intercept-model R² because the uncorrected total is huge.
     - Parameter Estimates has NO Intercept row — only `height`, DF=1.
     - Model DF = 1, Error DF = n - 1 = 18. */
libname d 'data';

title 'PROC REG NOINT: weight through the origin on height';
proc reg data=d.class;
  model weight = height / noint;
run;

title;
