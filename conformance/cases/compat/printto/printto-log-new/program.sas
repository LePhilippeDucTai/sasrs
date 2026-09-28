/* PROC PRINTTO — LOG= et NEW, puis retour aux destinations par defaut.
   Cf. SAS 9.4 Procedures Guide, PRINTTO Procedure : « LOG=LOG |
   file-specification routes the SAS log to a permanent external file » ;
   NEW « replaces the existing contents of a file » ; le PROC PRINTTO nu
   retablit les destinations par defaut. Le DATA step suivant s'execute
   normalement et produit WORK.SHIFTED. */
proc printto log='moved.log' new;
run;

data shifted;
  x = 41 + 1;
  output;
run;

proc printto;
run;
