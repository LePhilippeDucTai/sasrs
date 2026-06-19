/* M24.2 : PROC TTEST — 1-sample et 2-sample par sex sur sashelp.class */
libname d 'data';

/* 1-sample : height vs H0=60 (hypothese nul = taille moyenne = 60 pouces) */
title 'PROC TTEST 1-sample: height vs H0=60';
proc ttest data=d.class h0=60;
  var height;
run;

/* 2-sample : height et weight par sex (Pooled + Satterthwaite + F-test) */
title 'PROC TTEST 2-sample: height and weight by sex';
proc ttest data=d.class;
  class sex;
  var height weight;
run;
title;
