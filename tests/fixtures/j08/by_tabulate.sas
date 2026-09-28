/* J08-P2 : PROC TABULATE avec BY (donnees triees par sexe).
 * Une table par groupe BY, en-tete de groupe facon SAS, et OUT=
 * dont les lignes portent la variable BY. */
libname d 'data';

proc sort data=d.class out=class;
  by sex;
run;

proc tabulate data=class out=work.tabout;
  class sex;
  var height weight;
  by sex;
  table sex, height*mean weight*mean;
run;

proc print data=work.tabout noobs;
run;
