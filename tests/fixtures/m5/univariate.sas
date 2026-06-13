/* M5 : PROC UNIVARIATE — moments, quantiles (definition 5), extremes sur height. */
libname d 'data';

title 'Univariate analysis of height';
proc univariate data=d.class;
  var height;
run;
