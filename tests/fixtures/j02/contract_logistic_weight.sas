data t;
  input x y w;
  datalines;
1 0 1
1 1 2
2 0 3
2 1 4
;
run;
proc logistic data=t;
  model y=x;
  weight w;
run;
