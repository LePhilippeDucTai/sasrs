/* M23.3 - ODS EXCEL : routage vers un fichier .xlsx (XLSX pur Rust).
   Le listing texte est inactif pendant que Excel est la destination courante.
   Apres CLOSE, le listing texte reprend. */

libname d 'data';

data small;
    set d.class;
    if _n_ <= 3;
run;

ods excel file='report.xlsx';

title "PROC PRINT routed to Excel";
proc print data=small;
    var name sex age;
run;

ods excel close;

title "Text listing active again";
proc print data=small;
    var name age;
run;
title;
