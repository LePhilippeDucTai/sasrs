/* M33.6 : PROC PRINT — options differees BY / ID / SUM / DOUBLE / N.
   Donnees : sashelp.class (name sex age height weight).

   Oracles (calcules a la main) :
     Sommes globales : height = 1184.4, weight = 1900.5  (n=19).
     Par sexe (apres tri sur sex) :
       F (9 obs) : height = 545.3,  weight = 811.0
       M (10 obs): height = 639.1,  weight = 1089.5
     Grand total (BY + SUM) : height = 1184.4, weight = 1900.5.

   On trie d'abord sur sex (BY exige une entree triee). */
libname d 'data';

proc sort data=d.class out=class;
  by sex;
run;

title 'PRINT with SUM + DOUBLE + N (no BY)';
proc print data=class double n;
  var name sex height weight;
  sum height weight;
run;

title 'PRINT with BY sex, ID name, SUM per group + grand total, N per group';
proc print data=class n;
  by sex;
  id name;
  var age height weight;
  sum height weight;
run;
