title 'PROC LOGISTIC: binary outcome by binary predictor';

data work.counts;
  input y x count;
  datalines;
1 1 20
1 0 10
0 1  5
0 0 25
;

proc logistic data=work.counts;
  model y(descending) = x;
  freq count;
run;

title;
