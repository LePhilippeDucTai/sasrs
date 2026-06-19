title 'M30.1: PROC GPLOT + PROC GCHART (legacy graphics)';

data work.xy;
  input x y;
datalines;
1 2
2 4
3 3
4 5
5 6
;

data work.cats;
  input category $ count;
datalines;
A 10
B 25
C 15
D 30
;

/* Test 1: GPLOT sans ODS ON */
proc gplot data=work.xy;
  plot y*x;
run;

/* Test 2: avec ODS ON */
ods graphics on;

proc gplot data=work.xy;
  plot y*x;
run;

proc gchart data=work.cats;
  vbar category / sumvar=count;
  pie category;
run;

ods graphics off;

title;
