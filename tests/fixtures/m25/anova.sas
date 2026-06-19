/* tests/fixtures/m25/anova.sas */
libname d 'data';
title 'PROC ANOVA: height and weight by sex';

proc anova data=d.class;
  class sex;
  model height weight = sex;
  means sex;
run;

title;
