/* PROC NPAR1WAY WILCOXON — test de Wilcoxon à 2 échantillons, OUT=.
   Cf. SAS/STAT 9.4 — The NPAR1WAY Procedure (OUT= Data Set). Statistique Z
   avec correction de liens et p-value normale recalculées indépendamment
   en Python 3. */
libname ind 'data';

proc npar1way data=ind.wilks wilcoxon;
  class group;
  var response;
  output out=wk;
run;
