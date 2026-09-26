/* PROC TRANSPOSE — cf. PROC TRANSPOSE documentation. */
libname ind 'data';

proc transpose data=ind.wide out=trans;
run;
