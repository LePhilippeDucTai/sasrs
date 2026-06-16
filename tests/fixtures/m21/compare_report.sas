/* M21.1 PROC COMPARE + M21.4 PROC REPORT avancé (WHERE, GROUP, BREAK) */

libname d 'data';

/* PROC COMPARE : deux datasets avec une différence */
data base_ds;
    input id x y;
    datalines;
1 10 100
2 20 200
3 30 300
;
run;

data comp_ds;
    input id x y;
    datalines;
1 10 100
2 25 200
3 30 305
;
run;

title "COMPARE base vs comp (diff x ligne 2, y ligne 3)";
proc compare base=base_ds compare=comp_ds;
run;

/* PROC REPORT avec WHERE + GROUP + RBREAK (total général) summarize */
title "REPORT mean par sexe, WHERE age>=13, RBREAK total general";
proc report data=d.class nowd;
    column sex age height;
    define sex / group;
    define age / analysis mean;
    define height / analysis mean;
    where age >= 13;
    rbreak after / summarize;
run;
title;
