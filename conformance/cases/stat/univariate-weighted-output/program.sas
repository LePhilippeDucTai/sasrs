/* PROC UNIVARIATE pondérée (WEIGHT) — moyenne et écart-type pondérés.
   Cf. SAS/Base 9.4 — The UNIVARIATE Procedure (WEIGHT Statement). */
libname ind 'data';

proc univariate data=ind.grade noprint;
  var score;
  weight count;
  output out=ws n=n mean=wmean std=wstd;
run;
