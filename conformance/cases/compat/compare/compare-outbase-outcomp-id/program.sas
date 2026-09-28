/* PROC COMPARE — ID, VAR/WITH, OUT= OUTBASE OUTCOMP OUTDIF OUTNOEQUAL.
   Cf. SAS 9.4 Procedures Guide, COMPARE Procedure : OUT= contient les
   variables ID, les variables VAR, _TYPE_ (BASE/COMP/DIF) et _OBS_ ;
   OUTBASE/OUTCOMP recopient les observations de BASE=/COMPARE= ;
   OUTDIF ecrit la difference (y - x) pour chaque paire appariée ;
   OUTNOEQUAL supprime les observations où tout est jugé égal. */
libname b 'data';

proc compare base=b.base compare=b.comp out=out
             outbase outcomp outdif outnoequal noprint;
  id key;
  var x;
  with y;
run;
