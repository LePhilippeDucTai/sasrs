/* PROC PRINTTO — PRINT= route le listing, puis retour par defaut.
   Cf. SAS 9.4 Procedures Guide, PRINTTO Procedure : « PRINT= |
   file-specification routes procedure output to a permanent external
   file » ; le PROC PRINTTO nu retablit les destinations par defaut. Le
   DATA step suivant s'execute normalement et produit WORK.SHIFTED2. */
proc printto print='moved.lst';
run;

data shifted2;
  length tag $4;
  tag = 'ok';
  y = 7 * 6;
  output;
run;

proc printto;
run;
