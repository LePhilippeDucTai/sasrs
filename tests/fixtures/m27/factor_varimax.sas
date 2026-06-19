title 'PROC FACTOR: principal components, 2 factors, varimax rotation';
data work.three_vars;
  input x y z;
datalines;
1 2 5
2 4 4
3 3 3
4 5 2
5 1 1
6 6 6
;
proc factor data=work.three_vars nfactors=2 rotate=varimax;
  var x y z;
run;
title;
