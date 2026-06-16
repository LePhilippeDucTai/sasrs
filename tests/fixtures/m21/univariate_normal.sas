/* M21.3 : PROC UNIVARIATE — Tests for Normality (Shapiro-Wilk, KS,
   Cramer-von Mises, Anderson-Darling) + graphical statements deferred
   to ODS GRAPHICS (M29). */
libname d 'data';

title 'Univariate normality tests on height';
proc univariate data=d.class normal;
  var height;
  histogram height / normal;
  qqplot height;
run;
