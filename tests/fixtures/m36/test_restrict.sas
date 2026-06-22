libname d 'data';
title 'PROC REG: TEST statement (linear hypotheses)';

proc reg data=d.class;
  model weight = age height;
  test age = 0;
  equal: test age = height;
run;

title 'PROC REG: RESTRICT statement (constrained LS)';

proc reg data=d.class;
  model weight = age height;
  restrict age = height;
run;

title;
