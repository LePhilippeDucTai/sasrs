/* M34.9 : PROC FACTOR — PROMAX oblique rotation (2 factors) on sashelp.class.
   PROMAX starts from VARIMAX then applies a power target → oblique solution:
   a "Rotated Factor Pattern" plus a non-identity "Inter-Factor Correlations"
   matrix. */
libname d 'data';
title 'PROC FACTOR ROTATE=PROMAX (oblique)';
proc factor data=d.class nfactors=2 rotate=promax;
  var height weight age;
run;
title;
