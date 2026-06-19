title 'PROC SGPLOT: basic scatter and histogram';

data work.heights;
  input age height;
datalines;
10 140
12 150
14 158
16 165
18 170
20 172
25 175
30 176
35 175
40 174
;

/* Test 1: SGPLOT sans ODS GRAPHICS ON — note de non-activation */
proc sgplot data=work.heights;
  scatter x=age y=height;
run;

/* Test 2: activer ODS GRAPHICS puis SGPLOT */
ods graphics on;

proc sgplot data=work.heights;
  scatter x=age y=height;
  xaxis label='Age (years)';
  yaxis label='Height (cm)';
run;

proc sgplot data=work.heights;
  histogram height / binwidth=10;
run;

ods graphics off;

title;
