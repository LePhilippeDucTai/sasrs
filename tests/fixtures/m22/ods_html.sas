/* M22.4 — ODS HTML : routage de la sortie d'un PROC vers un fichier .html.
   Le LISTING texte est vide pendant que HTML est la destination courante
   (destinations exclusives en v1) ; le LOG porte la NOTE d'écriture du
   fichier. Le contenu HTML lui-même est vérifié par tests/ods_html.rs. */

libname d 'data';

/* Petit sous-ensemble (3 premières obs) pour un HTML compact. */
data small;
    set d.class;
    if _n_ <= 3;
run;

ods html file='report.html';

title "PROC PRINT routed to HTML";
proc print data=small;
    var name sex age height;
run;

ods html close;

/* Après fermeture HTML, le listing texte par défaut reprend. */
title "Text listing active again";
proc print data=small;
    var name age;
run;
title;
