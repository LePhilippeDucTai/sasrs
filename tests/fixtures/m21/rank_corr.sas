/* M21.5 — RANK méthodes + BY ; CORR Spearman/Kendall */

libname d 'data';

/* RANK : méthode FRACTION sur height */
proc rank data=d.class out=ranked fraction;
    var height;
    ranks height_frac;
run;

title "RANK FRACTION sur height";
proc print data=ranked;
    var name height height_frac;
run;

/* RANK avec BY : rangs indépendants par sexe */
proc sort data=d.class out=byclass;
    by sex;
run;

proc rank data=byclass out=ranked_by;
    by sex;
    var age;
    ranks age_rank;
run;

title "RANK age par groupe sex (BY)";
proc print data=ranked_by;
    var name sex age age_rank;
run;

/* CORR Spearman + Kendall sur height/weight */
title "CORR Spearman + Kendall (height, weight)";
proc corr data=d.class spearman kendall;
    var height weight;
run;
title;
