/* M14.4 — LIBNAME ... CSV (bibliothèque virtuelle) + PROC PRINT */
libname petlib csv 'data';

proc print data=petlib.pets;
run;
