/* PROC FREQ TABLES OUT= — cf. PROC FREQ documentation. */
libname ind 'data';

proc freq data=ind.survey;
  tables answer / out=counts;
run;
