libname d 'data';
title 'PROC REG: ridge regression (RIDGE=) with OUTVIF';

proc reg data=d.class ridge=0 0.05 0.1 outvif outest=rest;
  model weight = age height;
run;

proc print data=rest;
run;

title 'PROC REG: incomplete principal components (PCOMIT=)';

proc reg data=d.class pcomit=1 outest=pest;
  model weight = age height;
run;

proc print data=pest;
run;

title;
