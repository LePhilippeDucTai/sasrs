/* PROC CORR SPEARMAN — corrélation des rangs, dataset OUTS=.
   Cf. SAS/Base 9.4 — The CORR Procedure (SPEARMAN, OUTS= Data Set). */
libname ind 'data';

proc corr data=ind.htwt spearman outs=sc;
  var height weight;
run;
