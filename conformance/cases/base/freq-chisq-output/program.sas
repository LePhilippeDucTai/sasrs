/* PROC FREQ CHISQ sur table 2x2 — cf. PROC FREQ, Chi-Square Tests.
   Tableau pondéré par n : [[8,2],[4,6]]. */
libname ind 'data';

proc freq data=ind.twoway;
  weight n;
  tables treatment*result / chisq;
  output out=stats chisq;
run;
