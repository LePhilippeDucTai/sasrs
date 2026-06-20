/* M33.7 : PROC CONTENTS — options differees OUT= / SHORT / DETAILS.
   Donnees : sashelp.class (name sex age height weight).

   1) OUT=meta ecrit une ligne par variable (5 variables -> 5 obs) avec les
      colonnes NAME TYPE LENGTH VARNUM LABEL FORMAT (sous-ensemble documente du
      OUT= de SAS). TYPE : 1=numerique, 2=caractere (convention SAS).
      sashelp.class : name (Char), sex (Char), age/height/weight (Num).
      On verrouille la forme du dataset OUT= via PROC PRINT.

   2) DETAILS ajoute deux lignes au bloc d'en-tete (# Observations / # Variables).

   3) SHORT n'imprime qu'une liste a plat des noms (ordre alphabetique par
      defaut). */
libname d 'data';

data class;
  set d.class;
run;

title 'CONTENTS OUT= : one row per variable (locked via PROC PRINT)';
proc contents data=class out=meta;
run;

proc print data=meta;
run;

title 'CONTENTS DETAILS : extra observation/variable lines in the header';
proc contents data=class details;
run;

title 'CONTENTS SHORT : flat variable-name list only';
proc contents data=class short;
run;
