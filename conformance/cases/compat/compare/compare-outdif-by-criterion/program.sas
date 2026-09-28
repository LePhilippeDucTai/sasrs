/* PROC COMPARE — BY, METHOD=ABSOLUTE, CRITERION=, OUT= OUTDIF OUTNOEQUAL.
   Cf. SAS 9.4 Procedures Guide, COMPARE Procedure : avec METHOD=ABSOLUTE,
   deux valeurs sont inegales si |y - x| > CRITERION ; OUTNOEQUAL ne garde
   que les observations jugees inegales ; OUTDIF ecrit la difference ;
   l'appariement se fait par BY groupe puis par position. */
libname b 'data';

proc compare base=b.b2 compare=b.c2 out=out
             outdif outnoequal method=absolute criterion=0.001 noprint;
  by grp;
  var v;
run;
