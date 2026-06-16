/* M23.2 - ODS PDF : routage vers un fichier .pdf (PDF 1.4 pur Rust).
   Le listing texte est inactif pendant que PDF est la destination courante.
   Apres CLOSE, le listing texte reprend. */

libname d 'data';

data small;
    set d.class;
    if _n_ <= 3;
run;

ods pdf file='report.pdf';

title "PROC PRINT routed to PDF";
proc print data=small;
    var name sex age height;
run;

ods pdf close;

title "Text listing active again";
proc print data=small;
    var name age;
run;
title;
