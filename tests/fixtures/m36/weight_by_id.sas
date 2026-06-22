libname d 'data';
title 'PROC REG: WEIGHT (weighted least squares) with ID in residual table';

proc reg data=d.class;
  weight age;
  id name;
  model weight = height / r;
run;

title 'PROC REG: BY-group analysis (by sex)';

proc sort data=d.class out=cls;
  by sex;
run;

proc reg data=cls;
  by sex;
  model weight = height;
run;

title;
