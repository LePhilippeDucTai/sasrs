/* M7 : PROC APPEND (empiler B sur A) puis PROC DATASETS (change + delete). */
data work.a; x = 1; output; x = 2; output; run;
data work.b; x = 3; output; run;

proc append base=work.a data=work.b;
run;

proc datasets lib=work nolist;
  change a=combined;
  delete b;
quit;

title 'Appended then renamed to COMBINED';
proc print data=work.combined;
run;
