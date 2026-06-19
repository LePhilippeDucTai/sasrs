title 'PROC MIXED: random intercept model, REML and ML';
data work.balanced;
  input subj $ y;
datalines;
A 1
A 3
B 5
B 7
;
/* REML (default) */
proc mixed data=work.balanced;
  class subj;
  model y = / solution;
  random intercept / subject=subj type=vc;
run;

/* ML */
proc mixed data=work.balanced method=ml;
  class subj;
  model y = / solution;
  random intercept / subject=subj type=vc;
run;
title;
