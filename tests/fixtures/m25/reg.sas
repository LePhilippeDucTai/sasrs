libname d 'data';
title 'PROC REG: weight regressed on height';

proc reg data=d.class;
  model weight = height;
run;

title;
