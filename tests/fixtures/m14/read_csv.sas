/* M14.4 : LIBNAME CSV — bibliothèque virtuelle fichier-table.               */
/* Ecrit des données via LIBNAME CSV puis les relit.                         */
libname csv1 csv 'data';

* Créer une table via DATALINES puis la copier dans la bibl. CSV. ;
data work.grades;
  input student $ grade;
  datalines;
Alice 90
Bob 75
Carol 88
Diana 92
;
run;

data csv1.grades;
  set work.grades;
run;

* Vérifier l'existence et relire. ;
title 'Lu via LIBNAME CSV (csv1.grades)';
proc print data=csv1.grades;
run;

* SET direct depuis la bibliothèque CSV. ;
data work.copy;
  set csv1.grades;
run;

title 'Copie dans WORK depuis CSV';
proc print data=work.copy;
run;

libname csv1 clear;
