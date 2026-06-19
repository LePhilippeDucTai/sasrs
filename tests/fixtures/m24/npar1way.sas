/* M24.3 : PROC NPAR1WAY — Wilcoxon + Kruskal-Wallis sur sashelp.class */
libname d 'data';

/* 2-sample Wilcoxon rank-sum + Kruskal-Wallis par sex */
title 'PROC NPAR1WAY: height and weight by sex';
proc npar1way data=d.class wilcoxon;
  class sex;
  var height weight;
run;
title;
