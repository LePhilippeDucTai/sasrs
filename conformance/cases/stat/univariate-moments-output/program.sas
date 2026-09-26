/* PROC UNIVARIATE — moments (moyenne, écart-type, asymétrie, kurtosis)
   via OUTPUT OUT=.
   Cf. SAS/Base 9.4 — The UNIVARIATE Procedure (OUTPUT Statement). */
libname ind 'data';

proc univariate data=ind.scores noprint;
  var score;
  output out=mom n=n mean=mean std=std skewness=skewness kurtosis=kurtosis
         min=min max=max;
run;
