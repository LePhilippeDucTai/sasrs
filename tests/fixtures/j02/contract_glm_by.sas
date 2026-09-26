data t;
  input g x y;
  datalines;
1 1 2
1 2 4
2 1 8
2 2 16
;
run;
proc glm data=t;
  model y=x;
  by g;
run;
