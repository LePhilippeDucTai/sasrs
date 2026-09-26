/* PROC TTEST — test t à deux échantillons (pooled + Satterthwaite), OUT=.
   Cf. SAS/STAT 9.4 — The TTEST Procedure. Statistiques recalculées
   indépendamment en Python 3. */
libname ind 'data';

proc ttest data=ind.grow;
  class group;
  var height;
  output out=tt;
run;
