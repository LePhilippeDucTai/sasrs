/* M23.1 - ODS RTF : routage vers un fichier .rtf.
   Le listing texte est inactif pendant que RTF est la destination courante
   (destinations exclusives en v1). Le LOG porte la NOTE d'ecriture du fichier.
   Apres CLOSE, le listing texte reprend. */

libname d 'data';

data small;
    set d.class;
    if _n_ <= 3;
run;

ods rtf file='report.rtf';

title "PROC PRINT routed to RTF";
proc print data=small;
    var name sex age;
run;

ods rtf close;

title "Text listing active again";
proc print data=small;
    var name age;
run;
title;
