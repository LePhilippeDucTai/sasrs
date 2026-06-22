/* M34.7 : PROC GENMOD — DIST=GAMMA with a CLASS predictor (LINK=LOG).
   A single 2-level CLASS factor saturates the mean, so the fitted group means
   equal the observed group means and the estimates are exact:

   Group A (y = 2,4,6)   → mean 4
   Group B (y =10,20,30) → mean 20
   Reference = LAST level in sas_cmp order = B.
     Intercept = ln(20)            = 2.9957
     grp A     = ln(4) - ln(20)    = ln(0.2) = -1.6094

   Class Level Information lists grp (Levels 2, Values A B). */
title 'PROC GENMOD: Gamma regression with CLASS predictor';

data g;
  input grp $ y;
  datalines;
A 2
A 4
A 6
B 10
B 20
B 30
;

proc genmod data=g;
  class grp;
  model y = grp / dist=gamma link=log;
run;

title;
