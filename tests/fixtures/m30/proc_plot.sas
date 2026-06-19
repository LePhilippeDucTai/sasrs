title 'M30.2: PROC PLOT (ASCII scatter)';

data work.xy;
  input x y;
datalines;
1 10
2 20
3 15
4 30
5 25
6 40
;

/* Test 1: PLOT ASCII (sans ODS ON) */
proc plot data=work.xy;
  plot y*x;
run;

/* Test 2: avec ODS ON — délègue à image */
ods graphics on;
proc plot data=work.xy;
  plot y*x='*';
run;
ods graphics off;

title;
