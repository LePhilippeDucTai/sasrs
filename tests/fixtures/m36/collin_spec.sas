libname d 'data';
title 'PROC REG: collinearity (VIF/TOL/COLLIN) and specification (SPEC/DW/ACOV)';

proc reg data=d.class;
  model weight = age height / vif tol collin spec dw dwprob acov;
run;

title;
