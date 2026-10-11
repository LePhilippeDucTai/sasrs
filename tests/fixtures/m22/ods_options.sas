/* M22.2/J02-P9 — options globales ODS (CENTER/NOCENTER, DATE/NODATE,
   NUMBER/NONUMBER) acceptées et stockées sur la session. Le listing texte
   reste toujours centré, sans date ni numéro de page : NOCENTER, DATE et
   NUMBER émettent un WARNING « stored but not applied to the listing »
   (application du rendu : roadmap-avancee J11-P5) ; NODATE/NONUMBER, qui
   correspondent au listing, restent silencieux. */

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
