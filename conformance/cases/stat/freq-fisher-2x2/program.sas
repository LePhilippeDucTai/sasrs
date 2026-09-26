/* PROC FREQ — test exact de Fisher sur table 2x2 pondérée, OUTPUT FISHER.
   Cf. SAS/Base 9.4 — The FREQ Procedure (EXACT Fisher, OUTPUT). p-value
   exacte recalculée indépendamment en Python 3 (hypergéométrique). */
libname ind 'data';

proc freq data=ind.twoway;
  weight n;
  tables treatment*result / fisher;
  output out=fs fisher;
run;
