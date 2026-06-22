libname d 'data';
title 'PROC REG: SIMPLE/CORR + printed matrices (XPX, I, COVB, CORRB)';

proc reg data=d.class simple corr;
  model weight = age height / xpx i covb corrb;
run;

title 'OUTEST= dataset (with COVOUT, OUTSEB, EDF)';
proc reg data=d.class outest=est covout outseb edf;
  model weight = age height;
run;

proc print data=est;
run;

title 'OUTSSCP= dataset';
proc reg data=d.class outsscp=sscp;
  model weight = age height;
run;

proc print data=sscp;
run;

title;
