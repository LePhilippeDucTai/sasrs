libname d 'data';
title 'PROC REG: MTEST multivariate tests (two responses)';

proc reg data=d.class;
  model weight height = age;
  mtest age;
run;

title 'PROC REG: VAR / ADD / DELETE / REWEIGHT (run-group)';

proc reg data=d.class;
  var age height;
  model weight = age;
  add height;
  reweight age < 12;
run;

title;
