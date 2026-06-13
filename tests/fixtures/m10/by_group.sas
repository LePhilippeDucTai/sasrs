/* M10 : MEANS avec BY (donnees triees par sexe). */
libname d 'data';

proc sort data=d.class out=class;
  by sex;
run;

title 'Mean height and weight by sex';
proc means data=class;
  by sex;
  var height weight;
run;
