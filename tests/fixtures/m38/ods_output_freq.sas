/* M38.3 — ODS OUTPUT généralisé : capture de la table "OneWayFreqs" de
   PROC FREQ (chemin générique par nom d'objet ODS), puis nom de table
   inconnu → WARNING SAS "Output '...' was not created" en fin de proc. */

libname d 'data';

/* Capture la table OneWayFreqs du FREQ suivant dans WORK.f. */
ods output OneWayFreqs=f;

title "FREQ sex (listing + capture ODS OUTPUT OneWayFreqs)";
proc freq data=d.class;
    tables sex;
run;

/* Le dataset capturé : colonnes SAS Table, F_sex, sex, Frequency, Percent,
   CumFrequency, CumPercent — une observation par catégorie. */
title "Dataset captured by ODS OUTPUT (OneWayFreqs)";
proc print data=f;
run;
title;

/* Nom d'objet ODS que le proc ne produit pas : la demande reste sans objet
   et la fin du step émet le WARNING SAS (puis purge le registre). */
ods output NoSuchTable=g;

proc freq data=d.class;
    tables sex;
run;
