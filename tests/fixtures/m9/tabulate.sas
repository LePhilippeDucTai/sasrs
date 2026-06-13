/* M9 : PROC TABULATE. Frequence par sexe, puis moyennes height/weight par sexe. */
libname d 'data';

title 'Frequency of sex';
proc tabulate data=d.class;
  class sex;
  table sex;
run;

title 'Mean height and weight by sex';
proc tabulate data=d.class;
  class sex;
  var height weight;
  table sex, height*mean weight*mean;
run;
