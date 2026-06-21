/* M34.6 : PROC LOGISTIC — ordinal (proportional-odds cumulative logit).
   A 3-level ordered response y with one continuous predictor x. The data
   deliberately OVERLAP (each x region carries a mix of response levels) so the
   MLE converges to finite estimates rather than separating.

   Structural oracle: Response Profile orders the three levels (1,2,3); the model
   line reads "cumulative logit"; the two Intercept rows are strictly increasing
   (Intercept 1 < Intercept 2, the SAS logit[P(Y<=j)] convention) and the shared
   slope is finite (OR = exp(slope)). The exact MLE values are SAS-plausible /
   structurally verified rather than closed-form. (Score Test for Proportional
   Odds is deferred — NOTE.) */
title 'PROC LOGISTIC: ordinal proportional-odds model';

data ord;
  input y x;
  datalines;
1 1
2 1
3 1
1 2
2 2
3 2
1 3
2 3
3 3
2 4
3 4
1 4
3 5
2 5
;

proc logistic data=ord;
  model y = x;
run;

title;
