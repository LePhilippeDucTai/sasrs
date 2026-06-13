/* M5 : PROC MEANS — stats par defaut, CLASS, et OUTPUT OUT= (_TYPE_/_FREQ_). */
libname d 'data';

title 'Means of height and weight by sex';
proc means data=d.class;
  class sex;
  var height weight;
run;

proc means data=d.class noprint;
  class sex;
  var weight;
  output out=work.stats mean(weight)=avg_wt n(weight)=n_wt;
run;

title 'OUTPUT dataset with _TYPE_ / _FREQ_';
proc print data=work.stats;
run;
