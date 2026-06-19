title 'PROC GENMOD: Poisson, Binomial, Normal';

/* 1. Poisson: oracle ferme */
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

proc genmod data=work.pois;
  model y = x / dist=poisson link=log;
run;

/* 2. Binomial: cross-check LOGISTIC */
data work.counts;
  input y x count;
  datalines;
1 1 20
1 0 10
0 1  5
0 0 25
;

proc genmod data=work.counts;
  model y(descending) = x / dist=binomial link=logit;
  freq count;
run;

/* 3. Normal: cross-check OLS */
proc genmod data=work.pois;
  model y = x / dist=normal link=identity;
run;

title;
