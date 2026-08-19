/* M42.3 — ODS OUTPUT généralisé : capture de la table "SQL_Results" d'un
   SELECT nu de PROC SQL (même chemin générique par nom d'objet ODS que
   OneWayFreqs pour PROC FREQ). */

libname d 'data';

/* Capture le SELECT suivant dans WORK.res. */
ods output sql_results=res;

title 'PROC SQL SELECT (listing + capture ODS OUTPUT SQL_Results)';
proc sql;
  select name, age from d.class where sex = 'F' order by name;
quit;

title 'Dataset captured by ODS OUTPUT (SQL_Results)';
proc print data=res noobs;
run;
