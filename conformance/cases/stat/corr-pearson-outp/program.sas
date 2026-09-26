/* PROC CORR — corrélation de Pearson, dataset OUTP=.
   Cf. SAS/Base 9.4 — The CORR Procedure (OUTP= Data Set). */
libname ind 'data';

proc corr data=ind.htwt outp=pc;
  var height weight;
run;
