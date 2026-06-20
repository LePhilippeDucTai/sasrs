/* M33.3 : PROC MEANS / SUMMARY — options differees percentiles / WAYS / TYPES /
   PRINTALLTYPES. Donnees : sashelp.class (name sex age height weight).

   Toutes les valeurs sont verifiables a la main (Definition 5, QNTLDEF=5).

   1) Percentiles. Hauteurs (n=19) triees :
        51.3 56.3 56.5 57.3 57.5 59.0 59.8 62.5 62.5 62.8
        63.5 64.3 64.8 65.3 66.5 66.5 67.0 69.0 72.0
      Def 5 (np=19*p, j=floor, g=np-j ; g=0 -> moyenne x[j],x[j+1] ; sinon x[j+1]) :
        P25 : np=4.75 -> x[5]  = 57.5
        P50 : np=9.5  -> x[10] = 62.8  (= median)
        P75 : np=14.25-> x[15] = 66.5
        P95 : np=18.05-> x[19] = 72.0
        QRANGE = P75-P25 = 9.0
      Poids (n=19) tries :
        50.5 77 83 84 84 84.5 85 90 98 99.5 102.5 102.5 112 112 112.5 112.5 128 133 150
        P25 = x[5]  = 84.0   P50 = x[10] = 99.5   P75 = x[15] = 112.5
        P95 = x[19] = 150.0  QRANGE = 28.5

   2) PRINTALLTYPES avec une CLASS (sex) : par defaut MEANS n'imprime que le
      _TYPE_ le plus eleve (toutes CLASS croisees) ; PRINTALLTYPES imprime aussi
      le _TYPE_=0 (global) puis le _TYPE_=1 (par sexe).

   3) WAYS / TYPES avec deux CLASS (sex age) sur OUTPUT OUT= : WAYS 1 ne retient
      que les _TYPE_ a une seule CLASS active ; TYPES (sex) cible un croisement. */
libname d 'data';

data class;
  set d.class;
run;

title 'Percentiles via Definition 5 (height, weight)';
proc means data=class p25 median p75 p95 qrange;
  var height weight;
run;

title 'PRINTALLTYPES prints every _TYPE_ (default prints only the top type)';
proc means data=class n mean printalltypes;
  class sex;
  var height;
run;

title 'WAYS 1 keeps only single-CLASS _TYPE_ rows in OUTPUT';
proc means data=class noprint;
  class sex age;
  var height;
  ways 1;
  output out=ways1 mean(height)=mh;
run;

proc print data=ways1;
run;

title 'TYPES (sex) targets a specific crossing in OUTPUT';
proc means data=class noprint;
  class sex age;
  var height;
  types (sex);
  output out=types1 mean(height)=mh;
run;

proc print data=types1;
run;
