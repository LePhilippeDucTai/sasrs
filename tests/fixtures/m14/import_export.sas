/* M14.3 : PROC IMPORT / PROC EXPORT via LIBNAME CSV.                        */
/* Le répertoire data/ est crée par le harnais (base_dir = <tmp>).          */
/* On exporte d.class vers CSV puis on le réimporte via LIBNAME CSV.        */
libname d 'data';
libname csv1 csv 'data';

* Filtrer et écrire dans la bibliothèque CSV via un DATA step. ;
data csv1.aged14;
  set d.class;
  if age >= 14;
  keep name age height;
run;

* Lire la table CSV avec LIBNAME CSV. ;
title 'Elèves de 14 ans et plus (via LIBNAME CSV)';
proc print data=csv1.aged14;
run;

* Copier en WORK pour vérification. ;
data work.check;
  set csv1.aged14;
run;

title 'Copie dans WORK';
proc print data=work.check;
run;

libname csv1 clear;
