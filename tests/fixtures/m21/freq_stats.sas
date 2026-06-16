/* M21.2 — FREQ : statistiques avancées (Fisher, MEASURES, AGREE, CHISQ 1 voie).
   Données en observations individuelles (PROC FREQ ne pondère pas via WEIGHT
   dans cette version — voir PROGRESS.md ; on développe donc les comptes en
   lignes pour obtenir les tables 2x2 voulues, hand-vérifiables). */

libname d 'data';

/* CHISQ une voie (ajustement à l'équiprobabilité) sur sex de sashelp.class */
title "CHISQ une voie - sex (F=9, M=10)";
proc freq data=d.class;
    tables sex / chisq;
run;

/* Table 2x2 = [[3,1],[1,3]] pour Fisher exact + MEASURES.
   Fisher bilateral attendu ~0.4857 ; Odds Ratio = (3*3)/(1*1) = 9. */
data fishtab;
    input grp $ outcome $;
    datalines;
A yes
A yes
A yes
A no
B yes
B no
B no
B no
;
run;

title "Fisher exact + MEASURES sur 2x2 [[3,1],[1,3]]";
proc freq data=fishtab;
    tables grp*outcome / fisher measures;
run;

/* Table carree 2x2 = [[4,1],[1,4]] pour AGREE (kappa).
   Po=(4+4)/10=0.8, Pe=0.5 -> kappa=(0.8-0.5)/(1-0.5)=0.6. */
data ratings;
    input r1 $ r2 $;
    datalines;
P P
P P
P P
P P
P N
N P
N N
N N
N N
N N
;
run;

title "AGREE (kappa) sur 2x2 [[4,1],[1,4]] -> kappa=0.6";
proc freq data=ratings;
    tables r1*r2 / agree;
run;
title;
