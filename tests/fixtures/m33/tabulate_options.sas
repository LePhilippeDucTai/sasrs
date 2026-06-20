/* M33.4 : PROC TABULATE — options differees (labels d'en-tete, FORMAT= / *f=,
   OUT= dataset de cellules). Donnees : sashelp.class (name sex age height
   weight). Toutes les valeurs sont verifiables a la main.

   sashelp.class, par sexe :
     F (9) heights : 56.5 65.3 62.8 59.8 62.5 51.3 64.3 56.3 66.5
        sum = 545.3  mean = 545.3/9 = 60.588888...
     M (10) heights: 69.0 63.5 57.3 62.5 59.0 72.0 64.8 67.0 57.5 66.5
        sum = 639.1  mean = 639.1/10 = 63.91

   1) Labels d'en-tete : sex='Gender' (accepte ; les niveaux restent F/M dans
      le modele plat), mean='Average height' remplace le libelle de stat.
   2) FORMAT= 8.2 (defaut de table) + *f=6.1 par cellule.
   3) OUT= : un dataset de cellules ; PROC PRINT verrouille sa forme.
        Colonnes : sex _TYPE_ _PAGE_ _TABLE_ height_Mean. */
libname d 'data';

title 'Labelled + table-level formatted means (FORMAT=8.2)';
proc tabulate data=d.class format=8.2;
  class sex;
  var height;
  table sex='Gender', height*mean='Average height';
run;

title 'Per-cell format *f=6.1 overrides on weight sum';
proc tabulate data=d.class;
  class sex;
  var weight;
  table sex, weight*sum*f=6.1;
run;

title 'OUT= cell dataset, then PROC PRINT to lock its shape';
proc tabulate data=d.class out=cells;
  class sex;
  var height;
  table sex, height*mean;
run;

proc print data=cells;
run;
