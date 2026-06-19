title 'PROC FACTOR: principal components, one factor retained';
data work.corr2;
  input x y;
datalines;
1 2
2 3
3 3
4 5
5 4
;
proc factor data=work.corr2;
  var x y;
run;
title;
