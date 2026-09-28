/* J08-P2 : PROC CORR avec BY (donnees triees par sexe).
 * Un bloc Simple Statistics + une matrice Pearson par groupe BY,
 * en-tete de groupe facon SAS, et OUT= TYPE=CORR portant sex. */
libname d 'data';

proc sort data=d.class out=class;
  by sex;
run;

proc corr data=class outp=work.corrout;
  by sex;
  var height weight;
run;

proc print data=work.corrout noobs;
run;
