libname d 'data';
title 'PROC REG: RSQUARE all-subsets selection';

proc reg data=d.class;
  model weight = age height / selection=rsquare;
run;

title 'PROC REG: CP (Mallows) selection';

proc reg data=d.class;
  model weight = age height / selection=cp;
run;

title 'PROC REG: ADJRSQ selection';

proc reg data=d.class;
  model weight = age height / selection=adjrsq;
run;

title;
