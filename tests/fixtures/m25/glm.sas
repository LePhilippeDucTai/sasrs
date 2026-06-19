/* tests/fixtures/m25/glm.sas */
libname d 'data';
title 'PROC GLM: height and weight by sex';

proc glm data=d.class;
  class sex;
  model height weight = sex / solution;
  lsmeans sex / se;
  estimate 'F vs M' sex 1 -1;
  contrast 'F vs M' sex 1 -1;
run;

title;
