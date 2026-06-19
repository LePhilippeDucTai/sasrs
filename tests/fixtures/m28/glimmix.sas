title 'PROC GLIMMIX: Poisson, Binary, Normal+random (cross-checks)';

data work.pois;
  input y x;
datalines;
1 0
2 0
3 0
4 1
5 1
6 1
;

data work.counts;
  input y x count;
datalines;
1 1 20
1 0 10
0 1  5
0 0 25
;

data work.balanced;
  input subj $ y;
datalines;
A 1
A 3
B 5
B 7
;

/* Oracle 2: Poisson sans random — cross-check GENMOD */
proc glimmix data=work.pois;
  model y = x / dist=poisson link=log solution;
run;

/* Oracle 3: Binary + FREQ sans random — cross-check LOGISTIC */
proc glimmix data=work.counts;
  model y(event='1') = x / dist=binary link=logit solution;
  freq count;
run;

/* Oracle 1: Normal + random — cross-check MIXED */
proc glimmix data=work.balanced;
  class subj;
  model y = / dist=normal link=identity solution;
  random intercept / subject=subj type=vc;
run;

title;
