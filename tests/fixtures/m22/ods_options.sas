/* M22.2 — options globales ODS (NOCENTER/NODATE/NONUMBER) acceptées sans
   warning. En v1 elles sont stockées sur la session ; le listing texte par
   défaut reste inchangé (application au rendu différée). */

libname d 'data';

data small;
    set d.class;
    if _n_ <= 3;
run;

options nocenter nodate nonumber;

title "PROC PRINT with global ODS options set";
proc print data=small;
    var name sex age;
run;
title;
