/* PROC MEANS — option NWAY + CLASS / MISSING.
   Cf. SAS 9.4 Procedures Guide, MEANS Procedure : sans MISSING, les
   observations dont la valeur de CLASS est manquante sont exclues des
   groupes ; avec MISSING elles forment un niveau propre ; NWAY limite
   l'OUT= au _TYPE_ le plus elevé (combinaison de toutes les CLASS). */
libname ind 'data';

proc means data=ind.sales noprint;
  class region;
  var amount;
  output out=excl mean=m n=n;
run;

proc means data=ind.sales nway noprint missing;
  class region;
  var amount;
  output out=kept mean=m n=n;
run;
