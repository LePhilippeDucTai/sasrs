/* M34.6 : PROC LOGISTIC — CLASS predictor + OUTPUT OUT=.
   Same 2x2 counts as m26 (saturated → fitted p = cell proportions):
     x=1 : 20 events / 25  → p = 0.8
     x=0 : 10 events / 35  → p = 0.285714
   With x as a CLASS factor (ref = last level = '1'), the slope for level '0'
   vs '1' is logit(0.2857) - logit(0.8) = ln(0.4) - ln(4) = -ln(10), i.e. the
   odds ratio '0 vs 1' = 0.1 (and equivalently '1 vs 0' = 10, matching m26's
   continuous-x OR = 10). OUTPUT writes the predicted event probability. */
title 'PROC LOGISTIC: CLASS predictor with OUTPUT OUT=';

data counts;
  input y x count;
  datalines;
1 1 20
1 0 10
0 1  5
0 0 25
;

proc logistic data=counts;
  class x;
  model y(descending) = x;
  freq count;
  output out=pred predicted=phat;
run;

proc print data=pred noobs;
run;

title;
