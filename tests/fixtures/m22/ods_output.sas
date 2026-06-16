/* M22.3 — ODS OUTPUT : capture de la table "Summary" de PROC MEANS vers un
   dataset, puis impression du dataset capturé. */

libname d 'data';

/* Capture la table Summary de MEANS dans WORK.means_out. */
ods output Summary=means_out;

title "MEANS height/weight (sortie listing + capture ODS OUTPUT)";
proc means data=d.class;
    var height weight;
run;

/* Désactive la capture pour la suite. */
ods output close;

/* Le dataset capture : 1 obs par variable, colonnes Variable + stats. */
title "Dataset captured by ODS OUTPUT (means_out)";
proc print data=means_out;
run;
title;
