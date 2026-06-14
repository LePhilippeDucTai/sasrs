/* M15.7: LAG/LAGn/DIF/DIFn — lagged and differenced values per call site. */

data work.test;
  input x;

  /* LAG/LAG1 — value from previous call */
  lag_x = lag(x);

  /* LAG2 — value from two calls ago */
  lag2_x = lag2(x);

  /* DIF/DIF1 — difference from previous (x - lag(x)) */
  dif_x = dif(x);

  /* DIF2 — second difference (x - lag2(x)) */
  dif2_x = dif2(x);

  output;

  datalines;
1
2
3
4
5
;
run;

title 'M15.7 LAG/DIF Functions';
proc print data=work.test;
run;
