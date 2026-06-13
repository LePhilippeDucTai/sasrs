/* M9 : PROC CORR (Pearson). Stats simples + matrice ; puis WITH + NOSIMPLE/NOPROB. */
libname d 'data';

title 'Pearson correlations among height, weight, age';
proc corr data=d.class;
  var height weight age;
run;

title 'CORR with WITH, no simple statistics, no probabilities';
proc corr data=d.class nosimple noprob;
  var height weight;
  with age;
run;
