/* M34.3 : PROC NPAR1WAY — BY-group processing and OUTPUT OUT= dataset.
   sashelp.class split into age groups; each group keeps both sexes so the
   CLASS variable has 2 levels within every BY group. */
libname d 'data';

data class2;
  set d.class;
  if age >= 14 then agegrp = 'Old';
  else agegrp = 'Yng';
run;

proc sort data=class2 out=class_s;
  by agegrp;
run;

title 'NPAR1WAY BY agegrp: Wilcoxon of height by sex';
proc npar1way data=class_s wilcoxon;
  by agegrp;
  class sex;
  var height;
run;

title 'NPAR1WAY OUT= (default Wilcoxon + Kruskal-Wallis), height & weight';
proc npar1way data=d.class;
  class sex;
  var height weight;
  output out=npout;
run;

proc print data=npout noobs;
run;
title;
