/* M39.3 - FMTSEARCH= : ordre de resolution explicite entre le catalogue WORK
   et un libref permanent. WORK et MYLIB definissent tous deux un format
   COLOR pour les memes valeurs, avec un libelle different ; le format
   effectivement applique suit OPTIONS FMTSEARCH=, pas l'ordre des etapes qui
   les ont definis. Inverser FMTSEARCH= inverse le resultat. */

proc format;
  value color 1='Work-Red' 2='Work-Blue';
run;

libname mylib 'data';
proc format library=mylib;
  value color 1='Lib-Red' 2='Lib-Blue';
run;

options fmtsearch=(mylib work);
data t1;
  input c;
  format c color.;
  datalines;
1
2
;
run;

title "FMTSEARCH=(MYLIB WORK) -- MYLIB gagne";
proc print data=t1 noobs;
run;
title;

options fmtsearch=(work mylib);
data t2;
  input c;
  format c color.;
  datalines;
1
2
;
run;

title "FMTSEARCH=(WORK MYLIB) -- WORK gagne";
proc print data=t2 noobs;
run;
title;
