/* M38.4 — ODS SELECT/EXCLUDE : filtrage des objets ODS au point d'émission.
   Oracle : `ods exclude all;` supprime toute la sortie listing des steps
   suivants (aucune page, pas d'en-tête) SANS toucher au log ; la liste
   ALL/NONE persiste de step en step ; une liste NOMINATIVE est consommée à
   la fin du step de procédure suivant (retour à SELECT ALL) ; ODS OUTPUT
   capture même une table exclue de l'affichage. */

libname d 'data';

/* 1) EXCLUDE ALL : aucun listing pour les procs suivants, log inchangé
   (NOTEs de lecture et de timing normales). */
ods exclude all;

proc freq data=d.class;
    tables sex;
run;

/* ODS OUTPUT capture MÊME quand l'objet est exclu de l'affichage
   (la capture n'est pas un affichage). */
ods output OneWayFreqs=f;

proc freq data=d.class;
    tables sex;
run;

/* EXCLUDE ALL persiste au-delà des steps (idiome SAS ods exclude all /
   ods exclude none) : ce MEANS est supprimé aussi. */
proc means data=d.class;
    var height;
run;

/* 2) Retour au défaut : tout s'affiche de nouveau. */
ods exclude none;

title "Capture ODS OUTPUT realisee pendant EXCLUDE ALL";
proc print data=f;
run;

/* 3) Liste nominative : seule la section Moments du UNIVARIATE s'affiche
   (l'en-tete du proc et Variable: restent, les autres sections non). */
title "ODS SELECT Moments";
ods select Moments;

proc univariate data=d.class;
    var height;
run;

/* 4) La liste nominative a ete consommee par le step precedent (retour a
   SELECT ALL sans statement) : ce UNIVARIATE reaffiche toutes les sections. */
title "Apres consommation de la liste nominative (reset a SELECT ALL)";
proc univariate data=d.class;
    var height;
run;
title;
